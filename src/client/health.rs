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
#[derive(Debug, Clone)]
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
