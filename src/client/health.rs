//! Health-tab client wrappers and snapshot types.

use std::time::Instant;

use nifi_rust_client::dynamic::types::common::StorageUsageDto;

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
                available_processors: n.available_processors,
                uptime: n.uptime.clone(),
                total_threads: n.total_threads,
                gc: n.gc.clone(),
                content_repos: n.content_repos.clone(),
                flowfile_repo: n.flowfile_repo.clone(),
                provenance_repos: n.provenance_repos.clone(),
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
        update_nodes(&mut state, &diag);

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
        update_nodes(&mut state, &snap);

        assert_eq!(state.nodes.len(), 1);
        assert_eq!(state.nodes[0].node_address, "Cluster (aggregate)");
        assert_eq!(state.nodes[0].heap_used_bytes, 4_000);
        assert_eq!(state.nodes[0].heap_max_bytes, 8_000);
        assert_eq!(state.nodes[0].gc_collection_count, 42);
    }
}
