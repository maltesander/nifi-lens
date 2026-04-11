//! Overview tab polling worker.
//!
//! Polls four endpoints sequentially on a 10-second cadence and composes
//! one `ViewPayload::Overview` per cycle. Errors are logged and surfaced
//! via the app channel as `AppEvent::IntentOutcome(Err(...))` so the
//! existing banner mechanism (from Phase 0) shows them in the status bar.
//!
//! # Why a dedicated OS thread instead of `tokio::spawn`
//!
//! The `nifi-rust-client` dynamic-dispatch layer produces non-`Send` futures
//! (the builder chain holds a `&dyn` reference internally). `tokio::spawn`
//! requires `Send`, so the polling loop runs inside a `tokio::task::LocalSet`
//! on a dedicated OS thread that owns a single-thread Tokio runtime.
//!
//! The returned `JoinHandle<()>` is a thin wrapper. Calling `.abort()` on it
//! cancels the stop-channel, which causes the polling loop to exit at the
//! next tick.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;

use crate::client::NifiClient;
use crate::event::{AppEvent, OverviewPayload, ViewPayload};

const POLL_INTERVAL: Duration = Duration::from_secs(10);

/// Spawn the overview polling task. The caller is responsible for calling
/// `.abort()` on the returned handle when the user leaves the Overview tab.
pub fn spawn(client: Arc<RwLock<NifiClient>>, tx: mpsc::Sender<AppEvent>) -> JoinHandle<()> {
    // `stop_tx` is kept by the spawned handle; dropping it (via `.abort()`)
    // signals the worker loop to exit at the next tick.
    let (stop_tx, stop_rx) = oneshot::channel::<()>();

    // Spawn an OS thread that owns a single-thread Tokio runtime + LocalSet.
    // This lets `spawn_local` run non-`Send` futures from the dynamic client.
    std::thread::spawn(move || {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("overview worker: failed to build local runtime");

        let local = tokio::task::LocalSet::new();
        local.block_on(&rt, async move {
            let mut ticker = tokio::time::interval(POLL_INTERVAL);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            let mut stop = std::pin::pin!(stop_rx);
            loop {
                tokio::select! {
                    _ = &mut stop => {
                        tracing::debug!("overview worker: stop signal received, exiting");
                        return;
                    }
                    _ = ticker.tick() => {}
                }
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
        });
    });

    // Wrap `stop_tx` in a tokio task. When `.abort()` is called on the returned
    // `JoinHandle`, the task (and `stop_tx`) are dropped, which closes the
    // channel and signals the worker loop to exit.
    tokio::spawn(async move {
        let _stop_tx = stop_tx;
        // Park until aborted. We never actually send on the channel — we rely
        // on the drop triggered by `.abort()` to close it.
        std::future::pending::<()>().await;
    })
}

async fn poll_once(
    client: &Arc<RwLock<NifiClient>>,
) -> Result<OverviewPayload, crate::error::NifiLensError> {
    // Acquire the read lock and call each endpoint sequentially.
    // The dynamic client futures are not `Send`, so they must run on the
    // single-thread local runtime (see module-level doc).
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
