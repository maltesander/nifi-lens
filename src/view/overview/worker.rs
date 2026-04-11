//! Overview tab polling worker.
//!
//! Polls the four `/flow` endpoints on a 10-second cadence and composes
//! one `ViewPayload::Overview` per cycle. Errors are logged and surfaced
//! via the app channel as `AppEvent::IntentOutcome(Err(...))` so the
//! existing banner mechanism (from Phase 0) shows them in the status bar.
//!
//! # Why `spawn_local` instead of `tokio::spawn`
//!
//! `nifi-rust-client` 0.5.0's dynamic-dispatch traits use `async fn` in
//! trait without a `+ Send` bound, so the futures they return are not
//! `Send` and cannot run on the multi-thread `tokio::spawn`. The worker
//! runs on a `LocalSet` installed on the main thread by `lib::run_inner`,
//! sharing the thread with the UI loop. The cancel mechanism is the
//! standard `JoinHandle::abort()` — no oneshot or extra runtime.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::NifiClient;
use crate::event::{AppEvent, OverviewPayload, ViewPayload};

const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Spawn the overview polling task on the current `LocalSet`. Must be
/// called from a task running under a `LocalSet` — see `lib.rs`'s
/// `run_inner`.
///
/// Returns a `JoinHandle<()>`; the caller cancels the worker by calling
/// `.abort()` on the handle.
pub fn spawn(client: Arc<RwLock<NifiClient>>, tx: mpsc::Sender<AppEvent>) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            ticker.tick().await;
            match poll_once(&client).await {
                Ok(payload) => {
                    if tx
                        .send(AppEvent::Data(ViewPayload::Overview(payload)))
                        .await
                        .is_err()
                    {
                        tracing::debug!("overview worker: channel closed, exiting");
                        return;
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "overview worker: poll failed");
                    if tx.send(AppEvent::IntentOutcome(Err(err))).await.is_err() {
                        return;
                    }
                }
            }
        }
    })
}

async fn poll_once(
    client: &Arc<RwLock<NifiClient>>,
) -> Result<OverviewPayload, crate::error::NifiLensError> {
    // The dynamic-client futures are `!Send`, so we run them sequentially
    // from within this `spawn_local` task. `tokio::try_join!` would also
    // work here (the task is local, so the combined future need not be
    // Send), but sequential calls keep the cognitive load low and match
    // the single-threaded scheduling of the LocalSet.
    let guard = client.read().await;
    let about = guard.about().await?;
    let controller = guard.controller_status().await?;
    let root_pg = guard.root_pg_status().await?;
    let bulletin_board = guard.bulletin_board(None, Some(200)).await?;
    Ok(OverviewPayload {
        about,
        controller,
        root_pg,
        bulletin_board,
        fetched_at: SystemTime::now(),
    })
}
