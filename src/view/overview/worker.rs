//! Overview tab polling worker. Phase 3 onwards: dual cadence.
//!
//! Two independent tickers running in `tokio::select!`:
//!   - PG status @ 10s — produces `OverviewPayload::PgStatus`
//!   - System diagnostics @ 30s — produces `OverviewPayload::SystemDiag`
//!     (or `SystemDiagFallback` if the nodewise call fails and we
//!     successfully retry with aggregate-only)
//!
//! Both pollers compose into the same `ViewPayload::Overview(...)`
//! event channel so the reducer treats them uniformly.
//!
//! # Why `spawn_local` instead of `tokio::spawn`
//!
//! `nifi-rust-client` 0.9's dynamic-dispatch traits use `async fn` in
//! trait without a `+ Send` bound, so the futures they return are not
//! `Send` and cannot run on the multi-thread `tokio::spawn`. The worker
//! runs on a `LocalSet` installed on the main thread by `lib::run_inner`,
//! sharing the thread with the UI loop. The cancel mechanism is the
//! standard `JoinHandle::abort()` — no oneshot or extra runtime.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::client::NifiClient;
use crate::event::{AppEvent, OverviewPayload, OverviewPgStatusPayload, ViewPayload};

const PG_STATUS_INTERVAL: Duration = Duration::from_secs(10);
const SYSDIAG_INTERVAL: Duration = Duration::from_secs(30);

/// Spawn the dual-cadence overview polling task on the current `LocalSet`.
/// Returns a `JoinHandle<()>`; the caller cancels the worker by calling
/// `.abort()` on the handle.
pub fn spawn(client: Arc<RwLock<NifiClient>>, tx: mpsc::Sender<AppEvent>) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut pg_ticker = tokio::time::interval(PG_STATUS_INTERVAL);
        let mut sysdiag_ticker = tokio::time::interval(SYSDIAG_INTERVAL);
        pg_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        sysdiag_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = pg_ticker.tick() => { poll_pg_status(&client, &tx).await; }
                _ = sysdiag_ticker.tick() => { poll_system_diagnostics(&client, &tx).await; }
            }
        }
    })
}

async fn poll_pg_status(client: &Arc<RwLock<NifiClient>>, tx: &mpsc::Sender<AppEvent>) {
    let guard = client.read().await;
    let about = match guard.about().await {
        Ok(a) => a,
        Err(err) => {
            tracing::warn!(error = %err, "overview worker: about() failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
            return;
        }
    };
    let controller = match guard.controller_status().await {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(error = %err, "overview worker: controller_status() failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
            return;
        }
    };
    let root_pg = match guard.root_pg_status().await {
        Ok(r) => r,
        Err(err) => {
            tracing::warn!(error = %err, "overview worker: root_pg_status() failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
            return;
        }
    };
    let bulletin_board = match guard.bulletin_board(None, Some(200)).await {
        Ok(b) => b,
        Err(err) => {
            tracing::warn!(error = %err, "overview worker: bulletin_board() failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
            return;
        }
    };
    let payload = OverviewPgStatusPayload {
        about,
        controller,
        root_pg,
        bulletin_board,
        fetched_at: SystemTime::now(),
    };
    let _ = tx
        .send(AppEvent::Data(ViewPayload::Overview(
            OverviewPayload::PgStatus(payload),
        )))
        .await;
}

async fn poll_system_diagnostics(client: &Arc<RwLock<NifiClient>>, tx: &mpsc::Sender<AppEvent>) {
    let guard = client.read().await;
    match guard.system_diagnostics(true).await {
        Ok(diag) => {
            let _ = tx
                .send(AppEvent::Data(ViewPayload::Overview(
                    OverviewPayload::SystemDiag(diag),
                )))
                .await;
        }
        Err(nodewise_err) => {
            tracing::warn!(
                error = %nodewise_err,
                "overview worker: nodewise sysdiag failed, trying aggregate fallback"
            );
            match guard.system_diagnostics(false).await {
                Ok(diag) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Overview(
                            OverviewPayload::SystemDiagFallback {
                                diag,
                                warning:
                                    "nodewise diagnostics unavailable; showing cluster aggregate"
                                        .into(),
                            },
                        )))
                        .await;
                }
                Err(agg_err) => {
                    tracing::warn!(
                        error = %agg_err,
                        "overview worker: aggregate sysdiag also failed"
                    );
                    let _ = tx.send(AppEvent::IntentOutcome(Err(nodewise_err))).await;
                }
            }
        }
    }
}
