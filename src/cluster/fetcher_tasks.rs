//! Per-endpoint fetch task implementations. Each function spawns one
//! `tokio::task::spawn_local` task on the current `LocalSet` and
//! returns its `JoinHandle<()>`.
//!
//! Tasks 2–8 each add one function here; the store's `spawn_fetchers`
//! calls them during startup and context switch.

use std::sync::Arc;
use std::time::{Duration, Instant};

use tokio::sync::{Notify, RwLock, mpsc, watch};
use tokio::task::JoinHandle;

use crate::client::{ConnectionEndpointIds, ConnectionEndpoints, NifiClient};
use crate::cluster::fetcher::{adaptive_interval, sleep_with_jitter};
use crate::cluster::snapshot::FetchMeta;
use crate::cluster::store::ClusterUpdate;
use crate::error::NifiLensError;
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

/// Spawns the controller_services fetch loop. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::ControllerServices(..))` on
/// every cycle — success or failure — so the store can preserve
/// `last_ok` via `EndpointState::apply`.
pub(crate) fn spawn_controller_services(
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
                guard.controller_services_snapshot().await
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
                .send(AppEvent::ClusterUpdate(ClusterUpdate::ControllerServices(
                    result, meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("controller_services fetch: channel closed, exiting");
                return;
            }

            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Spawns the connections-by-PG fan-out fetch loop. Unlike the other
/// per-endpoint fetchers this one consumes a `watch::Receiver<Vec<String>>`
/// published by `ClusterStore::publish_pg_ids` — the list is rebuilt each
/// time `RootPgStatus` succeeds. Emits one
/// `AppEvent::ClusterUpdate(ClusterUpdate::Connections { .. })` per PG on
/// every cycle (success or failure).
///
/// The `select!` is three-way rather than the usual two: waking on
/// `pg_ids_rx.changed()` lets a fresh PG list short-circuit the sleep,
/// so newly added PGs pick up a fetch on the next RootPgStatus tick
/// rather than waiting a full `base_interval`. This also means the
/// fetcher implicitly inherits jitter from RootPgStatus (the publisher's
/// own jittered cadence drives `changed()`), so the inner `sleep` stays
/// unjittered — see plan note line 1646.
pub(crate) fn spawn_connections_by_pg(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    mut pg_ids_rx: watch::Receiver<Vec<String>>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            let pg_ids = pg_ids_rx.borrow_and_update().clone();
            if pg_ids.is_empty() {
                // No PGs known yet (or the flow has none). Wait for a
                // publish or a force notify — but also respect the
                // sleep timer so we don't spin forever if RootPgStatus
                // stalls.
                tokio::select! {
                    _ = tokio::time::sleep(next_interval) => {}
                    _ = cfg.force.notified() => {}
                    _ = pg_ids_rx.changed() => {}
                }
                continue;
            }

            let t0 = Instant::now();
            let results = run_parallel(&client, &pg_ids).await;
            let duration = t0.elapsed();
            // Per-PG `next_interval` mirrors the base cadence — each PG
            // fetch is cheap and independent, so we don't back off on
            // single-PG slowness. Still, feed the observed `duration`
            // into the adaptive formula so a slow cluster backs the
            // whole fan-out off uniformly.
            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            for (pg_id, result) in results {
                if tx
                    .send(AppEvent::ClusterUpdate(ClusterUpdate::Connections {
                        pg_id,
                        result,
                        meta,
                    }))
                    .await
                    .is_err()
                {
                    tracing::debug!("connections_by_pg fetch: channel closed, exiting");
                    return;
                }
            }

            tokio::select! {
                _ = tokio::time::sleep(next_interval) => {}
                _ = cfg.force.notified() => {}
                _ = pg_ids_rx.changed() => {}
            }
        }
    })
}

/// Fan-out fetch: one `/process-groups/{id}/connections` call per PG,
/// executed in parallel. Returns the results keyed by PG id, preserving
/// input order. Per-PG errors are passed through — the caller
/// (`spawn_connections_by_pg`) emits them as `ClusterUpdate::Connections`
/// so `EndpointState::apply` can preserve `last_ok`.
async fn run_parallel(
    client: &Arc<RwLock<NifiClient>>,
    pg_ids: &[String],
) -> Vec<(String, Result<ConnectionEndpoints, NifiLensError>)> {
    let guard = client.read().await;
    let context = guard.context_name().to_string();
    // `ProcessGroups<'_>` is non-Copy and non-Clone, so we cannot move
    // a single accessor into each fan-out future. Each future borrows
    // its own accessor from the held guard — the guard outlives the
    // `join_all`, so the borrow is sound.
    let futs = pg_ids.iter().map(|id| {
        let pgs = guard.processgroups();
        let ctx = &context;
        async move {
            let res = pgs.get_connections(id).await;
            (id.clone(), map_res(ctx, id, res))
        }
    });
    futures::future::join_all(futs).await
}

/// Collapse the raw `ConnectionsEntity` DTO into a
/// `ConnectionEndpoints` row — a `HashMap<conn_id, ConnectionEndpointIds>`
/// that the Browser reducer (Task 6) merges into the arena. Connections
/// missing an id are dropped; empty DTOs produce an empty map.
///
/// Mirrors the mapping `browser_tree` used to do inline before the
/// fan-out moved into the cluster store.
fn map_res(
    context: &str,
    pg_id: &str,
    res: Result<nifi_rust_client::dynamic::types::ConnectionsEntity, nifi_rust_client::NifiError>,
) -> Result<ConnectionEndpoints, NifiLensError> {
    match res {
        Ok(conns_entity) => {
            let mut by_connection = std::collections::HashMap::new();
            for entity in conns_entity.connections.unwrap_or_default() {
                let Some(conn_id) = entity.id.clone() else {
                    continue;
                };
                by_connection.insert(
                    conn_id,
                    ConnectionEndpointIds {
                        source_id: entity.source_id.clone().unwrap_or_default(),
                        destination_id: entity.destination_id.clone().unwrap_or_default(),
                    },
                );
            }
            Ok(ConnectionEndpoints { by_connection })
        }
        Err(err) => {
            tracing::warn!(
                %context,
                %pg_id,
                error = %err,
                "connections_by_pg fetch failed for PG"
            );
            Err(crate::client::classify_or_fallback(
                context,
                Box::new(err),
                |source| NifiLensError::PgConnectionsFetchFailed {
                    context: context.to_string(),
                    id: pg_id.to_string(),
                    source,
                },
            ))
        }
    }
}
