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

use crate::app::worker::send_poll_result;
use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::event::{AppEvent, OverviewPayload, OverviewPgStatusPayload, ViewPayload};

/// Spawn the dual-cadence overview polling task on the current `LocalSet`.
/// Returns a `JoinHandle<()>`; the caller cancels the worker by calling
/// `.abort()` on the handle.
pub fn spawn(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    pg_status: Duration,
    sysdiag: Duration,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut pg_ticker = tokio::time::interval(pg_status);
        let mut sysdiag_ticker = tokio::time::interval(sysdiag);
        pg_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        sysdiag_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = pg_ticker.tick() => {
                    send_poll_result(&tx, "overview pg_status", pg_status_payload(&client).await).await;
                }
                _ = sysdiag_ticker.tick() => {
                    send_poll_result(&tx, "overview sysdiag", sysdiag_payload(&client).await).await;
                }
            }
        }
    })
}

async fn pg_status_payload(client: &Arc<RwLock<NifiClient>>) -> Result<ViewPayload, NifiLensError> {
    let guard = client.read().await;
    let about = guard.about().await?;
    let controller = guard.controller_status().await?;
    let root_pg = guard.root_pg_status().await?;
    let bulletin_board = guard.bulletin_board(None, Some(200)).await?;
    Ok(ViewPayload::Overview(OverviewPayload::PgStatus(
        OverviewPgStatusPayload {
            about,
            controller,
            root_pg,
            bulletin_board,
            cs_counts: None,
            fetched_at: SystemTime::now(),
        },
    )))
}

/// Poll system-diagnostics with nodewise, falling back to aggregate-only
/// if the nodewise call is rejected (older NiFi versions or a
/// misconfigured cluster). If both fail, the nodewise error is
/// propagated.
async fn sysdiag_payload(client: &Arc<RwLock<NifiClient>>) -> Result<ViewPayload, NifiLensError> {
    let guard = client.read().await;
    match guard.system_diagnostics(true).await {
        Ok(diag) => Ok(ViewPayload::Overview(OverviewPayload::SystemDiag(diag))),
        Err(nodewise_err) => {
            tracing::warn!(
                error = %nodewise_err,
                "overview worker: nodewise sysdiag failed, trying aggregate fallback"
            );
            match guard.system_diagnostics(false).await {
                Ok(diag) => Ok(ViewPayload::Overview(OverviewPayload::SystemDiagFallback {
                    diag,
                    warning: "nodewise diagnostics unavailable; showing cluster aggregate".into(),
                })),
                Err(_agg_err) => Err(nodewise_err),
            }
        }
    }
}
