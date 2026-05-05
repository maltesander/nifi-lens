//! Drop-side cleanup spawn helper.
//!
//! RAII guards in nifi-lens fire fire-and-forget HTTP DELETE calls
//! against NiFi when their owners are dropped (e.g. closing a queue
//! listing modal triggers a `DELETE /flowfile-queues/{id}/listing-requests/{id}`).
//! Drops can happen during normal modal-close (runtime alive) or during
//! shutdown / panic unwind (runtime tearing down). `tokio::spawn` panics
//! in the second case; this helper silently no-ops instead.

use std::future::Future;

/// Spawn `fut` on the current tokio runtime if one is active. If no
/// runtime is reachable from the calling thread, drop `fut` silently
/// and return `false`. Use only for fire-and-forget cleanup work that
/// is acceptable to skip during runtime shutdown.
pub fn spawn_cleanup<F>(fut: F) -> bool
where
    F: Future<Output = ()> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(fut);
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test(flavor = "multi_thread", worker_threads = 1)]
    async fn spawn_cleanup_runs_when_runtime_alive() {
        let (tx, rx) = tokio::sync::oneshot::channel::<()>();
        let scheduled = spawn_cleanup(async move {
            let _ = tx.send(());
        });
        assert!(scheduled);
        // Wait for the spawned task to signal completion. Yields the
        // current task so the single worker thread can run the spawned future.
        rx.await
            .expect("spawned cleanup task should send completion signal");
    }

    #[test]
    fn spawn_cleanup_returns_false_outside_runtime() {
        let scheduled = spawn_cleanup(async {});
        assert!(!scheduled);
    }
}
