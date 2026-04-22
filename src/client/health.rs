//! Health-tab client wrappers and snapshot types.

use std::time::Instant;

use nifi_rust_client::dynamic::types::common::StorageUsageDto;
use time::OffsetDateTime;

use crate::app::navigation::ListNavigation;
use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

// ---------------------------------------------------------------------------
// Raw data snapshots returned by the client helpers
// ---------------------------------------------------------------------------

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

/// Per-node heap, GC, thread, load, and repository telemetry.
#[derive(Debug, Clone)]
pub struct NodeDiagnostics {
    pub address: String,
    pub heap_used_bytes: u64,
    pub heap_max_bytes: u64,
    pub gc: Vec<GcSnapshot>,
    pub load_average: Option<f64>,
    pub available_processors: Option<u32>,
    pub total_threads: u32,
    pub uptime: String,
    pub content_repos: Vec<RepoUsage>,
    pub flowfile_repo: Option<RepoUsage>,
    pub provenance_repos: Vec<RepoUsage>,
}

/// GC collector snapshot from a single node.
#[derive(Debug, Clone)]
pub struct GcSnapshot {
    pub name: String,
    pub collection_count: u64,
    pub collection_millis: u64,
}

/// Traffic-light severity applied to queues, repositories, and heap.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
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

/// Cluster-membership state for a single node, from
/// `/controller/cluster` `NodeDTO.status`. Unknown wire values map to
/// `Other` — kept unit-typed so the enum stays `Copy`/`Eq`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClusterNodeStatus {
    Connected,
    Connecting,
    Disconnected,
    Disconnecting,
    Offloading,
    Offloaded,
    Other,
}

impl ClusterNodeStatus {
    /// Map a NiFi wire status to the typed enum. Unknown values
    /// collapse to `Other` so new NiFi versions never panic.
    pub fn from_wire(s: &str) -> Self {
        match s {
            "CONNECTED" => Self::Connected,
            "CONNECTING" => Self::Connecting,
            "DISCONNECTED" => Self::Disconnected,
            "DISCONNECTING" => Self::Disconnecting,
            "OFFLOADING" => Self::Offloading,
            "OFFLOADED" => Self::Offloaded,
            _ => Self::Other,
        }
    }

    /// Returns `true` when the node is not expected to report fresh
    /// telemetry. The Nodes panel dims these rows and shows `───`
    /// placeholders for heap/gc/load columns.
    pub fn is_dead(self) -> bool {
        matches!(
            self,
            Self::Disconnected | Self::Disconnecting | Self::Offloading | Self::Offloaded,
        )
    }
}

/// One historical event recorded against a cluster node. Newest-first
/// inside `ClusterNodeRow.events`; capped to 8 at parse time.
#[derive(Debug, Clone)]
pub struct ClusterNodeEvent {
    pub timestamp_iso: String,
    pub category: Option<String>,
    pub message: String,
}

/// Per-node cluster-membership row from `/controller/cluster`.
#[derive(Debug, Clone)]
pub struct ClusterNodeRow {
    pub node_id: String,
    /// `"host:apiPort"` — composed identically to `SystemDiagSnapshot`
    /// node addresses so the Overview reducer can join on this string.
    pub address: String,
    pub status: ClusterNodeStatus,
    pub is_primary: bool,
    pub is_coordinator: bool,
    pub heartbeat_iso: Option<String>,
    pub node_start_iso: Option<String>,
    pub active_thread_count: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub events: Vec<ClusterNodeEvent>,
}

/// Snapshot of `GET /controller/cluster`. `fetched_at` is the reducer-
/// anchor used to compute per-node heartbeat ages.
///
/// No `Default` impl: `Instant` has no sensible default, and the
/// sibling `SystemDiagSnapshot` is declared the same way.
#[derive(Debug, Clone)]
pub struct ClusterNodesSnapshot {
    pub rows: Vec<ClusterNodeRow>,
    pub fetched_at: std::time::Instant,
    /// Wall-clock anchor captured at fetch time. Used by the `update_nodes`
    /// reducer to compute per-node heartbeat ages deterministically (an
    /// `Instant` cannot be subtracted from a wall-clock `OffsetDateTime`
    /// parsed from a NiFi heartbeat string).
    pub fetched_wall: OffsetDateTime,
}

/// Per-node cluster-membership view joined into `NodeHealthRow`.
/// `heartbeat_age` is materialized at reducer time so the list and the
/// modal see identical values and snapshot tests stay deterministic.
#[derive(Debug, Clone)]
pub struct ClusterMembership {
    pub node_id: String,
    pub status: ClusterNodeStatus,
    pub is_primary: bool,
    pub is_coordinator: bool,
    pub heartbeat_age: Option<std::time::Duration>,
    pub node_start_iso: Option<String>,
    pub active_thread_count: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub events: Vec<ClusterNodeEvent>,
}

const MAX_NODE_EVENTS: usize = 8;

impl ClusterNodesSnapshot {
    /// Project a `ClusterDto` into the store-owned snapshot shape. Pure;
    /// no I/O. Unknown wire statuses become `ClusterNodeStatus::Other`.
    /// Events are truncated to `MAX_NODE_EVENTS`, preserving newest-first
    /// order as NiFi returns them.
    pub fn from_cluster_dto(dto: &nifi_rust_client::dynamic::types::ClusterDto) -> Self {
        let rows = dto
            .nodes
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|n| {
                let address = format!(
                    "{}:{}",
                    n.address.as_deref().unwrap_or("unknown"),
                    n.api_port.unwrap_or(0),
                );
                let roles = n.roles.as_deref().unwrap_or_default();
                let is_primary = roles.iter().any(|r| r == "Primary Node");
                let is_coordinator = roles.iter().any(|r| r == "Cluster Coordinator");
                let events = n
                    .events
                    .as_deref()
                    .unwrap_or_default()
                    .iter()
                    .take(MAX_NODE_EVENTS)
                    .map(|e| ClusterNodeEvent {
                        timestamp_iso: e.timestamp.clone().unwrap_or_default(),
                        category: e.category.clone(),
                        message: e.message.clone().unwrap_or_default(),
                    })
                    .collect();
                ClusterNodeRow {
                    node_id: n.node_id.clone().unwrap_or_default(),
                    address,
                    status: ClusterNodeStatus::from_wire(n.status.as_deref().unwrap_or("")),
                    is_primary,
                    is_coordinator,
                    heartbeat_iso: n.heartbeat.clone(),
                    node_start_iso: n.node_start_time.clone(),
                    active_thread_count: n.active_thread_count.unwrap_or(0).max(0) as u32,
                    flow_files_queued: n.flow_files_queued.unwrap_or(0).max(0) as u32,
                    bytes_queued: n.bytes_queued.unwrap_or(0).max(0) as u64,
                    events,
                }
            })
            .collect();
        Self {
            rows,
            fetched_at: std::time::Instant::now(),
            fetched_wall: OffsetDateTime::now_utc(),
        }
    }
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
    pub available_processors: Option<u32>,
    pub uptime: String,
    pub total_threads: u32,
    pub gc: Vec<GcSnapshot>,
    pub content_repos: Vec<RepoUsage>,
    pub flowfile_repo: Option<RepoUsage>,
    pub provenance_repos: Vec<RepoUsage>,
    pub cluster: Option<ClusterMembership>,
}

/// Stateful container for the node-health table.
#[derive(Debug, Default)]
pub struct NodesState {
    pub nodes: Vec<NodeHealthRow>,
    pub selected: usize,
}

impl ListNavigation for NodesState {
    fn list_len(&self) -> usize {
        self.nodes.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.nodes.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }
}

// ---------------------------------------------------------------------------
// Pure extraction / scoring functions
// ---------------------------------------------------------------------------

/// Refresh node-health rows from a fresh system-diagnostics snapshot.
/// Optionally joins cluster-membership data from `/controller/cluster`.
///
/// GC deltas are computed against the previous `state.nodes` values.
/// Nodes that are new (not seen in the previous poll) receive `gc_delta = None`.
/// `state.selected` is clamped to the new row count.
///
/// Sysdiag is the source of truth for row existence. Cluster data is
/// joined on `address`; rows whose address doesn't appear in `cluster`
/// get `cluster = None`. Pass `cluster = None` on standalone NiFi or
/// before the first cluster-nodes fetch completes — the result is
/// identical to the pre-cluster-nodes behavior.
pub fn update_nodes(
    state: &mut NodesState,
    diag: &SystemDiagSnapshot,
    cluster: Option<&ClusterNodesSnapshot>,
) {
    // Capture previous GC totals keyed by node address.
    let old_gc: std::collections::HashMap<&str, u64> = state
        .nodes
        .iter()
        .map(|n| (n.node_address.as_str(), n.gc_collection_count))
        .collect();

    // Pre-index cluster rows by address for O(1) join.
    let cluster_by_addr: std::collections::HashMap<&str, &ClusterNodeRow> = cluster
        .map(|c| c.rows.iter().map(|r| (r.address.as_str(), r)).collect())
        .unwrap_or_default();

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

            let cluster_membership = cluster_by_addr.get(n.address.as_str()).map(|cr| {
                let heartbeat_age = compute_heartbeat_age(
                    cr.heartbeat_iso.as_deref(),
                    cluster.map(|c| c.fetched_wall),
                );
                ClusterMembership {
                    node_id: cr.node_id.clone(),
                    status: cr.status,
                    is_primary: cr.is_primary,
                    is_coordinator: cr.is_coordinator,
                    heartbeat_age,
                    node_start_iso: cr.node_start_iso.clone(),
                    active_thread_count: cr.active_thread_count,
                    flow_files_queued: cr.flow_files_queued,
                    bytes_queued: cr.bytes_queued,
                    events: cr.events.clone(),
                }
            });

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
                available_processors: n.available_processors,
                uptime: n.uptime.clone(),
                total_threads: n.total_threads,
                gc: n.gc.clone(),
                content_repos: n.content_repos.clone(),
                flowfile_repo: n.flowfile_repo.clone(),
                provenance_repos: n.provenance_repos.clone(),
                cluster: cluster_membership,
            }
        })
        .collect();

    // Sort alphabetically by address (case-insensitive) so the Overview
    // Nodes panel presents a stable, predictable order regardless of how
    // NiFi returns the node_snapshots array.
    state.nodes.sort_by(|a, b| {
        a.node_address
            .to_lowercase()
            .cmp(&b.node_address.to_lowercase())
    });

    // Clamp selection to valid range.
    if !state.nodes.is_empty() {
        state.selected = state.selected.min(state.nodes.len() - 1);
    } else {
        state.selected = 0;
    }
}

/// Compute a `std::time::Duration` age between a parsed NiFi heartbeat
/// and the snapshot's wall-clock anchor. Returns `None` when:
/// - The heartbeat string is absent or unparseable, or
/// - The cluster snapshot has no wall-clock anchor (shouldn't happen
///   in practice; defensive), or
/// - The heartbeat is in the future relative to the anchor (server
///   clock skew — we don't try to represent negative ages).
fn compute_heartbeat_age(
    heartbeat_iso: Option<&str>,
    fetched_wall: Option<OffsetDateTime>,
) -> Option<std::time::Duration> {
    let hb = heartbeat_iso.and_then(crate::timestamp::parse_nifi_timestamp)?;
    let anchor = fetched_wall?;
    let delta = anchor - hb;
    if delta.is_negative() {
        return None;
    }
    let whole = delta.whole_seconds();
    // whole is i64; we've already checked non-negative above.
    Some(std::time::Duration::from_secs(whole as u64))
}

// ---------------------------------------------------------------------------
// NifiClient methods
// ---------------------------------------------------------------------------

impl NifiClient {
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
            .systemdiagnostics()
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

        let mut nodes: Vec<NodeDiagnostics> = entity
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
                let content_repos =
                    extract_repo_usages(snap.content_repository_storage_usage.as_deref());
                let flowfile_repo = snap
                    .flow_file_repository_storage_usage
                    .as_ref()
                    .map(map_repo_usage);
                let provenance_repos =
                    extract_repo_usages(snap.provenance_repository_storage_usage.as_deref());

                Some(NodeDiagnostics {
                    address,
                    heap_used_bytes: snap.used_heap_bytes.unwrap_or(0).max(0) as u64,
                    heap_max_bytes: snap.max_heap_bytes.unwrap_or(0).max(0) as u64,
                    gc,
                    load_average: snap.processor_load_average,
                    available_processors: snap.available_processors.map(|v| v.max(0) as u32),
                    total_threads: snap.total_threads.unwrap_or(0).max(0) as u32,
                    uptime: snap.uptime.clone().unwrap_or_default(),
                    content_repos,
                    flowfile_repo,
                    provenance_repos,
                })
            })
            .collect();

        // When nodewise data is absent (e.g. aggregate-only fallback),
        // synthesize a single row from the aggregate snapshot so the
        // Nodes table is never empty.
        if let (true, Some(snap)) = (nodes.is_empty(), agg) {
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
            nodes.push(NodeDiagnostics {
                address: "Cluster (aggregate)".to_string(),
                heap_used_bytes: snap.used_heap_bytes.unwrap_or(0).max(0) as u64,
                heap_max_bytes: snap.max_heap_bytes.unwrap_or(0).max(0) as u64,
                gc,
                load_average: snap.processor_load_average,
                available_processors: snap.available_processors.map(|v| v.max(0) as u32),
                total_threads: snap.total_threads.unwrap_or(0).max(0) as u32,
                uptime: snap.uptime.clone().unwrap_or_default(),
                content_repos: extract_repo_usages(
                    snap.content_repository_storage_usage.as_deref(),
                ),
                flowfile_repo: snap
                    .flow_file_repository_storage_usage
                    .as_ref()
                    .map(map_repo_usage),
                provenance_repos: extract_repo_usages(
                    snap.provenance_repository_storage_usage.as_deref(),
                ),
            });
        }

        Ok(SystemDiagSnapshot {
            aggregate,
            nodes,
            fetched_at: Instant::now(),
        })
    }

    /// Calls `GET /nifi-api/controller/cluster`. On standalone NiFi the
    /// server returns HTTP 409 — the fetcher task (`spawn_cluster_nodes`,
    /// added in a later task) translates that specific shape to an empty
    /// snapshot rather than a failure. This method surfaces *all* errors;
    /// the shape-detection lives one layer up where it belongs.
    pub async fn cluster_nodes(&self) -> Result<ClusterNodesSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name(), "fetching /controller/cluster");
        let dto = self.inner.controller().get_cluster().await.map_err(|err| {
            classify_or_fallback(self.context_name(), Box::new(err), |source| {
                NifiLensError::ClusterNodesFailed {
                    context: self.context_name().to_string(),
                    source,
                }
            })
        })?;
        Ok(ClusterNodesSnapshot::from_cluster_dto(&dto))
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
            available_processors: Some(4),
            total_threads: 50,
            uptime: "1h".to_string(),
            content_repos: Vec::new(),
            flowfile_repo: None,
            provenance_repos: Vec::new(),
        }
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
        update_nodes(&mut state, &snap1, None);
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
        update_nodes(&mut state, &snap2, None);
        assert_eq!(state.nodes[0].gc_delta, Some(5));
        assert_eq!(state.nodes[0].gc_collection_count, 15);
    }

    #[test]
    fn update_nodes_sorts_nodes_alphabetically_case_insensitive() {
        let mut state = NodesState::default();

        // Input order deliberately not alphabetical; mix of upper/lower case
        // distinguishes case-insensitive from the default byte-wise sort
        // (where uppercase ASCII sorts before lowercase).
        let snap = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![
                node_diag("Charlie:8080", 1_000, 8_000, 0),
                node_diag("alpha:8080", 1_000, 8_000, 0),
                node_diag("Bravo:8080", 1_000, 8_000, 0),
            ],
        );
        update_nodes(&mut state, &snap, None);

        let addresses: Vec<&str> = state
            .nodes
            .iter()
            .map(|n| n.node_address.as_str())
            .collect();
        assert_eq!(addresses, vec!["alpha:8080", "Bravo:8080", "Charlie:8080"]);
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
        update_nodes(&mut state, &snap1, None);

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
        update_nodes(&mut state, &snap2, None);

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

    #[test]
    fn update_nodes_copies_gc_collectors_to_row() {
        use std::time::Instant;
        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![],
                flowfile_repo: None,
                provenance_repos: vec![],
            },
            nodes: vec![NodeDiagnostics {
                address: "node1:8080".into(),
                heap_used_bytes: 512 * 1024 * 1024,
                heap_max_bytes: 1024 * 1024 * 1024,
                gc: vec![
                    GcSnapshot {
                        name: "G1 Young".into(),
                        collection_count: 10,
                        collection_millis: 50,
                    },
                    GcSnapshot {
                        name: "G1 Old".into(),
                        collection_count: 2,
                        collection_millis: 120,
                    },
                ],
                load_average: None,
                available_processors: None,
                total_threads: 40,
                uptime: "1h".into(),
                content_repos: vec![RepoUsage {
                    identifier: "c".into(),
                    used_bytes: 60,
                    total_bytes: 100,
                    free_bytes: 40,
                    utilization_percent: 60,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "f".into(),
                    used_bytes: 30,
                    total_bytes: 100,
                    free_bytes: 70,
                    utilization_percent: 30,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "p".into(),
                    used_bytes: 20,
                    total_bytes: 100,
                    free_bytes: 80,
                    utilization_percent: 20,
                }],
            }],
            fetched_at: Instant::now(),
        };
        let mut state = NodesState::default();
        update_nodes(&mut state, &diag, None);

        let row = &state.nodes[0];
        assert_eq!(row.gc.len(), 2);
        assert_eq!(row.gc[0].name, "G1 Young");
        assert_eq!(row.gc[1].collection_millis, 120);
        assert_eq!(row.content_repos.len(), 1);
        assert_eq!(row.content_repos[0].utilization_percent, 60);
        assert_eq!(row.flowfile_repo.as_ref().unwrap().utilization_percent, 30);
        assert_eq!(row.provenance_repos[0].utilization_percent, 20);
    }

    #[test]
    fn update_nodes_aggregate_fallback_produces_single_row() {
        let mut state = NodesState::default();

        // Simulate an aggregate-only response (no per-node data) with a
        // single synthetic "Cluster (aggregate)" node.
        let snap = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![NodeDiagnostics {
                address: "Cluster (aggregate)".to_string(),
                heap_used_bytes: 4_000,
                heap_max_bytes: 8_000,
                gc: vec![GcSnapshot {
                    name: "G1".to_string(),
                    collection_count: 42,
                    collection_millis: 300,
                }],
                load_average: Some(1.5),
                available_processors: Some(8),
                total_threads: 100,
                uptime: "2 days".to_string(),
                content_repos: Vec::new(),
                flowfile_repo: None,
                provenance_repos: Vec::new(),
            }],
        );
        update_nodes(&mut state, &snap, None);

        assert_eq!(state.nodes.len(), 1);
        assert_eq!(state.nodes[0].node_address, "Cluster (aggregate)");
        assert_eq!(state.nodes[0].heap_used_bytes, 4_000);
        assert_eq!(state.nodes[0].heap_max_bytes, 8_000);
        assert_eq!(state.nodes[0].gc_collection_count, 42);
    }

    // -----------------------------------------------------------------------
    // ClusterNodesSnapshot tests
    // -----------------------------------------------------------------------

    #[test]
    fn cluster_nodes_snapshot_from_empty_cluster_is_empty() {
        use nifi_rust_client::dynamic::types::ClusterDto;
        let dto = ClusterDto::default();
        let snap = super::ClusterNodesSnapshot::from_cluster_dto(&dto);
        assert!(snap.rows.is_empty());
    }

    #[test]
    fn cluster_nodes_snapshot_parses_all_fields() {
        use nifi_rust_client::dynamic::types::{ClusterDto, NodeDto, NodeEventDto};
        let mut ev = NodeEventDto::default();
        ev.timestamp = Some("04/22/2026 10:14:03 UTC".into());
        ev.category = Some("CONNECTED".into());
        ev.message = Some("Node connected".into());

        let mut node = NodeDto::default();
        node.node_id = Some("5f2b8a17-1234-1234-1234-c394e97c3000".into());
        node.address = Some("node2.nifi".into());
        node.api_port = Some(8443);
        node.status = Some("CONNECTED".into());
        node.roles = Some(vec!["Primary Node".into(), "Cluster Coordinator".into()]);
        node.heartbeat = Some("04/22/2026 14:03:17 UTC".into());
        node.node_start_time = Some("04/22/2026 09:12:04 UTC".into());
        node.active_thread_count = Some(42);
        node.flow_files_queued = Some(1234);
        node.bytes_queued = Some(456 * 1024 * 1024);
        node.events = Some(vec![ev]);

        let mut dto = ClusterDto::default();
        dto.nodes = Some(vec![node]);

        let snap = super::ClusterNodesSnapshot::from_cluster_dto(&dto);
        assert_eq!(snap.rows.len(), 1);
        let row = &snap.rows[0];
        assert_eq!(row.node_id, "5f2b8a17-1234-1234-1234-c394e97c3000");
        assert_eq!(row.address, "node2.nifi:8443");
        assert_eq!(row.status, super::ClusterNodeStatus::Connected);
        assert!(row.is_primary);
        assert!(row.is_coordinator);
        assert_eq!(
            row.heartbeat_iso.as_deref(),
            Some("04/22/2026 14:03:17 UTC")
        );
        assert_eq!(row.active_thread_count, 42);
        assert_eq!(row.flow_files_queued, 1234);
        assert_eq!(row.bytes_queued, 456 * 1024 * 1024);
        assert_eq!(row.events.len(), 1);
        assert_eq!(row.events[0].category.as_deref(), Some("CONNECTED"));
    }

    #[test]
    fn cluster_nodes_snapshot_handles_unknown_status() {
        use nifi_rust_client::dynamic::types::{ClusterDto, NodeDto};
        let mut node = NodeDto::default();
        node.address = Some("node1".into());
        node.api_port = Some(8443);
        node.status = Some("WEIRD_FUTURE_STATE".into());

        let mut dto = ClusterDto::default();
        dto.nodes = Some(vec![node]);

        let snap = super::ClusterNodesSnapshot::from_cluster_dto(&dto);
        assert_eq!(snap.rows[0].status, super::ClusterNodeStatus::Other);
    }

    #[test]
    fn cluster_nodes_snapshot_caps_events_at_eight() {
        use nifi_rust_client::dynamic::types::{ClusterDto, NodeDto, NodeEventDto};
        let ev = |ts: &str| {
            let mut e = NodeEventDto::default();
            e.timestamp = Some(ts.to_string());
            e.category = Some("HEARTBEAT_RECEIVED".into());
            e.message = Some("x".into());
            e
        };
        let events = (0..20)
            .map(|i| ev(&format!("04/22/2026 10:14:{:02} UTC", i)))
            .collect();

        let mut node = NodeDto::default();
        node.address = Some("node1".into());
        node.api_port = Some(8443);
        node.status = Some("CONNECTED".into());
        node.events = Some(events);

        let mut dto = ClusterDto::default();
        dto.nodes = Some(vec![node]);

        let snap = super::ClusterNodesSnapshot::from_cluster_dto(&dto);
        assert_eq!(snap.rows[0].events.len(), 8);
        assert_eq!(
            snap.rows[0].events[0].timestamp_iso,
            "04/22/2026 10:14:00 UTC"
        );
    }

    #[test]
    fn cluster_nodes_snapshot_address_defaults_when_missing() {
        use nifi_rust_client::dynamic::types::{ClusterDto, NodeDto};
        let mut node = NodeDto::default();
        node.status = Some("CONNECTED".into());

        let mut dto = ClusterDto::default();
        dto.nodes = Some(vec![node]);

        let snap = super::ClusterNodesSnapshot::from_cluster_dto(&dto);
        assert_eq!(snap.rows[0].address, "unknown:0");
    }

    // -----------------------------------------------------------------------
    // ClusterNodeStatus tests
    // -----------------------------------------------------------------------

    #[test]
    fn cluster_node_status_from_wire_recognizes_known() {
        use super::ClusterNodeStatus as S;
        assert_eq!(S::from_wire("CONNECTED"), S::Connected);
        assert_eq!(S::from_wire("CONNECTING"), S::Connecting);
        assert_eq!(S::from_wire("DISCONNECTED"), S::Disconnected);
        assert_eq!(S::from_wire("DISCONNECTING"), S::Disconnecting);
        assert_eq!(S::from_wire("OFFLOADING"), S::Offloading);
        assert_eq!(S::from_wire("OFFLOADED"), S::Offloaded);
    }

    #[test]
    fn cluster_node_status_from_wire_unknown_is_other() {
        use super::ClusterNodeStatus as S;
        assert_eq!(S::from_wire("WEIRD_FUTURE_STATE"), S::Other);
        assert_eq!(S::from_wire(""), S::Other);
    }

    #[test]
    fn cluster_node_status_is_dead_matrix() {
        use super::ClusterNodeStatus as S;
        assert!(!S::Connected.is_dead());
        assert!(!S::Connecting.is_dead());
        assert!(!S::Other.is_dead());
        assert!(S::Disconnected.is_dead());
        assert!(S::Disconnecting.is_dead());
        assert!(S::Offloading.is_dead());
        assert!(S::Offloaded.is_dead());
    }

    #[test]
    fn node_health_row_defaults_cluster_to_none() {
        use super::{NodeHealthRow, Severity};
        let row = NodeHealthRow {
            node_address: "node1:8443".into(),
            heap_used_bytes: 0,
            heap_max_bytes: 0,
            heap_percent: 0,
            heap_severity: Severity::Green,
            gc_collection_count: 0,
            gc_delta: None,
            gc_millis: 0,
            load_average: None,
            available_processors: None,
            uptime: String::new(),
            total_threads: 0,
            gc: vec![],
            content_repos: vec![],
            flowfile_repo: None,
            provenance_repos: vec![],
            cluster: None,
        };
        assert!(row.cluster.is_none());
    }

    // -----------------------------------------------------------------------
    // update_nodes cluster-join tests
    // -----------------------------------------------------------------------

    #[test]
    fn update_nodes_without_cluster_snapshot_matches_today() {
        let mut state = NodesState::default();
        let snap = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8080", 1_000, 8_000, 10)],
        );
        super::update_nodes(&mut state, &snap, None);
        assert_eq!(state.nodes.len(), 1);
        assert!(state.nodes[0].cluster.is_none());
    }

    #[test]
    fn update_nodes_joins_by_address_port() {
        use super::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        let mut state = NodesState::default();
        let sysdiag = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8443", 1_000, 8_000, 10)],
        );
        let cluster = ClusterNodesSnapshot {
            rows: vec![ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8443".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: true,
                is_coordinator: false,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 7,
                flow_files_queued: 123,
                bytes_queued: 456,
                events: vec![],
            }],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        super::update_nodes(&mut state, &sysdiag, Some(&cluster));
        let row = &state.nodes[0];
        let m = row.cluster.as_ref().expect("cluster joined");
        assert_eq!(m.node_id, "id-1");
        assert!(m.is_primary);
        assert_eq!(m.active_thread_count, 7);
        assert_eq!(m.flow_files_queued, 123);
    }

    #[test]
    fn update_nodes_leaves_cluster_none_when_address_absent_from_cluster() {
        use super::ClusterNodesSnapshot;
        let mut state = NodesState::default();
        let sysdiag = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("ghost:8443", 1_000, 8_000, 10)],
        );
        let cluster = ClusterNodesSnapshot {
            rows: vec![],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        super::update_nodes(&mut state, &sysdiag, Some(&cluster));
        assert!(state.nodes[0].cluster.is_none());
    }

    #[test]
    fn update_nodes_heartbeat_age_is_deterministic_from_fetched_wall() {
        use super::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        let mut state = NodesState::default();
        let sysdiag = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8443", 1_000, 8_000, 10)],
        );
        // Anchor: snapshot was fetched exactly 5 seconds after the heartbeat.
        let hb = "04/22/2026 14:03:17 UTC";
        let hb_parsed = crate::timestamp::parse_nifi_timestamp(hb).expect("parseable");
        let fetched_wall = hb_parsed + time::Duration::seconds(5);
        let cluster = ClusterNodesSnapshot {
            rows: vec![ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8443".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_iso: Some(hb.into()),
                node_start_iso: None,
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            }],
            fetched_at: std::time::Instant::now(),
            fetched_wall,
        };
        super::update_nodes(&mut state, &sysdiag, Some(&cluster));
        let m = state.nodes[0].cluster.as_ref().unwrap();
        assert_eq!(m.heartbeat_age, Some(std::time::Duration::from_secs(5)));
    }

    #[test]
    fn update_nodes_unparseable_heartbeat_yields_none_age() {
        use super::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        let mut state = NodesState::default();
        let sysdiag = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8443", 1_000, 8_000, 10)],
        );
        let cluster = ClusterNodesSnapshot {
            rows: vec![ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8443".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_iso: Some("nonsense".into()),
                node_start_iso: None,
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            }],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        super::update_nodes(&mut state, &sysdiag, Some(&cluster));
        let m = state.nodes[0].cluster.as_ref().unwrap();
        assert!(m.heartbeat_age.is_none());
    }

    #[test]
    fn update_nodes_heartbeat_in_the_future_clamps_to_none() {
        // Defensive: if the server clock is ahead of ours, heartbeat
        // subtraction would be negative — represent that as None, not
        // a wrap-around Duration.
        use super::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        let mut state = NodesState::default();
        let sysdiag = diag_snap(
            Vec::new(),
            None,
            Vec::new(),
            vec![node_diag("node1:8443", 1_000, 8_000, 10)],
        );
        let hb = "04/22/2026 14:03:17 UTC";
        let hb_parsed = crate::timestamp::parse_nifi_timestamp(hb).unwrap();
        // fetched_wall is 10s BEFORE heartbeat — future-from-our-POV.
        let fetched_wall = hb_parsed - time::Duration::seconds(10);
        let cluster = ClusterNodesSnapshot {
            rows: vec![ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8443".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_iso: Some(hb.into()),
                node_start_iso: None,
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            }],
            fetched_at: std::time::Instant::now(),
            fetched_wall,
        };
        super::update_nodes(&mut state, &sysdiag, Some(&cluster));
        assert!(
            state.nodes[0]
                .cluster
                .as_ref()
                .unwrap()
                .heartbeat_age
                .is_none()
        );
    }
}
