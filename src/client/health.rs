//! Health-tab client wrappers and snapshot types.

use std::time::Instant;

use nifi_rust_client::dynamic::traits::{
    FlowApi as _, FlowStatusApi as _, SystemDiagnosticsApi as _,
};
use nifi_rust_client::dynamic::types::common::StorageUsageDto;

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

// ---------------------------------------------------------------------------
// Raw data snapshots returned by the client helpers
// ---------------------------------------------------------------------------

/// Full recursive process-group status snapshot: all connections and
/// processors visible from the root PG.
#[derive(Debug, Clone)]
pub struct FullPgStatusSnapshot {
    pub connections: Vec<ConnectionStatusRow>,
    pub processors: Vec<ProcessorStatusRow>,
    pub fetched_at: Instant,
}

/// One row of connection-queue telemetry extracted from the recursive PG
/// status payload.
#[derive(Debug, Clone)]
pub struct ConnectionStatusRow {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub source_name: String,
    pub destination_name: String,
    pub percent_use_count: u32,
    pub percent_use_bytes: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub queued_display: String,
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub predicted_millis_until_backpressure: Option<i64>,
}

/// One row of processor status extracted from the recursive PG status payload.
#[derive(Debug, Clone)]
pub struct ProcessorStatusRow {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub group_path: String,
    pub active_thread_count: u32,
    pub run_status: String,
    pub tasks_duration_nanos: u64,
}

/// Full system-diagnostics snapshot (aggregate + per-node breakdown).
#[derive(Debug, Clone)]
pub struct SystemDiagSnapshot {
    pub aggregate: SystemDiagAggregate,
    pub nodes: Vec<NodeDiagnostics>,
    pub fetched_at: Instant,
}

/// Aggregate storage-repository telemetry from the system-diagnostics response.
#[derive(Debug, Clone, Default)]
pub struct SystemDiagAggregate {
    pub content_repos: Vec<RepoUsage>,
    pub flowfile_repo: Option<RepoUsage>,
    pub provenance_repos: Vec<RepoUsage>,
}

/// Utilization numbers for a single storage repository.
#[derive(Debug, Clone)]
pub struct RepoUsage {
    pub identifier: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub utilization_percent: u32,
}

/// Per-node heap, GC, thread, and load telemetry.
#[derive(Debug, Clone)]
pub struct NodeDiagnostics {
    pub address: String,
    pub heap_used_bytes: u64,
    pub heap_max_bytes: u64,
    pub gc: Vec<GcSnapshot>,
    pub load_average: Option<f64>,
    pub total_threads: u32,
    pub uptime: String,
}

/// GC collector snapshot from a single node.
#[derive(Debug, Clone)]
pub struct GcSnapshot {
    pub name: String,
    pub collection_count: u64,
    pub collection_millis: u64,
}

// ---------------------------------------------------------------------------
// Derived view-model types produced by the extraction / scoring functions
// ---------------------------------------------------------------------------

/// One row in the queue-pressure leaderboard.
#[derive(Debug, Clone)]
pub struct QueuePressureRow {
    pub connection_id: String,
    pub group_id: String,
    pub name: String,
    pub source_name: String,
    pub destination_name: String,
    pub fill_percent: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub queued_display: String,
    pub bytes_in_5m: u64,
    pub bytes_out_5m: u64,
    pub time_to_full: TimeToFull,
    pub severity: Severity,
}

/// Predicted time until a connection queue reaches backpressure.
#[derive(Debug, Clone)]
pub enum TimeToFull {
    /// Queue is draining or at equilibrium — no impending backpressure.
    Stable,
    /// Predicted seconds until backpressure at the current fill rate.
    Seconds(u64),
    /// Already at or above backpressure threshold (fill ≥ 100 %).
    Overflowing,
}

/// Traffic-light severity applied to queues, repositories, and heap.
#[derive(Debug, Clone)]
pub enum Severity {
    Green,
    Yellow,
    Red,
}

impl Severity {
    /// Severity for a connection queue based on fill percentage.
    pub fn for_queue(fill: u32) -> Self {
        if fill >= 90 {
            Self::Red
        } else if fill >= 60 {
            Self::Yellow
        } else {
            Self::Green
        }
    }

    /// Severity for a storage repository based on utilization percentage.
    pub fn for_repo(util: u32) -> Self {
        if util >= 90 {
            Self::Red
        } else if util >= 70 {
            Self::Yellow
        } else {
            Self::Green
        }
    }

    /// Severity for node heap based on utilization percentage.
    pub fn for_heap(heap: u32) -> Self {
        if heap >= 90 {
            Self::Red
        } else if heap >= 75 {
            Self::Yellow
        } else {
            Self::Green
        }
    }
}

/// One storage-repository fill bar rendered in the Health detail pane.
#[derive(Debug, Clone)]
pub struct RepoFillBar {
    pub identifier: String,
    pub used_bytes: u64,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub utilization_percent: u32,
    pub severity: Severity,
}

/// One row in the node-health table.
#[derive(Debug, Clone)]
pub struct NodeHealthRow {
    pub node_address: String,
    pub heap_used_bytes: u64,
    pub heap_max_bytes: u64,
    pub heap_percent: u32,
    pub heap_severity: Severity,
    pub gc_collection_count: u64,
    pub gc_delta: Option<u64>,
    pub gc_millis: u64,
    pub load_average: Option<f64>,
    pub uptime: String,
    pub total_threads: u32,
}

/// One row in the processor active-thread table.
#[derive(Debug, Clone)]
pub struct ProcessorThreadRow {
    pub processor_id: String,
    pub group_id: String,
    pub name: String,
    pub group_path: String,
    pub active_threads: u32,
    pub run_status: String,
    pub tasks_duration_nanos: u64,
}

// ---------------------------------------------------------------------------
// State container types
// ---------------------------------------------------------------------------

/// Maximum rows returned by the queue-pressure and processor-thread leaderboards.
pub const TOP_N: usize = 20;

/// Stateful container for the queue-pressure leaderboard.
#[derive(Debug, Default)]
pub struct QueuePressureState {
    pub rows: Vec<QueuePressureRow>,
    pub selected: usize,
}

/// Stateful container for repository fill bars.
#[derive(Debug, Default)]
pub struct RepositoryState {
    pub content: Vec<RepoFillBar>,
    pub flowfile: Option<RepoFillBar>,
    pub provenance: Vec<RepoFillBar>,
}

/// Stateful container for the node-health table.
#[derive(Debug, Default)]
pub struct NodesState {
    pub nodes: Vec<NodeHealthRow>,
    pub selected: usize,
}

/// Stateful container for the processor active-thread table.
#[derive(Debug, Default)]
pub struct ProcessorThreadState {
    pub rows: Vec<ProcessorThreadRow>,
    pub selected: usize,
}

// ---------------------------------------------------------------------------
// Pure extraction / scoring functions
// ---------------------------------------------------------------------------

/// Build a queue-pressure leaderboard from a full PG status snapshot.
///
/// Connections with zero fill on both bytes and count axes are excluded.
/// Results are sorted by fill percentage descending and truncated to `top_n`.
pub fn compute_queue_pressure(
    snapshot: &FullPgStatusSnapshot,
    top_n: usize,
) -> Vec<QueuePressureRow> {
    let mut rows: Vec<QueuePressureRow> = snapshot
        .connections
        .iter()
        .filter(|c| c.percent_use_bytes > 0 || c.percent_use_count > 0)
        .map(|c| {
            let fill_percent = c.percent_use_bytes.max(c.percent_use_count);
            let time_to_full = compute_time_to_full(c, fill_percent);
            let severity = Severity::for_queue(fill_percent);
            QueuePressureRow {
                connection_id: c.id.clone(),
                group_id: c.group_id.clone(),
                name: c.name.clone(),
                source_name: c.source_name.clone(),
                destination_name: c.destination_name.clone(),
                fill_percent,
                flow_files_queued: c.flow_files_queued,
                bytes_queued: c.bytes_queued,
                queued_display: c.queued_display.clone(),
                bytes_in_5m: c.bytes_in,
                bytes_out_5m: c.bytes_out,
                time_to_full,
                severity,
            }
        })
        .collect();

    rows.sort_by(|a, b| b.fill_percent.cmp(&a.fill_percent));
    rows.truncate(top_n);
    rows
}

/// Compute the `TimeToFull` for a single connection.
fn compute_time_to_full(c: &ConnectionStatusRow, fill_percent: u32) -> TimeToFull {
    if fill_percent >= 100 {
        return TimeToFull::Overflowing;
    }

    if let Some(ms) = c.predicted_millis_until_backpressure {
        return if ms <= 0 {
            TimeToFull::Overflowing
        } else {
            TimeToFull::Seconds((ms as u64) / 1000)
        };
    }

    // Fallback: derive from 5-minute throughput window.
    // bytes_in / bytes_out are 5-minute totals.
    let net_rate = c.bytes_in as i64 - c.bytes_out as i64;
    if net_rate <= 0 {
        return TimeToFull::Stable;
    }

    // Estimate total capacity from current fill.
    // bytes_queued / (fill_percent / 100) ≈ capacity_bytes
    let capacity_bytes = if fill_percent > 0 {
        (c.bytes_queued as f64 / (fill_percent as f64 / 100.0)) as u64
    } else {
        return TimeToFull::Stable;
    };

    let remaining_bytes = capacity_bytes.saturating_sub(c.bytes_queued);
    // net_rate is bytes per 300 seconds; convert to bytes/second
    let net_rate_per_sec = net_rate as f64 / 300.0;
    let seconds = (remaining_bytes as f64 / net_rate_per_sec) as u64;
    TimeToFull::Seconds(seconds)
}

/// Build a processor active-thread leaderboard from a full PG status snapshot.
///
/// Idle processors (active_thread_count == 0) are excluded. Results are sorted
/// by active thread count descending and truncated to `top_n`.
pub fn compute_processor_threads(
    snapshot: &FullPgStatusSnapshot,
    top_n: usize,
) -> Vec<ProcessorThreadRow> {
    let mut rows: Vec<ProcessorThreadRow> = snapshot
        .processors
        .iter()
        .filter(|p| p.active_thread_count > 0)
        .map(|p| ProcessorThreadRow {
            processor_id: p.id.clone(),
            group_id: p.group_id.clone(),
            name: p.name.clone(),
            group_path: p.group_path.clone(),
            active_threads: p.active_thread_count,
            run_status: p.run_status.clone(),
            tasks_duration_nanos: p.tasks_duration_nanos,
        })
        .collect();

    rows.sort_by(|a, b| b.active_threads.cmp(&a.active_threads));
    rows.truncate(top_n);
    rows
}

/// Map system-diagnostics aggregate repository data to display-ready fill bars.
pub fn extract_repositories(diag: &SystemDiagSnapshot) -> RepositoryState {
    let content = diag
        .aggregate
        .content_repos
        .iter()
        .map(repo_to_fill_bar)
        .collect();

    let flowfile = diag.aggregate.flowfile_repo.as_ref().map(repo_to_fill_bar);

    let provenance = diag
        .aggregate
        .provenance_repos
        .iter()
        .map(repo_to_fill_bar)
        .collect();

    RepositoryState {
        content,
        flowfile,
        provenance,
    }
}

/// Convert a `RepoUsage` to a `RepoFillBar` with severity annotation.
fn repo_to_fill_bar(r: &RepoUsage) -> RepoFillBar {
    RepoFillBar {
        identifier: r.identifier.clone(),
        used_bytes: r.used_bytes,
        total_bytes: r.total_bytes,
        free_bytes: r.free_bytes,
        utilization_percent: r.utilization_percent,
        severity: Severity::for_repo(r.utilization_percent),
    }
}

/// Refresh node-health rows from a fresh system-diagnostics snapshot.
///
/// GC deltas are computed against the previous `state.nodes` values.
/// Nodes that are new (not seen in the previous poll) receive `gc_delta = None`.
/// `state.selected` is clamped to the new row count.
pub fn update_nodes(state: &mut NodesState, diag: &SystemDiagSnapshot) {
    // Capture previous GC totals keyed by node address.
    let old_gc: std::collections::HashMap<&str, u64> = state
        .nodes
        .iter()
        .map(|n| (n.node_address.as_str(), n.gc_collection_count))
        .collect();

    state.nodes = diag
        .nodes
        .iter()
        .map(|n| {
            let gc_total: u64 = n.gc.iter().map(|g| g.collection_count).sum();
            let gc_millis: u64 = n.gc.iter().map(|g| g.collection_millis).sum();
            let gc_delta = old_gc.get(n.address.as_str()).map(|prev| gc_total - prev);

            let heap_percent = if n.heap_max_bytes > 0 {
                ((n.heap_used_bytes as f64 / n.heap_max_bytes as f64) * 100.0) as u32
            } else {
                0
            };

            NodeHealthRow {
                node_address: n.address.clone(),
                heap_used_bytes: n.heap_used_bytes,
                heap_max_bytes: n.heap_max_bytes,
                heap_percent,
                heap_severity: Severity::for_heap(heap_percent),
                gc_collection_count: gc_total,
                gc_delta,
                gc_millis,
                load_average: n.load_average,
                uptime: n.uptime.clone(),
                total_threads: n.total_threads,
            }
        })
        .collect();

    // Clamp selection to valid range.
    if !state.nodes.is_empty() {
        state.selected = state.selected.min(state.nodes.len() - 1);
    } else {
        state.selected = 0;
    }
}

// ---------------------------------------------------------------------------
// NifiClient methods
// ---------------------------------------------------------------------------

impl NifiClient {
    /// Calls `flow_api().status("root").get_process_group_status(recursive=true)`
    /// and walks the recursive snapshot to extract every connection's throughput
    /// telemetry and every processor's thread count into a [`FullPgStatusSnapshot`].
    pub async fn root_pg_status_full(&self) -> Result<FullPgStatusSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            "fetching /flow/process-groups/root/status (health full)"
        );
        let entity = self
            .inner
            .flow_api()
            .status("root")
            .get_process_group_status(Some(true), None, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProcessGroupStatusFailed {
                        context: self.context_name().to_string(),
                        source,
                    }
                })
            })?;

        let mut connections: Vec<ConnectionStatusRow> = Vec::new();
        let mut processors: Vec<ProcessorStatusRow> = Vec::new();

        if let Some(pg_dto) = entity.process_group_status
            && let Some(agg) = pg_dto.aggregate_snapshot
        {
            walk_pg_for_health(&agg, "", &mut connections, &mut processors);
        }

        Ok(FullPgStatusSnapshot {
            connections,
            processors,
            fetched_at: Instant::now(),
        })
    }

    /// Calls `GET /nifi-api/system-diagnostics?nodewise=true` and extracts
    /// aggregate repository utilization and per-node heap/GC/load telemetry
    /// into a [`SystemDiagSnapshot`].
    pub async fn system_diagnostics(
        &self,
        nodewise: bool,
    ) -> Result<SystemDiagSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            nodewise,
            "fetching /system-diagnostics"
        );
        let entity = self
            .inner
            .systemdiagnostics_api()
            .get_system_diagnostics(Some(nodewise), None, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::SystemDiagnosticsFailed {
                        context: self.context_name().to_string(),
                        source,
                    }
                })
            })?;

        let agg = entity.aggregate_snapshot.as_ref();

        let aggregate = SystemDiagAggregate {
            content_repos: extract_repo_usages(
                agg.and_then(|s| s.content_repository_storage_usage.as_deref()),
            ),
            flowfile_repo: agg
                .and_then(|s| s.flow_file_repository_storage_usage.as_ref())
                .map(map_repo_usage),
            provenance_repos: extract_repo_usages(
                agg.and_then(|s| s.provenance_repository_storage_usage.as_deref()),
            ),
        };

        let nodes = entity
            .node_snapshots
            .as_deref()
            .unwrap_or_default()
            .iter()
            .filter_map(|n| {
                let snap = n.snapshot.as_ref()?;
                let address = format!(
                    "{}:{}",
                    n.address.as_deref().unwrap_or("unknown"),
                    n.api_port.unwrap_or(0),
                );
                let gc = snap
                    .garbage_collection
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .map(|g| GcSnapshot {
                        name: g.name.clone().unwrap_or_default(),
                        collection_count: g.collection_count.unwrap_or(0).max(0) as u64,
                        collection_millis: g.collection_millis.unwrap_or(0).max(0) as u64,
                    })
                    .collect();
                Some(NodeDiagnostics {
                    address,
                    heap_used_bytes: snap.used_heap_bytes.unwrap_or(0).max(0) as u64,
                    heap_max_bytes: snap.max_heap_bytes.unwrap_or(0).max(0) as u64,
                    gc,
                    load_average: snap.processor_load_average,
                    total_threads: snap.total_threads.unwrap_or(0).max(0) as u32,
                    uptime: snap.uptime.clone().unwrap_or_default(),
                })
            })
            .collect();

        Ok(SystemDiagSnapshot {
            aggregate,
            nodes,
            fetched_at: Instant::now(),
        })
    }
}

/// Extract utilization percentage from a `StorageUsageDto`.
///
/// NiFi returns `utilization` as a string like `"78%"`. We strip the suffix and
/// parse to `u32`. If that fails we fall back to computing `used / total * 100`.
fn parse_utilization(dto: &StorageUsageDto) -> u32 {
    if let Some(s) = dto.utilization.as_deref() {
        let trimmed = s.trim_end_matches('%').trim();
        if let Ok(v) = trimmed.parse::<u32>() {
            return v;
        }
        // Handle fractional strings like "78.3%".
        if let Ok(f) = trimmed.parse::<f64>() {
            return f as u32;
        }
    }
    // Computed fallback.
    let total = dto.total_space_bytes.unwrap_or(0);
    let used = dto.used_space_bytes.unwrap_or(0);
    if total > 0 {
        ((used as f64 / total as f64) * 100.0) as u32
    } else {
        0
    }
}

fn map_repo_usage(dto: &StorageUsageDto) -> RepoUsage {
    RepoUsage {
        identifier: dto.identifier.clone().unwrap_or_default(),
        used_bytes: dto.used_space_bytes.unwrap_or(0).max(0) as u64,
        total_bytes: dto.total_space_bytes.unwrap_or(0).max(0) as u64,
        free_bytes: dto.free_space_bytes.unwrap_or(0).max(0) as u64,
        utilization_percent: parse_utilization(dto),
    }
}

fn extract_repo_usages(repos: Option<&[StorageUsageDto]>) -> Vec<RepoUsage> {
    repos
        .unwrap_or_default()
        .iter()
        .map(map_repo_usage)
        .collect()
}

/// Threshold above which a prediction value is treated as "infinity" / not useful.
/// NiFi emits `i64::MAX` when it cannot produce a meaningful prediction.
const PREDICTION_INFINITY_THRESHOLD: i64 = i64::MAX / 2;

/// Recursive walker that extracts connections and processors from a
/// `ProcessGroupStatusSnapshotDto`. Mirrors `walk_pg_snapshot` in
/// `src/client/browser.rs` but harvests throughput and thread-count fields
/// instead of building an arena.
fn walk_pg_for_health(
    snap: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    parent_path: &str,
    connections: &mut Vec<ConnectionStatusRow>,
    processors: &mut Vec<ProcessorStatusRow>,
) {
    let pg_name = snap.name.as_deref().unwrap_or_default();
    let group_path = if parent_path.is_empty() {
        pg_name.to_string()
    } else {
        format!("{parent_path} / {pg_name}")
    };

    // Extract connections.
    if let Some(conns) = snap.connection_status_snapshots.as_ref() {
        for entity in conns {
            let Some(c) = entity.connection_status_snapshot.as_ref() else {
                continue;
            };
            let predicted = c.predictions.as_ref().and_then(|p| {
                let by_bytes = p
                    .predicted_millis_until_bytes_backpressure
                    .filter(|&v| v < PREDICTION_INFINITY_THRESHOLD);
                let by_count = p
                    .predicted_millis_until_count_backpressure
                    .filter(|&v| v < PREDICTION_INFINITY_THRESHOLD);
                match (by_bytes, by_count) {
                    (None, None) => None,
                    (Some(b), None) => Some(b),
                    (None, Some(c)) => Some(c),
                    (Some(b), Some(c)) => Some(b.min(c)),
                }
            });
            connections.push(ConnectionStatusRow {
                id: c.id.clone().unwrap_or_default(),
                group_id: c.group_id.clone().unwrap_or_default(),
                name: c.name.clone().unwrap_or_default(),
                source_name: c.source_name.clone().unwrap_or_default(),
                destination_name: c.destination_name.clone().unwrap_or_default(),
                percent_use_count: c.percent_use_count.unwrap_or(0).max(0) as u32,
                percent_use_bytes: c.percent_use_bytes.unwrap_or(0).max(0) as u32,
                flow_files_queued: c.flow_files_queued.unwrap_or(0).max(0) as u32,
                bytes_queued: c.bytes_queued.unwrap_or(0).max(0) as u64,
                queued_display: c.queued.clone().unwrap_or_default(),
                bytes_in: c.bytes_in.unwrap_or(0).max(0) as u64,
                bytes_out: c.bytes_out.unwrap_or(0).max(0) as u64,
                predicted_millis_until_backpressure: predicted,
            });
        }
    }

    // Extract processors.
    if let Some(procs) = snap.processor_status_snapshots.as_ref() {
        for entity in procs {
            let Some(p) = entity.processor_status_snapshot.as_ref() else {
                continue;
            };
            processors.push(ProcessorStatusRow {
                id: p.id.clone().unwrap_or_default(),
                group_id: p.group_id.clone().unwrap_or_default(),
                name: p.name.clone().unwrap_or_default(),
                group_path: group_path.clone(),
                active_thread_count: p.active_thread_count.unwrap_or(0).max(0) as u32,
                run_status: p.run_status.clone().unwrap_or_default(),
                tasks_duration_nanos: p.tasks_duration_nanos.unwrap_or(0).max(0) as u64,
            });
        }
    }

    // Recurse into child PGs.
    if let Some(children) = snap.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                walk_pg_for_health(child, &group_path, connections, processors);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Unit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;

    // -----------------------------------------------------------------------
    // Test helpers
    // -----------------------------------------------------------------------

    fn conn(
        id: &str,
        pct_bytes: u32,
        pct_count: u32,
        bytes_in: u64,
        bytes_out: u64,
        bytes_queued: u64,
        predicted_ms: Option<i64>,
    ) -> ConnectionStatusRow {
        ConnectionStatusRow {
            id: id.to_string(),
            group_id: "g".to_string(),
            name: id.to_string(),
            source_name: "src".to_string(),
            destination_name: "dst".to_string(),
            percent_use_count: pct_count,
            percent_use_bytes: pct_bytes,
            flow_files_queued: 0,
            bytes_queued,
            queued_display: String::new(),
            bytes_in,
            bytes_out,
            predicted_millis_until_backpressure: predicted_ms,
        }
    }

    fn snap_with_conns(conns: Vec<ConnectionStatusRow>) -> FullPgStatusSnapshot {
        FullPgStatusSnapshot {
            connections: conns,
            processors: Vec::new(),
            fetched_at: Instant::now(),
        }
    }

    fn snap_with_procs(procs: Vec<ProcessorStatusRow>) -> FullPgStatusSnapshot {
        FullPgStatusSnapshot {
            connections: Vec::new(),
            processors: procs,
            fetched_at: Instant::now(),
        }
    }

    fn proc_row(id: &str, active_threads: u32) -> ProcessorStatusRow {
        ProcessorStatusRow {
            id: id.to_string(),
            group_id: "g".to_string(),
            name: id.to_string(),
            group_path: "root".to_string(),
            active_thread_count: active_threads,
            run_status: "Running".to_string(),
            tasks_duration_nanos: 0,
        }
    }

    fn repo(id: &str, util_pct: u32) -> RepoUsage {
        RepoUsage {
            identifier: id.to_string(),
            used_bytes: util_pct as u64 * 1_000,
            total_bytes: 100_000,
            free_bytes: (100 - util_pct) as u64 * 1_000,
            utilization_percent: util_pct,
        }
    }

    fn diag_snap(
        content_repos: Vec<RepoUsage>,
        flowfile_repo: Option<RepoUsage>,
        provenance_repos: Vec<RepoUsage>,
        nodes: Vec<NodeDiagnostics>,
    ) -> SystemDiagSnapshot {
        SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos,
                flowfile_repo,
                provenance_repos,
            },
            nodes,
            fetched_at: Instant::now(),
        }
    }

    fn node_diag(address: &str, heap_used: u64, heap_max: u64, gc_count: u64) -> NodeDiagnostics {
        NodeDiagnostics {
            address: address.to_string(),
            heap_used_bytes: heap_used,
            heap_max_bytes: heap_max,
            gc: vec![GcSnapshot {
                name: "G1".to_string(),
                collection_count: gc_count,
                collection_millis: gc_count * 10,
            }],
            load_average: Some(1.0),
            total_threads: 50,
            uptime: "1h".to_string(),
        }
    }

    // -----------------------------------------------------------------------
    // compute_queue_pressure tests
    // -----------------------------------------------------------------------

    #[test]
    fn queue_pressure_sorts_by_fill_descending_and_omits_zero() {
        let conns = vec![
            conn("a", 10, 0, 0, 0, 1000, None),
            conn("b", 90, 0, 0, 0, 9000, None),
            conn("c", 0, 0, 0, 0, 0, None), // zero — must be omitted
            conn("d", 50, 0, 0, 0, 5000, None),
        ];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, TOP_N);

        assert_eq!(rows.len(), 3, "zero-fill connection must be omitted");
        assert_eq!(rows[0].fill_percent, 90);
        assert_eq!(rows[1].fill_percent, 50);
        assert_eq!(rows[2].fill_percent, 10);
    }

    #[test]
    fn queue_pressure_top_n_truncates() {
        let conns = vec![
            conn("a", 10, 0, 0, 0, 1000, None),
            conn("b", 90, 0, 0, 0, 9000, None),
            conn("c", 50, 0, 0, 0, 5000, None),
        ];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, 2);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].fill_percent, 90);
        assert_eq!(rows[1].fill_percent, 50);
    }

    #[test]
    fn queue_pressure_uses_server_prediction_when_available() {
        let conns = vec![conn("a", 50, 0, 100, 0, 5000, Some(840_000))];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, TOP_N);
        assert!(matches!(rows[0].time_to_full, TimeToFull::Seconds(840)));
    }

    #[test]
    fn queue_pressure_fallback_filling_queue() {
        // bytes_in 1000, bytes_out 400 over 5 min → net 600 bytes / 300 s = 2 bytes/s
        // fill=50%, queued=5000, capacity≈10000, remaining=5000
        // seconds ≈ 5000 / 2 = 2500
        let conns = vec![conn("a", 50, 0, 1000, 400, 5000, None)];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, TOP_N);
        match &rows[0].time_to_full {
            TimeToFull::Seconds(s) => assert!(*s > 0, "expected positive seconds"),
            other => panic!("expected Seconds, got {other:?}"),
        }
    }

    #[test]
    fn queue_pressure_fallback_draining_queue() {
        // bytes_out > bytes_in → net_rate ≤ 0 → Stable
        let conns = vec![conn("a", 50, 0, 100, 500, 5000, None)];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, TOP_N);
        assert!(matches!(rows[0].time_to_full, TimeToFull::Stable));
    }

    #[test]
    fn queue_pressure_overflowing() {
        let conns = vec![conn("a", 100, 0, 0, 0, 10000, None)];
        let snap = snap_with_conns(conns);
        let rows = compute_queue_pressure(&snap, TOP_N);
        assert!(matches!(rows[0].time_to_full, TimeToFull::Overflowing));
    }

    #[test]
    fn queue_pressure_severity_thresholds() {
        // Severity::for_queue: ≥90→Red, ≥60→Yellow, else Green
        let cases = [
            (49_u32, false, false), // Green
            (60_u32, false, true),  // Yellow
            (79_u32, false, true),  // Yellow
            (90_u32, true, false),  // Red
        ];
        for (pct, expect_red, expect_yellow) in cases {
            let conns = vec![conn("a", pct, 0, 0, 0, pct as u64 * 100, None)];
            let snap = snap_with_conns(conns);
            let rows = compute_queue_pressure(&snap, TOP_N);
            let sev = &rows[0].severity;
            if expect_red {
                assert!(matches!(sev, Severity::Red), "pct={pct} should be Red");
            } else if expect_yellow {
                assert!(
                    matches!(sev, Severity::Yellow),
                    "pct={pct} should be Yellow"
                );
            } else {
                assert!(matches!(sev, Severity::Green), "pct={pct} should be Green");
            }
        }
    }

    // -----------------------------------------------------------------------
    // compute_processor_threads tests
    // -----------------------------------------------------------------------

    #[test]
    fn processor_threads_sorts_descending_omits_idle() {
        let procs = vec![
            proc_row("idle", 0),
            proc_row("busy5", 5),
            proc_row("busiest8", 8),
        ];
        let snap = snap_with_procs(procs);
        let rows = compute_processor_threads(&snap, TOP_N);
        assert_eq!(rows.len(), 2, "idle processor must be omitted");
        assert_eq!(rows[0].active_threads, 8);
        assert_eq!(rows[1].active_threads, 5);
    }

    // -----------------------------------------------------------------------
    // extract_repositories tests
    // -----------------------------------------------------------------------

    #[test]
    fn extract_repositories_maps_all_three() {
        let snap = diag_snap(
            vec![repo("content-1", 60)],
            Some(repo("flowfile", 30)),
            vec![repo("provenance-1", 91)],
            Vec::new(),
        );
        let state = extract_repositories(&snap);

        assert_eq!(state.content.len(), 1);
        // for_repo: ≥90→Red, ≥70→Yellow, else Green. 60 → Green.
        assert!(matches!(state.content[0].severity, Severity::Green));
        assert_eq!(state.content[0].utilization_percent, 60);

        let ff = state.flowfile.as_ref().expect("flowfile must be Some");
        assert!(matches!(ff.severity, Severity::Green)); // 30 → Green
        assert_eq!(ff.utilization_percent, 30);

        assert_eq!(state.provenance.len(), 1);
        assert!(matches!(state.provenance[0].severity, Severity::Red)); // 91 → Red
        assert_eq!(state.provenance[0].utilization_percent, 91);
    }

    // -----------------------------------------------------------------------
    // update_nodes tests
    // -----------------------------------------------------------------------

    #[test]
    fn update_nodes_computes_gc_delta_on_second_poll() {
        let mut state = NodesState::default();

        // First poll: gc_count = 10
        let snap1 = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8080", 1_000, 8_000, 10)],
        );
        update_nodes(&mut state, &snap1);
        assert_eq!(state.nodes.len(), 1);
        assert!(state.nodes[0].gc_delta.is_none(), "first poll → no delta");
        assert_eq!(state.nodes[0].gc_collection_count, 10);

        // Second poll: gc_count = 15 → delta = 5
        let snap2 = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8080", 1_000, 8_000, 15)],
        );
        update_nodes(&mut state, &snap2);
        assert_eq!(state.nodes[0].gc_delta, Some(5));
        assert_eq!(state.nodes[0].gc_collection_count, 15);
    }

    #[test]
    fn update_nodes_new_node_gets_none_delta() {
        let mut state = NodesState::default();

        // First poll: only node1
        let snap1 = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8080", 1_000, 8_000, 10)],
        );
        update_nodes(&mut state, &snap1);

        // Second poll: node1 + new node2
        let snap2 = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![
                node_diag("node1:8080", 1_000, 8_000, 15),
                node_diag("node2:8080", 2_000, 8_000, 3),
            ],
        );
        update_nodes(&mut state, &snap2);

        let node1 = state
            .nodes
            .iter()
            .find(|n| n.node_address == "node1:8080")
            .unwrap();
        let node2 = state
            .nodes
            .iter()
            .find(|n| n.node_address == "node2:8080")
            .unwrap();

        assert_eq!(node1.gc_delta, Some(5));
        assert!(
            node2.gc_delta.is_none(),
            "brand-new node must get None delta"
        );
    }
}
