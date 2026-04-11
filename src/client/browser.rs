//! Browser-tab client wrappers. Phase 3 adds the tree fetch and four
//! per-node detail fetches used by the Browser tab's hybrid data flow.

use std::time::SystemTime;

use nifi_rust_client::dynamic::traits::{FlowApi as _, FlowStatusApi as _};

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

/// Flat, arena-ready shape produced by walking the recursive
/// `ProcessGroupStatusDTO`. Consumers index into `nodes` by `usize`;
/// parents are referenced via `RawNode::parent_idx`.
#[derive(Debug, Clone)]
pub struct RecursiveSnapshot {
    pub nodes: Vec<RawNode>,
    pub fetched_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct RawNode {
    pub parent_idx: Option<usize>,
    pub kind: NodeKind,
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub status_summary: NodeStatusSummary,
}

/// Kind tag for every arena entry. Mirrors the same-named enum in
/// `view::browser::state`; lives here so the client boundary can build
/// the arena without pulling in the TUI crate module.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NodeKind {
    ProcessGroup,
    Processor,
    Connection,
    InputPort,
    OutputPort,
    ControllerService,
}

#[derive(Debug, Clone)]
pub enum NodeStatusSummary {
    ProcessGroup {
        running: u32,
        stopped: u32,
        invalid: u32,
        disabled: u32,
    },
    Processor {
        run_status: String,
    },
    Connection {
        fill_percent: u32,
        flow_files_queued: u32,
        queued_display: String,
    },
    ControllerService {
        state: String,
    },
    None,
}

impl NifiClient {
    /// Recursive `process-groups/root/status` fetch flattened into an
    /// arena. Matches the Overview tab's existing root-status call but
    /// emits a richer shape with every processor / connection / port
    /// individually represented.
    pub async fn browser_tree(&self) -> Result<RecursiveSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            "fetching /flow/process-groups/root/status (browser tree)"
        );
        let entity = self
            .inner
            .flow_api()
            .status("root")
            .get_process_group_status(Some(true), None, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::BrowserTreeFailed {
                        context: self.context_name().to_string(),
                        source,
                    }
                })
            })?;

        let mut nodes: Vec<RawNode> = Vec::new();
        if let Some(pg_dto) = entity.process_group_status
            && let Some(agg) = pg_dto.aggregate_snapshot
        {
            walk_pg_snapshot(&agg, None, &mut nodes);
        }

        Ok(RecursiveSnapshot {
            nodes,
            fetched_at: SystemTime::now(),
        })
    }
}

/// Recursive walker. Appends the current PG, then all processors, all
/// connections, all input ports, all output ports, and finally recurses
/// into child PGs. Child-node parent indices are the arena index of the
/// PG row that was just pushed.
fn walk_pg_snapshot(
    snap: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    parent_idx: Option<usize>,
    out: &mut Vec<RawNode>,
) {
    let pg_idx = out.len();
    out.push(RawNode {
        parent_idx,
        kind: NodeKind::ProcessGroup,
        id: snap.id.clone().unwrap_or_default(),
        group_id: snap.id.clone().unwrap_or_default(),
        name: snap.name.clone().unwrap_or_default(),
        // ProcessGroupStatusSnapshotDto does not carry running/stopped/invalid/
        // disabled counts; those live on ControllerStatusDto (whole-cluster).
        // Child-PG counts come from the nested entity, not the snapshot DTO.
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
    });

    if let Some(procs) = snap.processor_status_snapshots.as_ref() {
        for entity in procs {
            let Some(p) = entity.processor_status_snapshot.as_ref() else {
                continue;
            };
            out.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::Processor,
                id: p.id.clone().unwrap_or_default(),
                group_id: p.group_id.clone().unwrap_or_default(),
                name: p.name.clone().unwrap_or_default(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: p.run_status.clone().unwrap_or_default(),
                },
            });
        }
    }

    if let Some(conns) = snap.connection_status_snapshots.as_ref() {
        for entity in conns {
            let Some(c) = entity.connection_status_snapshot.as_ref() else {
                continue;
            };
            let by_count = c.percent_use_count.unwrap_or(0).max(0) as u32;
            let by_bytes = c.percent_use_bytes.unwrap_or(0).max(0) as u32;
            out.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::Connection,
                id: c.id.clone().unwrap_or_default(),
                group_id: c.group_id.clone().unwrap_or_default(),
                name: c.name.clone().unwrap_or_default(),
                status_summary: NodeStatusSummary::Connection {
                    fill_percent: by_count.max(by_bytes),
                    flow_files_queued: c.flow_files_queued.unwrap_or(0).max(0) as u32,
                    queued_display: c.queued.clone().unwrap_or_default(),
                },
            });
        }
    }

    if let Some(ports) = snap.input_port_status_snapshots.as_ref() {
        for entity in ports {
            let Some(p) = entity.port_status_snapshot.as_ref() else {
                continue;
            };
            out.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::InputPort,
                id: p.id.clone().unwrap_or_default(),
                group_id: p.group_id.clone().unwrap_or_default(),
                name: p.name.clone().unwrap_or_default(),
                status_summary: NodeStatusSummary::None,
            });
        }
    }

    if let Some(ports) = snap.output_port_status_snapshots.as_ref() {
        for entity in ports {
            let Some(p) = entity.port_status_snapshot.as_ref() else {
                continue;
            };
            out.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::OutputPort,
                id: p.id.clone().unwrap_or_default(),
                group_id: p.group_id.clone().unwrap_or_default(),
                name: p.name.clone().unwrap_or_default(),
                status_summary: NodeStatusSummary::None,
            });
        }
    }

    if let Some(children) = snap.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                walk_pg_snapshot(child, Some(pg_idx), out);
            }
        }
    }
}
