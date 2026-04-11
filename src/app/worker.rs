//! WorkerRegistry: owns the one currently-running view worker task and
//! swaps it on tab change.
//!
//! Phase 1 only spawns a worker for the Overview tab; other tabs get no
//! worker (Phases 2–4 add theirs).

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tracing;

use crate::app::state::ViewId;
use crate::client::NifiClient;
use crate::event::AppEvent;

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
    pub fn ensure(
        &mut self,
        view: ViewId,
        client: &Arc<RwLock<NifiClient>>,
        tx: &mpsc::Sender<AppEvent>,
        bulletins_last_id: Option<i64>,
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
        }
        let handle = match view {
            ViewId::Overview => {
                tracing::debug!(?view, "worker registry: spawning overview worker");
                Some(crate::view::overview::worker::spawn(
                    client.clone(),
                    tx.clone(),
                ))
            }
            ViewId::Bulletins => {
                tracing::debug!(?view, "worker registry: spawning bulletins worker");
                Some(crate::view::bulletins::worker::spawn(
                    client.clone(),
                    tx.clone(),
                    bulletins_last_id,
                ))
            }
            ViewId::Browser | ViewId::Tracer => {
                tracing::debug!(?view, "worker registry: no worker for this view");
                None
            }
        };
        if let Some(handle) = handle {
            self.current = Some((view, handle));
        }
    }

    /// Abort the currently-running view worker, if any. Called on app
    /// shutdown. Phase 1 only ever has one active worker, so this aborts
    /// exactly one handle or none.
    pub fn shutdown(&mut self) {
        tracing::debug!("worker registry: shutting down");
        if let Some((_, handle)) = self.current.take() {
            handle.abort();
        }
    }
}
