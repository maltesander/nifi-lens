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

#[derive(Default)]
pub struct WorkerRegistry {
    current: Option<(ViewId, JoinHandle<()>)>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ensure the worker for `view` is running, aborting any previously
    /// active worker. No-ops when `view` already matches the currently-
    /// running worker.
    ///
    /// `cluster` is the central `ClusterStore`. The registry calls
    /// `subscribe` on entry into a view and `unsubscribe` on exit so
    /// `ClusterStore` can gate expensive endpoints to only the views
    /// that need them (Task 10).
    pub fn ensure(
        &mut self,
        view: ViewId,
        client: &Arc<RwLock<NifiClient>>,
        tx: &mpsc::Sender<AppEvent>,
        browser: &mut crate::view::browser::state::BrowserState,
        cluster: &mut crate::cluster::ClusterStore,
        _polling: &crate::config::PollingConfig,
    ) {
        if matches!(&self.current, Some((existing, _)) if *existing == view) {
            return;
        }
        if let Some((existing_view, handle)) = self.current.take() {
            tracing::debug!(
                from = ?existing_view,
                to = ?view,
                "worker registry: swapping view worker"
            );
            handle.abort();
            // Unsubscribe the outgoing view from its cluster endpoints
            // and drop any per-view channels held on AppState.
            match existing_view {
                ViewId::Overview => {
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::RootPgStatus,
                        ViewId::Overview,
                    );
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::ControllerServices,
                        ViewId::Overview,
                    );
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::ControllerStatus,
                        ViewId::Overview,
                    );
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::SystemDiagnostics,
                        ViewId::Overview,
                    );
                    cluster.unsubscribe(crate::cluster::ClusterEndpoint::About, ViewId::Overview);
                    cluster
                        .unsubscribe(crate::cluster::ClusterEndpoint::Bulletins, ViewId::Overview);
                }
                ViewId::Bulletins => {
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::Bulletins,
                        ViewId::Bulletins,
                    );
                }
                ViewId::Browser => {
                    browser.detail_tx = None;
                    // The three cluster endpoints that feed the Browser
                    // arena are all released here — while Browser is
                    // inactive the store can gate (Task 10) the
                    // connections fan-out so inactive tabs don't drive
                    // per-PG requests.
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::RootPgStatus,
                        ViewId::Browser,
                    );
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::ControllerServices,
                        ViewId::Browser,
                    );
                    cluster.unsubscribe(
                        crate::cluster::ClusterEndpoint::ConnectionsByPg,
                        ViewId::Browser,
                    );
                }
                _ => {}
            }
        }
        let handle = match view {
            ViewId::Overview => {
                tracing::debug!(?view, "worker registry: overview is store-only");
                // Task 8: Overview has no per-view worker. It subscribes
                // to six cluster-wide endpoints and projects each into
                // `OverviewState` via the `redraw_*` reducers wired in
                // the `ClusterChanged` arm.
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::RootPgStatus,
                    ViewId::Overview,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::ControllerServices,
                    ViewId::Overview,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::ControllerStatus,
                    ViewId::Overview,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::SystemDiagnostics,
                    ViewId::Overview,
                );
                cluster.subscribe(crate::cluster::ClusterEndpoint::About, ViewId::Overview);
                // Overview's sparkline + noisy-components panel reads
                // from the shared bulletins ring.
                cluster.subscribe(crate::cluster::ClusterEndpoint::Bulletins, ViewId::Overview);
                None
            }
            ViewId::Bulletins => {
                tracing::debug!(
                    ?view,
                    "worker registry: subscribing bulletins to shared ring"
                );
                // Task 7: Bulletins no longer has a per-view worker —
                // the cluster store polls the shared `BulletinRing` and
                // `redraw_bulletins` mirrors it into `BulletinsState`.
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::Bulletins,
                    ViewId::Bulletins,
                );
                None
            }
            ViewId::Browser => {
                tracing::debug!(?view, "worker registry: spawning browser worker");
                // Browser's arena is rebuilt from the cluster snapshot
                // (Task 6). Subscribe to every endpoint it reads so the
                // store knows Browser is active — RootPgStatus gives the
                // PG/processor/connection/port skeleton, ControllerServices
                // attaches CS rows, ConnectionsByPg backfills endpoint ids.
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::RootPgStatus,
                    ViewId::Browser,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::ControllerServices,
                    ViewId::Browser,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::ConnectionsByPg,
                    ViewId::Browser,
                );
                let (detail_tx, detail_rx) = mpsc::unbounded_channel();
                browser.detail_tx = Some(detail_tx);
                Some(crate::view::browser::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    detail_rx,
                ))
            }
            ViewId::Events => {
                tracing::debug!(?view, "worker registry: no worker for this view");
                None
            }
            ViewId::Tracer => {
                tracing::debug!(?view, "worker registry: no worker for this view");
                None
            }
        };
        if let Some(handle) = handle {
            self.current = Some((view, handle));
        }
    }

    /// Abort the current worker so the next `ensure()` call spawns a
    /// fresh one. Used after context switch — the view tab hasn't changed
    /// but the backing client has.
    pub fn invalidate(&mut self) {
        if let Some((view, handle)) = self.current.take() {
            tracing::debug!(?view, "worker registry: invalidating for context switch");
            handle.abort();
        }
    }

    /// Abort the currently-running view worker, if any. Called on app
    /// shutdown. The registry only ever holds one active worker, so this
    /// aborts exactly one handle or none.
    pub fn shutdown(&mut self) {
        tracing::debug!("worker registry: shutting down");
        if let Some((_, handle)) = self.current.take() {
            handle.abort();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::event::EventsPayload;

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
}
