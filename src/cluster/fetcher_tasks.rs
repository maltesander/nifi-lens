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
    /// Concurrency cap for fanout fetchers; ignored by non-fanout
    /// endpoints (controller_status, sysdiag, bulletins, etc.).
    pub batch_concurrency: usize,
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
            let results = run_parallel(&client, &pg_ids, cfg.batch_concurrency).await;
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
                    Ok(crate::client::overview::ClusterNodesSnapshot {
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

/// Detect the standalone-NiFi shape on `/controller/cluster`. NiFi
/// emits a 409 with a body of "Only a node connected to a cluster can
/// process the request." for non-clustered servers, but
/// `nifi-rust-client` 0.11.0 maps that response to a `NifiError::NotFound`
/// (the debug repr never contains the literal "409"). We match on either
/// the explicit `NotClustered` / `409` variant — for forward compatibility
/// with future client releases — or the canonical message text.
///
/// Matches on the debug representation of the source error (the error
/// type is boxed via `NifiLensError::ClusterNodesFailed.source`). Conservative
/// — if the shape changes across nifi-rust-client versions this falls
/// back to the generic failure path and the endpoint shows as `Failed`,
/// which is safe.
fn error_is_standalone_409(err: &NifiLensError) -> bool {
    let debug_repr = format!("{err:?}");
    debug_repr.contains("409")
        || debug_repr.contains("NotClustered")
        || debug_repr.contains("Only a node connected to a cluster")
}

#[cfg(test)]
mod standalone_409_tests {
    use super::*;

    /// Construct a `NifiLensError::ClusterNodesFailed` whose source's
    /// debug repr contains the given substring. This mimics the shape the
    /// real fetcher sees from `nifi-rust-client`.
    fn err_with_debug(substring: &str) -> NifiLensError {
        // A simple error wrapper whose Debug impl emits the substring.
        #[derive(Debug)]
        struct StubError(String);
        impl std::fmt::Display for StubError {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(f, "{}", self.0)
            }
        }
        impl std::error::Error for StubError {}
        NifiLensError::ClusterNodesFailed {
            context: "test".into(),
            source: Box::new(StubError(substring.to_string())),
        }
    }

    #[test]
    fn matches_literal_409() {
        let err = err_with_debug("HTTP 409 Conflict");
        assert!(error_is_standalone_409(&err));
    }

    #[test]
    fn matches_not_clustered_variant_name() {
        let err = err_with_debug("NotClustered");
        assert!(error_is_standalone_409(&err));
    }

    #[test]
    fn matches_canonical_message_text() {
        // Exact text NiFi 2.6.0 returns; nifi-rust-client 0.11.0 maps
        // this to `NotFound` (no 409 in repr) so the message-text matcher
        // is the only path that catches it.
        let err = err_with_debug("Only a node connected to a cluster can process this request.");
        assert!(error_is_standalone_409(&err));
    }

    #[test]
    fn does_not_match_unrelated_error() {
        let err = err_with_debug("connection refused");
        assert!(!error_is_standalone_409(&err));
    }
}

/// Fan-out fetch: one `/process-groups/{id}/connections` call per PG,
/// executed concurrently with at most `concurrency` in-flight requests.
/// Per-PG errors are passed through — the caller
/// (`spawn_connections_by_pg`) emits them as `ClusterUpdate::Connections`
/// so `EndpointState::apply` can preserve `last_ok`. The downstream
/// receiver applies updates per-PG independently, so output order
/// (which `buffer_unordered` does not preserve) is irrelevant.
async fn run_parallel(
    client: &Arc<RwLock<NifiClient>>,
    pg_ids: &[String],
    concurrency: usize,
) -> Vec<(String, Result<ConnectionEndpoints, NifiLensError>)> {
    use futures::stream::{self, StreamExt};
    let guard = client.read().await;
    let context = guard.context_name().to_string();
    let concurrency = concurrency.max(1);
    // `ProcessGroups<'_>` is non-Copy and non-Clone, so we cannot move
    // a single accessor into each fan-out future. Each future borrows
    // its own accessor from the held guard — the guard outlives the
    // stream, so the borrow is sound.
    let stream = stream::iter(pg_ids.iter().map(|pg_id| {
        let pg_id = pg_id.clone();
        let pgs = guard.processgroups();
        let ctx = context.clone();
        async move {
            let res = pgs.get_connections(&pg_id).await;
            let mapped = map_res(&ctx, &pg_id, res);
            (pg_id, mapped)
        }
    }));
    stream
        .buffer_unordered(concurrency)
        .collect::<Vec<_>>()
        .await
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
            let fallback = fallback_host_port(&base_url);
            if fallback.is_none() && !http_logged {
                tracing::info!("tls_certs: non-HTTPS base_url ({base_url}); skipping probe");
                http_logged = true;
            }
            let targets = if addresses.is_empty() {
                // Pre-first-address cycle: probe `base_url` under a
                // self-describing key so the modal shows something
                // while we wait for ClusterNodes / SystemDiagnostics.
                fallback
                    .as_ref()
                    .map(|(h, p)| vec![format!("{h}:{p}")])
                    .unwrap_or_default()
            } else {
                addresses
            };

            tracing::debug!(?targets, ?fallback, "tls_certs: probing");
            let t0 = Instant::now();
            let snap = probe_all(&targets, fallback, PROBE_TIMEOUT).await;
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

/// Spawns the version-control fan-out fetch loop. Subscriber-gated
/// (Browser only). On each tick, takes the latest PG-id list from
/// `pg_ids_rx`, fans out `versions/process-groups/{id}` calls via
/// `NifiClient::version_information_batch`, and emits the resulting
/// map as `ClusterUpdate::VersionControl`. Per-PG errors degrade
/// individual rows (logged at `warn!` inside the batch helper); the
/// outer call always succeeds with whatever entries it could collect.
///
/// The select! mirrors `spawn_connections_by_pg`: waking on
/// `pg_ids_rx.changed()` lets a refreshed PG list short-circuit the
/// sleep, so newly added versioned PGs appear within one RootPgStatus
/// tick.
pub(crate) fn spawn_version_control(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    mut pg_ids_rx: watch::Receiver<Vec<String>>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let pg_ids = pg_ids_rx.borrow_and_update().clone();
            if pg_ids.is_empty() {
                tokio::select! {
                    _ = tokio::time::sleep(next_interval) => {}
                    _ = cfg.force.notified() => {}
                    _ = pg_ids_rx.changed() => {}
                }
                continue;
            }

            let t0 = Instant::now();
            // version_information_batch never returns Err — per-PG
            // failures are logged and omitted. The Result wrapper exists
            // so this fetcher's variant matches the rest of the cluster
            // store's "preserve last_ok on failure" contract.
            let map = {
                let guard = client.read().await;
                guard
                    .version_information_batch(&pg_ids, cfg.batch_concurrency)
                    .await
            };
            let duration = t0.elapsed();
            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(ClusterUpdate::VersionControl(
                    Ok(map),
                    meta,
                )))
                .await
                .is_err()
            {
                tracing::debug!("version_control fetch: channel closed, exiting");
                return;
            }

            tokio::select! {
                _ = tokio::time::sleep(next_interval) => {}
                _ = cfg.force.notified() => {}
                _ = pg_ids_rx.changed() => {}
            }
        }
    })
}

/// Spawns the parameter-context-bindings fan-out fetch loop.
/// Subscriber-gated (Browser only). On each tick, takes the latest
/// PG-id list from `pg_ids_rx`, fans out
/// `processgroups().get_process_group(id)` calls via
/// `NifiClient::parameter_context_bindings_batch`, and emits the
/// resulting map as `ClusterUpdate::ParameterContextBindings`.
///
/// The `select!` mirrors `spawn_version_control` exactly: waking on
/// `pg_ids_rx.changed()` lets a refreshed PG list short-circuit the
/// sleep so newly added PGs appear within one RootPgStatus tick.
pub(crate) fn spawn_parameter_context_bindings(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    mut pg_ids_rx: watch::Receiver<Vec<String>>,
    cfg: FetchTaskConfig,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut next_interval = cfg.base_interval;
        loop {
            if cfg.gated && !subscribers_present(&cfg.subscriber_counter) {
                cfg.force.notified().await;
                continue;
            }
            let pg_ids = pg_ids_rx.borrow_and_update().clone();
            if pg_ids.is_empty() {
                tokio::select! {
                    _ = tokio::time::sleep(next_interval) => {}
                    _ = cfg.force.notified() => {}
                    _ = pg_ids_rx.changed() => {}
                }
                continue;
            }

            let t0 = Instant::now();
            // `parameter_context_bindings_batch` never returns `Err` —
            // per-PG failures are logged and omitted. The `Result` wrapper
            // exists so this fetcher's variant matches the rest of the
            // cluster store's "preserve last_ok on failure" contract.
            let map = {
                let guard = client.read().await;
                guard
                    .parameter_context_bindings_batch(&pg_ids, cfg.batch_concurrency)
                    .await
            };
            let duration = t0.elapsed();
            next_interval = adaptive_interval(cfg.base_interval, duration, cfg.max_interval);
            let meta = FetchMeta {
                fetched_at: t0,
                fetch_duration: duration,
                next_interval,
            };
            if tx
                .send(AppEvent::ClusterUpdate(
                    ClusterUpdate::ParameterContextBindings(Ok(map), meta),
                ))
                .await
                .is_err()
            {
                tracing::debug!("parameter_context_bindings fetch: channel closed, exiting");
                return;
            }

            tokio::select! {
                _ = tokio::time::sleep(next_interval) => {}
                _ = cfg.force.notified() => {}
                _ = pg_ids_rx.changed() => {}
            }
        }
    })
}

/// Parse the context `base_url` into a `(host, port)` pair. Returns
/// `None` for non-HTTPS URLs or unparseable input. Used as the TLS
/// probe's fallback target when a node address isn't a valid
/// `host:port` (e.g. the sysdiag aggregate-fallback placeholder on
/// standalone NiFi) or before any address has been published.
fn fallback_host_port(base_url: &str) -> Option<(String, u16)> {
    let parsed = url::Url::parse(base_url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    let host = parsed.host_str()?.to_string();
    let port = parsed.port_or_known_default()?;
    Some((host, port))
}

#[cfg(test)]
mod parameter_context_bindings_fetcher_tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::AtomicUsize;
    use std::time::Duration;
    use tokio::sync::{Notify, RwLock, mpsc, watch};
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use crate::client::NifiClient;
    use crate::cluster::store::ClusterUpdate;

    async fn test_client(server: &MockServer) -> NifiClient {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"about": {"version": "2.6.0", "title": "NiFi"}}),
                ),
            )
            .mount(server)
            .await;

        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic client");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        NifiClient::from_parts(inner, "test", version)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn emits_map_after_first_tick() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let server = MockServer::start().await;

                Mock::given(method("GET"))
                    .and(path("/nifi-api/process-groups/pg-a"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "pg-a",
                        "parameterContext": {
                            "id": "ctx-1",
                            "component": { "id": "ctx-1", "name": "prod-ctx" }
                        }
                    })))
                    .mount(&server)
                    .await;

                Mock::given(method("GET"))
                    .and(path("/nifi-api/process-groups/pg-b"))
                    .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "id": "pg-b"
                    })))
                    .mount(&server)
                    .await;

                let client = test_client(&server).await;
                let client = Arc::new(RwLock::new(client));
                let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
                let (pg_ids_tx, pg_ids_rx) =
                    watch::channel(vec!["pg-a".to_string(), "pg-b".to_string()]);
                let cfg = FetchTaskConfig {
                    base_interval: Duration::from_millis(200),
                    max_interval: Duration::from_secs(5),
                    jitter_percent: 0,
                    force: Arc::new(Notify::new()),
                    gated: false,
                    subscriber_counter: Arc::new(AtomicUsize::new(1)),
                    batch_concurrency: 4,
                };

                let handle = spawn_parameter_context_bindings(client, tx, pg_ids_rx, cfg);

                let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                    .await
                    .expect("fetcher emitted no event within timeout")
                    .expect("channel closed");
                handle.abort();
                drop(pg_ids_tx);

                match event {
                    AppEvent::ClusterUpdate(ClusterUpdate::ParameterContextBindings(
                        Ok(map),
                        _meta,
                    )) => {
                        assert_eq!(
                            map.by_pg_id.len(),
                            2,
                            "expected entries for both pg-a and pg-b"
                        );
                        let binding = map
                            .by_pg_id
                            .get("pg-a")
                            .expect("pg-a present")
                            .as_ref()
                            .expect("pg-a has a binding");
                        assert_eq!(binding.id, "ctx-1");
                        assert_eq!(binding.name, "prod-ctx");
                        assert!(
                            map.by_pg_id.get("pg-b").expect("pg-b present").is_none(),
                            "pg-b has no parameter context"
                        );
                    }
                    _other => panic!("unexpected event variant"),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn parks_when_pg_ids_empty() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                // We only need to verify that the fetcher does NOT emit
                // an event when pg_ids is empty. A short timeout suffices.
                let server = MockServer::start().await;
                let client = test_client(&server).await;
                let client = Arc::new(RwLock::new(client));
                let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
                // Publish an empty PG list — fetcher should wait.
                let (_pg_ids_tx, pg_ids_rx) = watch::channel::<Vec<String>>(Vec::new());
                let cfg = FetchTaskConfig {
                    base_interval: Duration::from_millis(50),
                    max_interval: Duration::from_secs(5),
                    jitter_percent: 0,
                    force: Arc::new(Notify::new()),
                    gated: false,
                    subscriber_counter: Arc::new(AtomicUsize::new(1)),
                    batch_concurrency: 4,
                };

                let handle = spawn_parameter_context_bindings(client, tx, pg_ids_rx, cfg);

                // Fetcher should not emit within a short window.
                let result = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
                handle.abort();
                assert!(
                    result.is_err(),
                    "fetcher should not emit when pg_ids is empty"
                );
            })
            .await;
    }

    /// Drives an actual `spawn_*` fetcher through the full subscriber-gating
    /// lifecycle:
    ///   1. gated with no subscribers → must park (no events).
    ///   2. subscriber arrives (counter 0→1, `force.notify_one()`) → fires.
    ///   3. `handle.abort()` → no further events.
    ///
    /// Uses real-clock with bounded timeouts (matching the
    /// `emits_map_after_first_tick` style above). `start_paused = true`
    /// was tried first but interacts poorly with wiremock's mock-server
    /// task and `tokio::time::timeout` (paused time means timeouts never
    /// fire unless every awaiter advances time in lock-step). Real-clock
    /// gives a deterministic-enough test at the cost of a small wall-clock
    /// budget (~250ms) — acceptable per AGENTS.md test-style guidance.
    #[tokio::test(flavor = "current_thread")]
    async fn gated_fetcher_parks_until_subscriber_arrives_and_aborts_cleanly() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let server = MockServer::start().await;

                Mock::given(method("GET"))
                    .and(path("/nifi-api/process-groups/pg-1"))
                    .respond_with(
                        ResponseTemplate::new(200).set_body_json(serde_json::json!({"id": "pg-1"})),
                    )
                    .mount(&server)
                    .await;

                let client = test_client(&server).await;
                let client = Arc::new(RwLock::new(client));
                let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
                let (pg_ids_tx, pg_ids_rx) = watch::channel(vec!["pg-1".to_string()]);

                let force = Arc::new(Notify::new());
                let counter = Arc::new(AtomicUsize::new(0));
                let cfg = FetchTaskConfig {
                    base_interval: Duration::from_millis(50),
                    max_interval: Duration::from_secs(5),
                    jitter_percent: 0,
                    force: force.clone(),
                    gated: true,
                    subscriber_counter: counter.clone(),
                    batch_concurrency: 4,
                };

                let handle = spawn_parameter_context_bindings(client, tx, pg_ids_rx, cfg);

                // Step 1: gated, no subscribers — fetcher must park on
                // `force.notified()`. Wait well past one base_interval and
                // verify nothing arrives.
                let parked = tokio::time::timeout(Duration::from_millis(150), rx.recv()).await;
                assert!(
                    parked.is_err(),
                    "gated fetcher must not emit before any subscriber arrives"
                );

                // Step 2: subscribe (0 → 1) and notify. The fetcher unparks
                // and fires within one base_interval.
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                force.notify_one();

                let event = tokio::time::timeout(Duration::from_secs(2), rx.recv())
                    .await
                    .expect("fetcher should emit within 2s after subscribe")
                    .expect("channel closed");
                assert!(
                    matches!(
                        event,
                        AppEvent::ClusterUpdate(ClusterUpdate::ParameterContextBindings(_, _))
                    ),
                    "unexpected event variant"
                );

                // Step 3: abort, drain any in-flight events from the next
                // tick that may have raced ahead, then verify the channel
                // goes silent. With a 50ms base_interval the fetcher can
                // emit one more event between the Step-2 receive and the
                // abort taking effect — drain race-buffered items, then
                // wait past several intervals to prove no NEW events.
                handle.abort();
                // Yield + small wait so the abort takes effect before we
                // drain. Then drain any synchronously-buffered events
                // (try_recv is non-blocking so this terminates).
                tokio::task::yield_now().await;
                tokio::time::sleep(Duration::from_millis(100)).await;
                while rx.try_recv().is_ok() {
                    // drain race-buffered events from the post-Step-2 tick
                }
                // Now verify the channel stays silent well past several
                // intervals — proving the fetcher is truly gone. (After
                // abort, the task's sender clone is dropped and `recv()`
                // would return `Ok(None)`, so use a short timeout window.)
                let after_abort = tokio::time::timeout(Duration::from_millis(300), rx.recv()).await;
                // After abort the sender is dropped → `recv()` resolves to
                // `Ok(None)`. Either timeout (Err) OR `Ok(None)` is success;
                // any `Ok(Some(_))` means the fetcher is still alive.
                match after_abort {
                    Err(_) | Ok(None) => {}
                    Ok(Some(_)) => panic!("event arrived after abort"),
                }

                drop(pg_ids_tx);
            })
            .await;
    }
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
                    batch_concurrency: 4,
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
