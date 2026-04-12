//! Health-tab client wrappers and snapshot types.

use std::time::Instant;

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
