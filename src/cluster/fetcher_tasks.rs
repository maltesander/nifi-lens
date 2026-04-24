//! Per-endpoint fetch task implementations. Each function spawns one
//! `tokio::task::spawn_local` task on the current `LocalSet` and
//! returns its `JoinHandle<()>`.
//!
//! Tasks 2–8 each add one function here; the store's `spawn_fetchers`
//! calls them during startup and context switch.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::{Duration, Instant};

use time::OffsetDateTime;
use tokio::sync::{Notify, RwLock, mpsc, watch};
use tokio::task::JoinHandle;

use crate::client::{ConnectionEndpointIds, ConnectionEndpoints, NifiClient};
use crate::cluster::fetcher::{adaptive_interval, sleep_with_jitter, subscribers_present};
use crate::cluster::snapshot::FetchMeta;
use crate::cluster::store::ClusterUpdate;
use crate::error::NifiLensError;
use crate::event::AppEvent;

/// Per-task configuration handed to each `spawn_*` function. `force` is
/// the endpoint-local `Arc<Notify>` the UI task signals on force
/// refresh or first-subscriber.
///
/// `gated` marks endpoints whose fetch loops park when `subscriber_counter`
/// reports zero. The gate runs at the top of each loop iteration — on
/// `gated: false` endpoints the guard is a cheap constant check that
/// always evaluates false, so always-on fetchers see no behavior change.
pub(crate) struct FetchTaskConfig {
    pub base_interval: Duration,
    pub max_interval: Duration,
    pub jitter_percent: u8,
    pub force: Arc<Notify>,
    pub gated: bool,
    pub subscriber_counter: Arc<std::sync::atomic::AtomicUsize>,
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
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                // Park until a subscriber arrives (0→1 transition wakes
                // `force` via `ClusterStore::subscribe`) or an explicit
                // force refresh. The immediate re-check at the loop top
                // is what makes both paths correct.
                cfg.force.notified().await;
                continue;
            }
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
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
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
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
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

/// Spawns the bulletins fetch loop. Cursor-aware: holds an
/// `Arc<AtomicI64>` initialized from the ring's `last_id` at spawn time
/// and advanced to `max(batch_id)` after each successful fetch. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::BulletinsDelta { result, meta })`
/// on every cycle — the store merges successful batches into the
/// cluster-owned `BulletinRing` and preserves `last_error` on failure.
///
/// Sentinel: a cursor of `0` is treated as "unbounded" and sent as
/// `after_id = None` to the server. NiFi bulletin ids are positive, so
/// `0` can stand in for `None` without collision.
pub(crate) fn spawn_bulletins(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cursor: Arc<AtomicI64>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let t0 = Instant::now();
            let after = cursor.load(Ordering::Relaxed);
            let after_opt = if after > 0 { Some(after) } else { None };
            // Holding the read guard across the NiFi call is safe: the
            // sole writer of `client` is the context-switch intent, which
            // tears down this store before replacing the `NifiClient`.
            let result = {
                let guard = client.read().await;
                guard.bulletin_board(after_opt, Some(1000)).await
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
            let mapped = result.map(|snap| {
                if let Some(max) = snap.bulletins.iter().map(|b| b.id).max() {
                    cursor.store(max.max(after), Ordering::Relaxed);
                }
                snap.bulletins
            });
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::BulletinsDelta {
                    result: mapped,
                    meta,
                }))
                .await
                .is_err()
            {
                tracing::debug!("bulletins fetch: channel closed, exiting");
                return;
            }

            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Spawns the system_diagnostics fetch loop. Calls
/// `system_diagnostics(nodewise=true)` first and transparently falls back
/// to `system_diagnostics(nodewise=false)` (cluster-aggregate only) when
/// the nodewise variant fails — older NiFi versions and a handful of
/// misconfigured clusters reject the nodewise query but still serve the
/// aggregate. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::SystemDiagnostics(..))` on
/// every cycle — success or failure — so `EndpointState::apply` can
/// preserve `last_ok`.
///
/// Fallback is **silent-ish**: it logs a `tracing::warn!` on each
/// nodewise→aggregate transition (tracked via a local `Option<bool>`) so
/// log-driven debugging can still see the rollover without spamming on
/// every steady-state aggregate tick. The pre-Task-8 implementation
/// surfaced the rollover as a transient UI banner via a dedicated
/// `OverviewPayload::SystemDiagFallback` variant; that banner is
/// dropped in Task 8 — see the task report.
pub(crate) fn spawn_system_diagnostics(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        // `None` before first success, `Some(true)` after a nodewise
        // success, `Some(false)` after an aggregate-fallback success.
        // Used to suppress repeat log lines in steady state.
        let mut last_nodewise: Option<bool> = None;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let t0 = Instant::now();
            // Holding the read guard across the NiFi call is safe: the
            // sole writer of `client` is the context-switch intent, which
            // tears down this store before replacing the `NifiClient`.
            let (result, used_nodewise) = {
                let guard = client.read().await;
                match guard.system_diagnostics(true).await {
                    Ok(snap) => (Ok(snap), Some(true)),
                    Err(nodewise_err) => {
                        tracing::warn!(
                            error = %nodewise_err,
                            "system_diagnostics(nodewise=true) failed; falling back to aggregate"
                        );
                        match guard.system_diagnostics(false).await {
                            Ok(snap) => (Ok(snap), Some(false)),
                            Err(agg_err) => {
                                tracing::warn!(
                                    error = %agg_err,
                                    "system_diagnostics aggregate fallback also failed"
                                );
                                // Propagate the nodewise error — it is
                                // the root cause for most operators.
                                (Err(nodewise_err), None)
                            }
                        }
                    }
                }
            };
            // Log once per transition, not once per tick. This replaces
            // the pre-Task-8 banner shown via
            // `OverviewPayload::SystemDiagFallback`.
            if let Some(nodewise) = used_nodewise
                && last_nodewise != Some(nodewise)
            {
                if nodewise {
                    tracing::info!("system_diagnostics: switched to nodewise mode");
                } else {
                    tracing::warn!("system_diagnostics: using aggregate-only fallback");
                }
                last_nodewise = Some(nodewise);
            }
            let duration = t0.elapsed();
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::SystemDiagnostics(
                    result, meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("system_diagnostics fetch: channel closed, exiting");
                return;
            }

            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Spawns the about fetch loop. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::About(..))` on every cycle —
/// success or failure — so `EndpointState::apply` can preserve
/// `last_ok`. `about` is slow-moving (NiFi version) so the default
/// cadence is 5 minutes.
pub(crate) fn spawn_about(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let t0 = Instant::now();
            // Holding the read guard across the NiFi call is safe: the
            // sole writer of `client` is the context-switch intent, which
            // tears down this store before replacing the `NifiClient`.
            let result = {
                let guard = client.read().await;
                guard.about().await
            };
            let duration = t0.elapsed();
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::About(result, meta)))
                .await
                .is_err()
            {
                tracing::debug!("about fetch: channel closed, exiting");
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
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                // Park until a subscriber arrives or a force refresh.
                // Distinct from the three-way select! at the loop bottom
                // — the select wakes on arbitrary force events *while*
                // subscribers are present; this gate wakes specifically
                // on the 0→1 `notify_one` issued by `subscribe`.
                cfg.force.notified().await;
                continue;
            }
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

/// Spawns the cluster-nodes fetch loop. Emits
/// `AppEvent::ClusterUpdate(ClusterUpdate::ClusterNodes(..))` on every
/// cycle — success or failure — so `EndpointState::apply` can preserve
/// `last_ok`.
///
/// Standalone NiFi returns HTTP 409 for `/controller/cluster`. This
/// fetcher recognizes that specific case and emits an empty-rows
/// `Ok(ClusterNodesSnapshot)` rather than a failure — the 409 is the
/// expected steady state on a non-clustered server. The fetcher logs
/// the standalone detection once (at `info`) and then stays silent.
pub(crate) fn spawn_cluster_nodes(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        let mut standalone_logged = false;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let t0 = Instant::now();
            let raw = {
                let guard = client.read().await;
                guard.cluster_nodes().await
            };
            let result = match raw {
                Ok(snap) => Ok(snap),
                Err(ref err) if error_is_standalone_409(err) => {
                    if !standalone_logged {
                        tracing::info!(
                            "cluster_nodes: standalone NiFi (409 on /controller/cluster); serving empty snapshot"
                        );
                        standalone_logged = true;
                    }
                    Ok(crate::client::health::ClusterNodesSnapshot {
                        rows: vec![],
                        fetched_at: t0,
                        fetched_wall: OffsetDateTime::now_utc(),
                    })
                }
                Err(err) => Err(err),
            };
            let duration = t0.elapsed();
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::ClusterNodes(
                    result, meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("cluster_nodes fetch: channel closed, exiting");
                return;
            }
            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Detect the standalone-NiFi 409 shape on `/controller/cluster`. Matches
/// on the debug representation of the source error (the error type is
/// boxed via `NifiLensError::ClusterNodesFailed.source`). Conservative
/// — if the shape changes across nifi-rust-client versions this falls
/// back to the generic failure path and the endpoint shows as `Failed`,
/// which is safe.
fn error_is_standalone_409(err: &NifiLensError) -> bool {
    let debug_repr = format!("{err:?}");
    debug_repr.contains("409") || debug_repr.contains("NotClustered")
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

use crate::client::tls_cert::probe_all;

/// Spawn the tls-certs fetch loop. Subscriber-gated when
/// `cfg.gated`: parks when no view subscribes, wakes via `cfg.force`.
/// Probes the addresses currently published on `addresses_rx`; falls
/// back to the bound `base_url` when the cluster list is empty
/// (standalone NiFi / pre-first-fetch).
///
/// Always emits `Ok(TlsCertsSnapshot)` — per-node failures are
/// per-entry inside the snapshot. Non-HTTPS base URLs skip probing
/// entirely (empty snapshot) with a one-time `info` log.
pub(crate) fn spawn_tls_certs(
    tx: mpsc::Sender<AppEvent>,
    addresses_rx: watch::Receiver<Vec<String>>,
    base_url: String,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        let mut http_logged = false;

        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }

            let addresses = addresses_rx.borrow().clone();
            let targets = if addresses.is_empty() {
                match fallback_target(&base_url) {
                    Some(t) => vec![t],
                    None => {
                        if !http_logged {
                            tracing::info!(
                                "tls_certs: non-HTTPS base_url ({base_url}); skipping probe"
                            );
                            http_logged = true;
                        }
                        Vec::new()
                    }
                }
            } else {
                addresses
            };

            let t0 = Instant::now();
            let snap = probe_all(&targets, PROBE_TIMEOUT).await;
            let duration = t0.elapsed();
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::TlsCerts(
                    Ok(snap),
                    meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("tls_certs fetch: channel closed, exiting");
                return;
            }
            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            sleep_with_jitter(next_interval, cfg.jitter_percent, &cfg.force).await;
        }
    })
}

/// Parse the context `base_url`. Returns `Some("host:port")` when the
/// URL is `https`; `None` for anything else (plain http, bad URL).
fn fallback_target(base_url: &str) -> Option<String> {
    let parsed = url::Url::parse(base_url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?;
    let port = parsed.port_or_known_default()?;
    Some(format!("{host}:{port}"))
}

#[cfg(test)]
mod tls_certs_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::sync::{Notify, mpsc, watch};

    #[tokio::test(flavor = "current_thread")]
    async fn spawn_tls_certs_emits_empty_snapshot_on_non_https_base_url() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
                let (_addr_tx, addr_rx) = watch::channel::<Vec<String>>(Vec::new());
                let cfg = FetchTaskConfig {
                    base_interval: Duration::from_millis(50),
                    max_interval: Duration::from_secs(5),
                    jitter_percent: 0,
                    force: Arc::new(Notify::new()),
                    gated: false,
                    subscriber_counter: Arc::new(AtomicUsize::new(1)),
                };
                let handle =
                    spawn_tls_certs(tx.clone(), addr_rx, "http://plain-nifi:8080".into(), cfg);

                // First emission should land within the base interval.
                let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
                    .await
                    .expect("fetcher emitted no event")
                    .expect("channel closed");
                handle.abort();

                match event {
                    AppEvent::ClusterUpdate(ClusterUpdate::TlsCerts(Ok(snap), _)) => {
                        assert!(
                            snap.certs.is_empty(),
                            "non-HTTPS base_url should yield empty snapshot"
                        );
                    }
                    _ => panic!("unexpected event variant"),
                }
            })
            .await;
    }
}
