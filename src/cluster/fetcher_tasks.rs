//! Per-endpoint fetch task implementations. Each function spawns one
//! `tokio::task::spawn_local` task on the current `LocalSet` and
//! returns its `JoinHandle<()>`.
//!
//! Tasks 2–8 each add one function here; the store's `spawn_fetchers`
//! calls them during startup and context switch.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Notify, RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::NifiClient;
use crate::cluster::fetcher::{adaptive_interval, sleep_with_jitter};
use crate::cluster::snapshot::FetchMeta;
use crate::cluster::store::ClusterUpdate;
use crate::event::AppEvent;

/// Per-task configuration handed to each `spawn_*` function. `force` is
/// the endpoint-local `Arc<Notify>` the UI task signals on force
/// refresh or first-subscriber.
pub(crate) struct FetchTaskConfig {
    pub base_interval: Duration,
    pub max_interval: Duration,
    pub jitter_percent: u8,
    pub force: Arc<Notify>,
}

/// Spawns the controller_status fetch loop. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::ControllerStatus(..))` on
/// every cycle — success or failure — so the store can preserve
/// `last_ok` via `EndpointState::apply`.
pub(crate) fn spawn_controller_status(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            let t0 = Instant::now();
            // Holding the read guard across the NiFi call is safe: the
            // sole writer of `client` is the context-switch intent, which
            // tears down this store before replacing the `NifiClient`.
            let result = {
                let guard = client.read().await;
                guard.controller_status().await
            };
            let duration = t0.elapsed();
            let meta = FetchMeta {
                // Timestamp the *request*, not the response. On a slow
                // cluster where fetch_duration is seconds, the data
                // already represents state at `t0`, not now.
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::ControllerStatus(
                    result, meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("controller_status fetch: channel closed, exiting");
                return;
            }

            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Spawns the root_pg_status fetch loop. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::RootPgStatus(..))` on every
/// cycle — success or failure — so the store can preserve `last_ok`
/// via `EndpointState::apply`.
pub(crate) fn spawn_root_pg_status(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            let t0 = Instant::now();
            // Holding the read guard across the NiFi call is safe: the
            // sole writer of `client` is the context-switch intent, which
            // tears down this store before replacing the `NifiClient`.
            let result = {
                let guard = client.read().await;
                guard.root_pg_status().await
            };
            let duration = t0.elapsed();
            let meta = FetchMeta {
                // Timestamp the *request*, not the response. On a slow
                // cluster where fetch_duration is seconds, the data
                // already represents state at `t0`, not now.
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::RootPgStatus(
                    result, meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("root_pg_status fetch: channel closed, exiting");
                return;
            }

            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}
