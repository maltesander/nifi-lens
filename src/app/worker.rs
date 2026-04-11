//! WorkerRegistry: owns the one currently-running view worker task and
//! swaps it on tab change.
//!
//! Phase 1 only spawns a worker for the Overview tab; other tabs get no
//! worker (Phases 2–4 add theirs).

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

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
    ) {
        if matches!(&self.current, Some((existing, _)) if *existing == view) {
            return;
        }
        if let Some((_, handle)) = self.current.take() {
            handle.abort();
        }
        let handle = match view {
            ViewId::Overview => Some(crate::view::overview::worker::spawn(
                client.clone(),
                tx.clone(),
            )),
            // Phases 2–4 will spawn bulletins / browser / tracer workers here.
            ViewId::Bulletins | ViewId::Browser | ViewId::Tracer => None,
        };
        if let Some(handle) = handle {
            self.current = Some((view, handle));
        }
    }

    /// Abort whatever is running. Called on app shutdown.
    pub fn shutdown(&mut self) {
        if let Some((_, handle)) = self.current.take() {
            handle.abort();
        }
    }
}
