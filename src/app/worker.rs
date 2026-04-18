//! WorkerRegistry: owns the one currently-running view worker task and
//! swaps it on tab change.
//!
//! Phase 1 shipped the Overview worker (10s cadence) and Phase 2 added
//! the Bulletins worker (5s cadence). Browser (Phase 3) and Tracer
//! (Phase 4) will plug into the same pattern.

use std::future::Future;
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tracing;

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

/// Spawn a polling worker on the current `LocalSet` that calls `poll_fn`
/// every `interval`, shipping results via [`send_poll_result`]. `label`
/// is used for log output. The closure is `FnMut` so callers like
/// Bulletins can capture mutable cursor state.
pub(crate) fn spawn_polling_worker<F, Fut>(
    interval: Duration,
    label: &'static str,
    mut poll_fn: F,
    tx: mpsc::Sender<AppEvent>,
) -> JoinHandle<()>
where
    F: FnMut() -> Fut + 'static,
    Fut: Future<Output = Result<ViewPayload, NifiLensError>>,
{
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(interval);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            send_poll_result(&tx, label, poll_fn().await).await;
            if tx.is_closed() {
                tracing::debug!(worker = label, "channel closed, exiting");
                return;
            }
        }
    })
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
    #[allow(clippy::too_many_arguments)]
    pub fn ensure(
        &mut self,
        view: ViewId,
        client: &Arc<RwLock<NifiClient>>,
        tx: &mpsc::Sender<AppEvent>,
        bulletins_last_id: Option<i64>,
        browser: &mut crate::view::browser::state::BrowserState,
        cluster: &mut crate::cluster::ClusterStore,
        polling: &crate::config::PollingConfig,
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
                }
                ViewId::Browser => {
                    browser.detail_tx = None;
                    browser.force_tick_tx = None;
                }
                _ => {}
            }
        }
        let handle = match view {
            ViewId::Overview => {
                tracing::debug!(?view, "worker registry: spawning overview worker");
                // Subscribe for cluster-wide endpoints Overview consumes.
                // Additional endpoints (ControllerStatus, SystemDiagnostics,
                // About) move into the cluster store in later tasks and
                // will be subscribed here then.
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::RootPgStatus,
                    ViewId::Overview,
                );
                cluster.subscribe(
                    crate::cluster::ClusterEndpoint::ControllerServices,
                    ViewId::Overview,
                );
                Some(crate::view::overview::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    polling.overview.pg_status,
                    polling.overview.sysdiag,
                ))
            }
            ViewId::Bulletins => {
                tracing::debug!(?view, "worker registry: spawning bulletins worker");
                Some(crate::view::bulletins::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    bulletins_last_id,
                    polling.bulletins.interval,
                ))
            }
            ViewId::Browser => {
                tracing::debug!(?view, "worker registry: spawning browser worker");
                let (detail_tx, detail_rx) = mpsc::unbounded_channel();
                let (force_tx, force_rx) = tokio::sync::oneshot::channel();
                browser.detail_tx = Some(detail_tx);
                browser.force_tick_tx = Some(force_tx);
                Some(crate::view::browser::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    detail_rx,
                    force_rx,
                    polling.browser.interval,
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
    use crate::event::BulletinsPayload;

    #[tokio::test]
    async fn send_poll_result_ok_yields_data_event() {
        let (tx, mut rx) = mpsc::channel(4);
        let payload = ViewPayload::Bulletins(BulletinsPayload {
            bulletins: Vec::new(),
            fetched_at: std::time::SystemTime::now(),
        });
        send_poll_result(&tx, "test", Ok(payload)).await;
        let ev = rx.recv().await.expect("event");
        assert!(
            matches!(ev, AppEvent::Data(ViewPayload::Bulletins(_))),
            "expected Data(Bulletins)"
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
