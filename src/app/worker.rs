//! WorkerRegistry: owns the one currently-running view worker task and
//! swaps it on tab change.
//!
//! Phase 1 shipped the Overview worker (10s cadence) and Phase 2 added
//! the Bulletins worker (5s cadence). Phase 3's Browser worker and
//! Phase 4's Tracer worker plug into the same pattern. Task 7 retired
//! the Bulletins worker — Bulletins now subscribes to the cluster-owned
//! `BulletinRing` and has no per-view task. Task 8 retired the Overview
//! worker the same way: Overview subscribes to six cluster endpoints
//! and projects them into `OverviewState` via the `redraw_*` reducers.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::app::state::ViewId;
use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::event::{AppEvent, ViewPayload};

/// Ship a poll result to the UI task: success becomes `AppEvent::Data`,
/// error is logged at `warn!` and forwarded as `AppEvent::IntentOutcome`.
/// `label` identifies the worker in log output.
pub(crate) async fn send_poll_result(
    tx: &mpsc::Sender<AppEvent>,
    label: &'static str,
    result: Result<ViewPayload, NifiLensError>,
) {
    match result {
        Ok(payload) => {
            if tx.send(AppEvent::Data(payload)).await.is_err() {
                tracing::debug!(worker = label, "channel closed during data send");
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, worker = label, "poll failed");
            if tx.send(AppEvent::IntentOutcome(Err(err))).await.is_err() {
                tracing::debug!(worker = label, "channel closed during error send");
            }
        }
    }
}

/// Tracks which view is currently "active" (i.e. the owner of the
/// subscription set and any spawned worker handle).
///
/// The registry must decouple "active view" from "handle ownership":
/// most views have no worker handle (Overview / Bulletins / Events /
/// Tracer), yet every view participates in subscribe/unsubscribe
/// bookkeeping. Tying the transition logic to handle presence would
/// leak subscriptions across handle-less tab swaps.
#[derive(Default)]
pub struct WorkerRegistry {
    /// The view that currently "owns" the subscribe state.
    active: Option<ViewId>,
    /// The spawned worker handle, if any. Today only Browser spawns one.
    handle: Option<JoinHandle<()>>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure `view` is the active view. On transition, the outgoing
    /// view's subscriptions are released and any spawned handle is
    /// aborted; then the incoming view's subscriptions are taken and
    /// (for Browser) a worker is spawned.
    ///
    /// `cluster` is the central `ClusterStore`. The registry calls
    /// `subscribe` on entry into a view and `unsubscribe` on exit so
    /// `ClusterStore` can gate expensive endpoints to only the views
    /// that need them.
    pub fn ensure(
        &mut self,
        view: ViewId,
        client: &Arc<RwLock<NifiClient>>,
        tx: &mpsc::Sender<AppEvent>,
        browser: &mut crate::view::browser::state::BrowserState,
        cluster: &mut crate::cluster::ClusterStore,
    ) {
        if self.active == Some(view) {
            return;
        }

        // Outgoing view: release subscriptions and drop per-view state
        // BEFORE touching the incoming side. This runs unconditionally
        // on every transition — regardless of whether the outgoing view
        // owned a worker handle.
        if let Some(existing_view) = self.active.take() {
            tracing::debug!(
                from = ?existing_view,
                to = ?view,
                "worker registry: swapping view"
            );
            if let Some(handle) = self.handle.take() {
                handle.abort();
            }
            Self::transition_out(existing_view, cluster, browser);
        }

        // Incoming view: subscribe, then spawn the worker handle (if any).
        Self::transition_in_subscribe(view, cluster);
        self.handle = match view {
            ViewId::Browser => {
                tracing::debug!(?view, "worker registry: spawning browser worker");
                let (detail_tx, detail_rx) = mpsc::unbounded_channel();
                browser.detail_tx = Some(detail_tx);
                Some(crate::view::browser::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    detail_rx,
                ))
            }
            ViewId::Overview => {
                tracing::debug!(?view, "worker registry: overview is store-only");
                None
            }
            ViewId::Bulletins => {
                tracing::debug!(
                    ?view,
                    "worker registry: subscribing bulletins to shared ring"
                );
                None
            }
            ViewId::Events | ViewId::Tracer => {
                tracing::debug!(?view, "worker registry: no worker for this view");
                None
            }
        };
        self.active = Some(view);
    }

    /// Unsubscribe the outgoing view from every endpoint it holds and
    /// drop any per-view channels on `AppState`. Pure state-machine
    /// logic; no I/O, no client access — exercised directly by the
    /// transition tests.
    fn transition_out(
        existing_view: ViewId,
        cluster: &mut crate::cluster::ClusterStore,
        browser: &mut crate::view::browser::state::BrowserState,
    ) {
        use crate::cluster::ClusterEndpoint;
        match existing_view {
            ViewId::Overview => {
                cluster.unsubscribe(ClusterEndpoint::RootPgStatus, ViewId::Overview);
                cluster.unsubscribe(ClusterEndpoint::ControllerServices, ViewId::Overview);
                cluster.unsubscribe(ClusterEndpoint::ControllerStatus, ViewId::Overview);
                cluster.unsubscribe(ClusterEndpoint::SystemDiagnostics, ViewId::Overview);
                cluster.unsubscribe(ClusterEndpoint::About, ViewId::Overview);
                cluster.unsubscribe(ClusterEndpoint::Bulletins, ViewId::Overview);
            }
            ViewId::Bulletins => {
                cluster.unsubscribe(ClusterEndpoint::Bulletins, ViewId::Bulletins);
            }
            ViewId::Browser => {
                browser.detail_tx = None;
                // The three cluster endpoints that feed the Browser
                // arena are all released here — while Browser is
                // inactive the store gates the connections fan-out so
                // inactive tabs don't drive per-PG requests.
                cluster.unsubscribe(ClusterEndpoint::RootPgStatus, ViewId::Browser);
                cluster.unsubscribe(ClusterEndpoint::ControllerServices, ViewId::Browser);
                cluster.unsubscribe(ClusterEndpoint::ConnectionsByPg, ViewId::Browser);
            }
            ViewId::Events | ViewId::Tracer => {}
        }
    }

    /// Subscribe the incoming view to every endpoint it reads. Pure
    /// state-machine logic; no I/O, no client access — exercised
    /// directly by the transition tests.
    fn transition_in_subscribe(view: ViewId, cluster: &mut crate::cluster::ClusterStore) {
        use crate::cluster::ClusterEndpoint;
        match view {
            ViewId::Overview => {
                // Overview has no per-view worker; it subscribes to
                // six cluster-wide endpoints and projects each into
                // `OverviewState` via the `redraw_*` reducers.
                cluster.subscribe(ClusterEndpoint::RootPgStatus, ViewId::Overview);
                cluster.subscribe(ClusterEndpoint::ControllerServices, ViewId::Overview);
                cluster.subscribe(ClusterEndpoint::ControllerStatus, ViewId::Overview);
                cluster.subscribe(ClusterEndpoint::SystemDiagnostics, ViewId::Overview);
                cluster.subscribe(ClusterEndpoint::About, ViewId::Overview);
                // Overview's sparkline + noisy-components panel reads
                // from the shared bulletins ring.
                cluster.subscribe(ClusterEndpoint::Bulletins, ViewId::Overview);
            }
            ViewId::Bulletins => {
                // Bulletins mirrors the shared `BulletinRing` into its
                // own view state.
                cluster.subscribe(ClusterEndpoint::Bulletins, ViewId::Bulletins);
            }
            ViewId::Browser => {
                // Browser's arena is rebuilt from the cluster snapshot.
                // Subscribe to every endpoint it reads — RootPgStatus
                // provides the PG/processor/connection/port skeleton,
                // ControllerServices attaches CS rows, ConnectionsByPg
                // backfills endpoint ids.
                cluster.subscribe(ClusterEndpoint::RootPgStatus, ViewId::Browser);
                cluster.subscribe(ClusterEndpoint::ControllerServices, ViewId::Browser);
                cluster.subscribe(ClusterEndpoint::ConnectionsByPg, ViewId::Browser);
            }
            ViewId::Events | ViewId::Tracer => {}
        }
    }

    /// Abort the current worker so the next `ensure()` call spawns a
    /// fresh one. Used after context switch — the view tab hasn't changed
    /// but the backing client has. Subscribers are managed by the caller
    /// via `state.cluster` (see `pending_worker_restart` handling).
    pub fn invalidate(&mut self) {
        if let Some(handle) = self.handle.take() {
            tracing::debug!(
                active = ?self.active,
                "worker registry: invalidating for context switch"
            );
            handle.abort();
        }
        self.active = None;
    }

    /// Abort the currently-running view worker, if any. Called on app
    /// shutdown. The registry only ever holds one active worker, so this
    /// aborts exactly one handle or none.
    pub fn shutdown(&mut self) {
        tracing::debug!("worker registry: shutting down");
        if let Some(handle) = self.handle.take() {
            handle.abort();
        }
        self.active = None;
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::cluster::{ClusterEndpoint, ClusterStore, config::ClusterPollingConfig};
    use crate::event::EventsPayload;
    use crate::view::browser::state::BrowserState;

    #[tokio::test]
    async fn send_poll_result_ok_yields_data_event() {
        let (tx, mut rx) = mpsc::channel(4);
        // Any ViewPayload works for this smoke test; pick Events to
        // avoid depending on the Bulletins ring fixture.
        let payload = ViewPayload::Events(EventsPayload::QueryFailed {
            query_id: None,
            error: "smoke".into(),
        });
        send_poll_result(&tx, "test", Ok(payload)).await;
        let ev = rx.recv().await.expect("event");
        assert!(
            matches!(ev, AppEvent::Data(ViewPayload::Events(_))),
            "expected Data(Events)"
        );
    }

    #[tokio::test]
    async fn send_poll_result_err_yields_intent_outcome() {
        let (tx, mut rx) = mpsc::channel(4);
        let err = NifiLensError::ConfigMissing {
            path: PathBuf::from("/nonexistent"),
        };
        send_poll_result(&tx, "test", Err(err)).await;
        let ev = rx.recv().await.expect("event");
        assert!(
            matches!(ev, AppEvent::IntentOutcome(Err(_))),
            "expected IntentOutcome(Err)"
        );
    }

    /// Simulates the subscription-state-machine half of `ensure`
    /// without constructing a real `NifiClient` / `mpsc::Sender`.
    /// Mirrors the transition logic in `ensure`: early-return when
    /// the view is unchanged, else run `transition_out` + clear any
    /// handle slot + run `transition_in_subscribe` + update `active`.
    ///
    /// Browser normally spawns a real worker handle; the test helper
    /// cannot, so the `handle` slot stays `None` for Browser entries.
    /// The subscribe/unsubscribe bookkeeping is identical to production.
    fn ensure_subscriptions_only(
        registry: &mut WorkerRegistry,
        view: ViewId,
        browser: &mut BrowserState,
        cluster: &mut ClusterStore,
    ) {
        if registry.active == Some(view) {
            return;
        }
        if let Some(existing_view) = registry.active.take() {
            if let Some(handle) = registry.handle.take() {
                handle.abort();
            }
            WorkerRegistry::transition_out(existing_view, cluster, browser);
        }
        WorkerRegistry::transition_in_subscribe(view, cluster);
        registry.active = Some(view);
    }

    /// Count how many of `endpoints` have `view` registered as a
    /// subscriber. Introspects the canonical per-endpoint sets via
    /// `debug_snapshot`, which is the only view-aware accessor the
    /// registry exposes.
    fn count_view_subscriptions(
        store: &ClusterStore,
        view: ViewId,
        endpoints: &[ClusterEndpoint],
    ) -> usize {
        let snap = store.subscribers.debug_snapshot();
        endpoints
            .iter()
            .filter(|ep| {
                snap.iter()
                    .any(|(snap_ep, subs)| snap_ep == *ep && subs.iter().any(|s| s.0 == view))
            })
            .count()
    }

    fn overview_subscriber_total(store: &ClusterStore) -> usize {
        count_view_subscriptions(
            store,
            ViewId::Overview,
            &[
                ClusterEndpoint::RootPgStatus,
                ClusterEndpoint::ControllerServices,
                ClusterEndpoint::ControllerStatus,
                ClusterEndpoint::SystemDiagnostics,
                ClusterEndpoint::About,
                ClusterEndpoint::Bulletins,
            ],
        )
    }

    fn browser_subscriber_total(store: &ClusterStore) -> usize {
        count_view_subscriptions(
            store,
            ViewId::Browser,
            &[
                ClusterEndpoint::RootPgStatus,
                ClusterEndpoint::ControllerServices,
                ClusterEndpoint::ConnectionsByPg,
            ],
        )
    }

    #[test]
    fn ensure_transitions_release_and_acquire_subscribers() {
        // Regression for the pre-fix subscription leak: transitions
        // between handle-less views (Overview → Events, Events → Tracer,
        // Tracer → Bulletins, …) must still fire the outgoing view's
        // unsubscribe path.
        let mut store = ClusterStore::new(ClusterPollingConfig::default(), 100);
        let mut browser = BrowserState::default();
        let mut registry = WorkerRegistry::new();

        // Enter Overview: all six Overview endpoints are subscribed.
        ensure_subscriptions_only(&mut registry, ViewId::Overview, &mut browser, &mut store);
        assert_eq!(
            overview_subscriber_total(&store),
            6,
            "Overview entry should add six subscribers"
        );
        assert_eq!(browser_subscriber_total(&store), 0);
        assert_eq!(registry.active, Some(ViewId::Overview));

        // Overview → Events (handle-less transition). Pre-fix, this
        // was the broken path: Overview's six subscribers leaked for
        // the rest of the session.
        ensure_subscriptions_only(&mut registry, ViewId::Events, &mut browser, &mut store);
        assert_eq!(
            overview_subscriber_total(&store),
            0,
            "Overview → Events must drop all Overview subscribers"
        );
        assert_eq!(registry.active, Some(ViewId::Events));

        // Events → Browser: Browser subscribes to its three endpoints.
        ensure_subscriptions_only(&mut registry, ViewId::Browser, &mut browser, &mut store);
        assert_eq!(
            browser_subscriber_total(&store),
            3,
            "Browser entry should add three subscribers"
        );
        assert_eq!(overview_subscriber_total(&store), 0);

        // Browser → Tracer (handle-less transition out of a handle-
        // owning view). Browser's three subscribers must be released.
        ensure_subscriptions_only(&mut registry, ViewId::Tracer, &mut browser, &mut store);
        assert_eq!(
            browser_subscriber_total(&store),
            0,
            "Browser → Tracer must drop all Browser subscribers"
        );
        assert_eq!(registry.active, Some(ViewId::Tracer));

        // Tracer → Bulletins (handle-less → handle-less). Bulletins
        // alone subscribes to the Bulletins endpoint.
        ensure_subscriptions_only(&mut registry, ViewId::Bulletins, &mut browser, &mut store);
        assert_eq!(store.subscribers.count(ClusterEndpoint::Bulletins), 1);

        // Bulletins → Overview: second Overview entry must re-subscribe
        // all six endpoints. Bulletins's single subscriber is released
        // before Overview re-adds its own Bulletins subscriber, so the
        // final Bulletins count is 1 (from Overview, not the original).
        ensure_subscriptions_only(&mut registry, ViewId::Overview, &mut browser, &mut store);
        assert_eq!(
            overview_subscriber_total(&store),
            6,
            "Second Overview entry should re-subscribe all six endpoints"
        );
        assert_eq!(store.subscribers.count(ClusterEndpoint::Bulletins), 1);
    }

    #[test]
    fn ensure_same_view_is_noop() {
        // Calling ensure with the already-active view must not
        // double-count subscribers nor touch the registry.
        let mut store = ClusterStore::new(ClusterPollingConfig::default(), 100);
        let mut browser = BrowserState::default();
        let mut registry = WorkerRegistry::new();

        ensure_subscriptions_only(&mut registry, ViewId::Overview, &mut browser, &mut store);
        assert_eq!(overview_subscriber_total(&store), 6);

        ensure_subscriptions_only(&mut registry, ViewId::Overview, &mut browser, &mut store);
        assert_eq!(
            overview_subscriber_total(&store),
            6,
            "Re-entering the same view must not double-subscribe"
        );
    }

    #[test]
    fn invalidate_clears_active_view() {
        // After `invalidate()` the registry has no active view, so the
        // next `ensure` treats the transition as a fresh entry. This
        // keeps context-switch semantics correct: the caller rebuilds
        // the cluster store, and the next `ensure` subscribes to the
        // fresh store without tripping the "same view, no-op" branch.
        let mut store = ClusterStore::new(ClusterPollingConfig::default(), 100);
        let mut browser = BrowserState::default();
        let mut registry = WorkerRegistry::new();

        ensure_subscriptions_only(&mut registry, ViewId::Overview, &mut browser, &mut store);
        assert_eq!(registry.active, Some(ViewId::Overview));

        registry.invalidate();
        assert_eq!(registry.active, None);
        assert!(registry.handle.is_none());

        // A fresh store (as produced by `spawn_fetchers` after a
        // context switch) starts with zero subscribers; the next
        // `ensure` repopulates them.
        let mut fresh_store = ClusterStore::new(ClusterPollingConfig::default(), 100);
        ensure_subscriptions_only(
            &mut registry,
            ViewId::Overview,
            &mut browser,
            &mut fresh_store,
        );
        assert_eq!(overview_subscriber_total(&fresh_store), 6);
    }
}
