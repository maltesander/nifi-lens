//! Browser-tab client wrappers. Phase 3 adds the tree fetch and four
//! per-node detail fetches used by the Browser tab's hybrid data flow.

use std::time::SystemTime;

use nifi_rust_client::dynamic::traits::{
    FlowApi as _, FlowControllerServicesApi as _, FlowStatusApi as _, ProcessGroupsApi as _,
};

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

/// Type-specific status fields carried alongside each arena node. Mirrors the
/// different DTO shapes returned by the NiFi API per component kind. The
/// `Port` variant covers input and output ports, which carry no additional
/// per-kind status fields beyond their `NodeKind` tag.
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
    Port,
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
        // `ProcessGroupStatusSnapshotDto` does not expose a `groupId` / `group_id`
        // field (unlike processor/connection/port DTOs). The PG's own parent
        // context comes from its position in the arena via `parent_idx`.
        group_id: String::new(),
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
                status_summary: NodeStatusSummary::Port,
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
                status_summary: NodeStatusSummary::Port,
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

/// Identity + aggregate counts for a single Process Group, combined from
/// two API calls: `GET /process-groups/{id}` and
/// `GET /flow/process-groups/{id}/controller-services`.
#[derive(Debug, Clone)]
pub struct ProcessGroupDetail {
    pub id: String,
    pub name: String,
    pub parent_group_id: Option<String>,
    pub running: u32,
    pub stopped: u32,
    pub invalid: u32,
    pub disabled: u32,
    pub active_threads: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub queued_display: String,
    pub controller_services: Vec<ControllerServiceSummary>,
}

/// Minimal controller-service summary used in the Browser detail pane.
#[derive(Debug, Clone)]
pub struct ControllerServiceSummary {
    pub id: String,
    pub name: String,
    pub type_short: String,
    pub state: String,
}

impl NifiClient {
    /// Two-endpoint fetch: the process group's `component` for identity
    /// and aggregate counts, then the PG's controller services list.
    /// Maps each call to its own typed error variant so callers can
    /// tell which endpoint failed.
    pub async fn browser_pg_detail(
        &self,
        pg_id: &str,
    ) -> Result<ProcessGroupDetail, NifiLensError> {
        tracing::debug!(context = %self.context_name(), %pg_id, "fetching PG detail");

        // 1) Base PG entity — identity + component counts.
        let pg_entity = self
            .inner
            .processgroups_api()
            .get_process_group(pg_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProcessGroupDetailFailed {
                        context: self.context_name().to_string(),
                        id: pg_id.to_string(),
                        source,
                    }
                })
            })?;
        let component = pg_entity.component.unwrap_or_default();
        let status_agg = pg_entity
            .status
            .as_ref()
            .and_then(|s| s.aggregate_snapshot.as_ref());

        // 2) CS list for this PG.
        let cs_entity = self
            .inner
            .flow_api()
            .controller_services(pg_id)
            .get_controller_services_from_group(None, None, None, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ControllerServicesListFailed {
                        context: self.context_name().to_string(),
                        id: pg_id.to_string(),
                        source,
                    }
                })
            })?;

        let controller_services = cs_entity
            .controller_services
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let c = e.component?;
                Some(ControllerServiceSummary {
                    id: c.id.clone().unwrap_or_default(),
                    name: c.name.clone().unwrap_or_default(),
                    // NOTE: the library uses `r#type` (raw identifier) for
                    // the `type` JSON field; the plan called this `type_field`
                    // but the actual Rust field is `r#type`.
                    type_short: short_type(c.r#type.as_deref().unwrap_or("")),
                    state: c.state.clone().unwrap_or_default(),
                })
            })
            .collect();

        Ok(ProcessGroupDetail {
            id: pg_id.to_string(),
            name: component.name.unwrap_or_default(),
            parent_group_id: component.parent_group_id,
            running: component.running_count.unwrap_or(0).max(0) as u32,
            stopped: component.stopped_count.unwrap_or(0).max(0) as u32,
            invalid: component.invalid_count.unwrap_or(0).max(0) as u32,
            disabled: component.disabled_count.unwrap_or(0).max(0) as u32,
            active_threads: status_agg
                .and_then(|a| a.active_thread_count)
                .unwrap_or(0)
                .max(0) as u32,
            flow_files_queued: status_agg
                .and_then(|a| a.flow_files_queued)
                .unwrap_or(0)
                .max(0) as u32,
            bytes_queued: status_agg.and_then(|a| a.bytes_queued).unwrap_or(0).max(0) as u64,
            queued_display: status_agg
                .and_then(|a| a.queued.clone())
                .unwrap_or_default(),
            controller_services,
        })
    }
}

/// Extract the short class name from a fully qualified Java type, e.g.
/// `org.apache.nifi.kafka.service.Kafka3ConnectionService` →
/// `Kafka3ConnectionService`. Passed-through unchanged when there is no
/// dot.
pub(crate) fn short_type(fqn: &str) -> String {
    fqn.rsplit('.').next().unwrap_or(fqn).to_string()
}

/// Flat snapshot of a single processor's identity, scheduling configuration,
/// properties, and validation errors. Consumed directly by the Browser detail
/// render pane.
#[derive(Debug, Clone)]
pub struct ProcessorDetail {
    /// The processor's stable UUID.
    pub id: String,
    /// Human-readable display name configured by the user.
    pub name: String,
    /// Fully-qualified Java class name of the processor implementation.
    pub type_name: String,
    /// NAR bundle coordinates formatted as `group:artifact:version`.
    pub bundle: String,
    /// Scheduler run-state string (e.g. `"RUNNING"`, `"STOPPED"`).
    pub run_status: String,
    /// Scheduling strategy (e.g. `"TIMER_DRIVEN"`, `"EVENT_DRIVEN"`).
    pub scheduling_strategy: String,
    /// Scheduling period string (e.g. `"1 sec"`, `"0 sec"`).
    pub scheduling_period: String,
    /// Number of concurrently schedulable tasks.
    pub concurrent_tasks: u32,
    /// Run duration in milliseconds (batch mode hint).
    pub run_duration_ms: u64,
    /// FlowFile penalty duration string (e.g. `"30 sec"`).
    pub penalty_duration: String,
    /// Yield duration string (e.g. `"1 sec"`).
    pub yield_duration: String,
    /// Minimum severity level for bulletins (e.g. `"WARN"`, `"INFO"`).
    pub bulletin_level: String,
    /// Processor properties as ordered key-value pairs. HashMap iteration
    /// order is non-deterministic; stable display ordering is Phase 5 polish.
    pub properties: Vec<(String, String)>,
    /// Validation error messages that must be resolved before the processor
    /// can be started.
    pub validation_errors: Vec<String>,
}

impl NifiClient {
    /// Single-endpoint detail fetch for a processor node. Returns the
    /// full identity, scheduling config, properties, and validation
    /// errors as a flat snapshot the render pane can consume directly.
    pub async fn browser_processor_detail(
        &self,
        proc_id: &str,
    ) -> Result<ProcessorDetail, NifiLensError> {
        use nifi_rust_client::dynamic::traits::ProcessorsApi as _;

        tracing::debug!(context = %self.context_name(), %proc_id, "fetching processor detail");
        let entity = self
            .processors_api()
            .get_processor(proc_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProcessorDetailFailed {
                        context: self.context_name().to_string(),
                        id: proc_id.to_string(),
                        source,
                    }
                })
            })?;

        let component = entity.component.unwrap_or_default();
        let config = component.config.clone().unwrap_or_default();
        let bundle_str = component
            .bundle
            .as_ref()
            .map(|b| {
                format!(
                    "{}:{}:{}",
                    b.group.clone().unwrap_or_default(),
                    b.artifact.clone().unwrap_or_default(),
                    b.version.clone().unwrap_or_default(),
                )
            })
            .unwrap_or_default();

        // Library returns `Option<HashMap<String, Option<String>>>`. Flatten
        // into `Vec<(String, String)>`. HashMap ordering is non-deterministic;
        // the UI displays whatever order iteration returns. Stable property
        // ordering is a Phase 5 polish item.
        let properties: Vec<(String, String)> = config
            .properties
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.unwrap_or_default()))
            .collect();

        let validation_errors = component.validation_errors.unwrap_or_default();

        Ok(ProcessorDetail {
            id: proc_id.to_string(),
            name: component.name.unwrap_or_default(),
            type_name: component.r#type.unwrap_or_default(),
            bundle: bundle_str,
            run_status: component.state.unwrap_or_default(),
            scheduling_strategy: config.scheduling_strategy.unwrap_or_default(),
            scheduling_period: config.scheduling_period.unwrap_or_default(),
            concurrent_tasks: config
                .concurrently_schedulable_task_count
                .unwrap_or(0)
                .max(0) as u32,
            run_duration_ms: config.run_duration_millis.unwrap_or(0).max(0) as u64,
            penalty_duration: config.penalty_duration.unwrap_or_default(),
            yield_duration: config.yield_duration.unwrap_or_default(),
            bulletin_level: config.bulletin_level.unwrap_or_default(),
            properties,
            validation_errors,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::short_type;

    #[test]
    fn short_type_strips_package_prefix() {
        assert_eq!(
            short_type("org.apache.nifi.kafka.service.Kafka3ConnectionService"),
            "Kafka3ConnectionService"
        );
    }

    #[test]
    fn short_type_passthrough_when_no_dot() {
        assert_eq!(short_type("MyProcessor"), "MyProcessor");
    }

    #[test]
    fn short_type_empty_string() {
        assert_eq!(short_type(""), "");
    }
}
