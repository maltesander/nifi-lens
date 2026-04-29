//! Browser-tab client wrappers: per-node detail fetches used by the
//! Browser tab's hybrid data flow. The arena itself is rebuilt from
//! the shared cluster snapshot rather than fetched here.

use std::time::SystemTime;

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

/// Flat, arena-ready container produced by walking the recursive
/// `ProcessGroupStatusDTO`. Lives on alongside `RootPgStatusSnapshot.nodes`
/// as a compact test fixture shape — view-reducer tests that need to
/// feed the arena builder construct a `RecursiveSnapshot` and call
/// `apply_tree_snapshot` directly rather than assembling a full
/// `ClusterSnapshot`.
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

/// Synthetic folder kinds emitted only by the reducer to bucket
/// queue and CS leaves underneath their owning PG. Never produced by
/// the client walker.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FolderKind {
    Queues,
    ControllerServices,
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
    RemoteProcessGroup,
    Folder(FolderKind),
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
        source_id: String,
        source_name: String,
        destination_id: String,
        destination_name: String,
    },
    ControllerService {
        state: String,
    },
    Port,
    RemoteProcessGroup {
        /// Wire value from `RemoteProcessGroupStatusSnapshotDto::transmission_status`.
        /// Either "Transmitting" or "Not Transmitting" — verbatim, the
        /// renderer matches on these strings.
        transmission_status: String,
        active_threads: u32,
        flow_files_received: u32,
        flow_files_sent: u32,
        bytes_received: u64,
        bytes_sent: u64,
        /// Singular `target_uri` from the recursive snapshot. The
        /// Identity-pane upgrades to `target_uris` (plural) when the
        /// on-demand detail fetch returns it.
        target_uri: String,
    },
    Folder {
        count: u32,
    },
}

/// Endpoint IDs for every connection in a single process group, keyed
/// by connection id. Produced by the `ClusterStore` connections-by-PG
/// fetcher (one snapshot entry per PG) and merged into the browser
/// arena by the view reducer in Task 6.
///
/// The nested `ConnectionEndpointIds` carries one connection's
/// `(source_id, destination_id)` pair.
#[derive(Debug, Clone, Default)]
pub struct ConnectionEndpoints {
    pub by_connection: std::collections::HashMap<String, ConnectionEndpointIds>,
}

/// Source / destination id pair for a single connection. Either side
/// may be empty when the upstream DTO omitted the id (very rare —
/// `/process-groups/{id}/connections` always populates both).
#[derive(Debug, Clone, Default)]
pub struct ConnectionEndpointIds {
    pub source_id: String,
    pub destination_id: String,
}

/// Recursive walker that appends PG / processor / connection / input port
/// / output port rows for one PG to `out`, then recurses into child PGs.
/// Controller-service rows are NOT appended here — the Browser reducer
/// attaches those from the separate `ControllerServicesSnapshot.members`
/// list because CS identity/state lives in a different API call.
///
/// Child-node parent indices are the arena index of the PG row that was
/// just pushed. Called by `RootPgStatusSnapshot::from_aggregate` so the
/// recursive-status fetch populates the browser arena without an extra
/// call.
pub fn walk_pg_nodes(
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
                    source_id: c.source_id.clone().unwrap_or_default(),
                    source_name: c.source_name.clone().unwrap_or_default(),
                    destination_id: c.destination_id.clone().unwrap_or_default(),
                    destination_name: c.destination_name.clone().unwrap_or_default(),
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

    if let Some(rpgs) = snap.remote_process_group_status_snapshots.as_ref() {
        for entity in rpgs {
            let Some(r) = entity.remote_process_group_status_snapshot.as_ref() else {
                continue;
            };
            out.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::RemoteProcessGroup,
                id: r.id.clone().unwrap_or_default(),
                group_id: r.group_id.clone().unwrap_or_default(),
                name: r.name.clone().unwrap_or_default(),
                status_summary: NodeStatusSummary::RemoteProcessGroup {
                    transmission_status: r.transmission_status.clone().unwrap_or_default(),
                    active_threads: r.active_thread_count.unwrap_or(0).max(0) as u32,
                    flow_files_received: r.flow_files_received.unwrap_or(0).max(0) as u32,
                    flow_files_sent: r.flow_files_sent.unwrap_or(0).max(0) as u32,
                    bytes_received: r.bytes_received.unwrap_or(0).max(0) as u64,
                    bytes_sent: r.bytes_sent.unwrap_or(0).max(0) as u64,
                    target_uri: r.target_uri.clone().unwrap_or_default(),
                },
            });
        }
    }

    if let Some(children) = snap.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                walk_pg_nodes(child, Some(pg_idx), out);
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
            .processgroups()
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
            .flow()
            .get_controller_services_from_group(pg_id, None, None, None, None)
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
    /// order is non-deterministic; stable display ordering is deferred polish.
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
        tracing::debug!(context = %self.context_name(), %proc_id, "fetching processor detail");
        let entity = self
            .processors()
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
        // Build bundle_str before moving config out so component.config can be moved.
        let bundle_str = component
            .bundle
            .as_ref()
            .map(|b| {
                format!(
                    "{}:{}:{}",
                    b.group.as_deref().unwrap_or(""),
                    b.artifact.as_deref().unwrap_or(""),
                    b.version.as_deref().unwrap_or(""),
                )
            })
            .unwrap_or_default();
        let config = component.config.unwrap_or_default();

        let mut properties: Vec<(String, String)> = config
            .properties
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.unwrap_or_default()))
            .collect();
        properties.sort_by(|a, b| a.0.cmp(&b.0));

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

/// Detail snapshot for a Connection node in the Browser tab's detail
/// pane. Flattens the library's `ConnectionEntity` into the fields the
/// renderer actually uses.
#[derive(Debug, Clone)]
pub struct ConnectionDetail {
    /// The connection's stable UUID.
    pub id: String,
    /// Human-readable display name configured by the user.
    pub name: String,
    /// UUID of the source connectable component.
    pub source_id: String,
    /// Display name of the source connectable component.
    pub source_name: String,
    /// Component kind of the source (e.g. `"PROCESSOR"`).
    pub source_type: String,
    /// Process-group UUID that contains the source component.
    pub source_group_id: String,
    /// UUID of the destination connectable component.
    pub destination_id: String,
    /// Display name of the destination connectable component.
    pub destination_name: String,
    /// Component kind of the destination (e.g. `"PROCESSOR"`).
    pub destination_type: String,
    /// Process-group UUID that contains the destination component.
    pub destination_group_id: String,
    /// Relationships from the source that are routed into this connection.
    pub selected_relationships: Vec<String>,
    /// All relationships the source currently exposes.
    pub available_relationships: Vec<String>,
    /// Object-count threshold at which back-pressure is applied.
    pub back_pressure_object_threshold: u64,
    /// Data-size threshold string at which back-pressure is applied (e.g. `"1 GB"`).
    pub back_pressure_data_size_threshold: String,
    /// Maximum age a FlowFile may sit in the queue before it is expired (e.g. `"0 sec"`).
    pub flow_file_expiration: String,
    /// Load-balance strategy string (e.g. `"DO_NOT_LOAD_BALANCE"`).
    pub load_balance_strategy: String,
    /// Queue fill percentage: `max(percent_use_count, percent_use_bytes)`.
    pub fill_percent: u32,
    /// Number of FlowFiles currently queued.
    pub flow_files_queued: u32,
    /// Total bytes currently queued.
    pub bytes_queued: u64,
    /// Human-readable queue size string (e.g. `"5,500 / 50 MB"`).
    pub queued_display: String,
}

impl NifiClient {
    /// Single-endpoint detail fetch for a connection. Returns the
    /// source/destination identities, selected and available
    /// relationships, back-pressure thresholds, expiration settings,
    /// and the live fill percentage (same `max(percent_use_count,
    /// percent_use_bytes)` derivation as `QueueSnapshot`).
    pub async fn browser_connection_detail(
        &self,
        conn_id: &str,
    ) -> Result<ConnectionDetail, NifiLensError> {
        tracing::debug!(context = %self.context_name(), %conn_id, "fetching connection detail");
        let entity = self
            .connections()
            .get_connection(conn_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ConnectionDetailFailed {
                        context: self.context_name().to_string(),
                        id: conn_id.to_string(),
                        source,
                    }
                })
            })?;

        let component = entity.component.unwrap_or_default();
        let agg = entity
            .status
            .as_ref()
            .and_then(|s| s.aggregate_snapshot.as_ref());

        // Read queue stats before moving fields out of component.
        let by_count = agg.and_then(|a| a.percent_use_count).unwrap_or(0).max(0) as u32;
        let by_bytes = agg.and_then(|a| a.percent_use_bytes).unwrap_or(0).max(0) as u32;
        let flow_files_queued = agg.and_then(|a| a.flow_files_queued).unwrap_or(0).max(0) as u32;
        let bytes_queued = agg.and_then(|a| a.bytes_queued).unwrap_or(0).max(0) as u64;
        let queued_display = agg
            .and_then(|a| a.queued.as_deref())
            .unwrap_or("")
            .to_owned();

        // Move source/destination out of component before consuming other fields.
        let source = component.source.unwrap_or_default();
        let dest = component.destination.unwrap_or_default();

        Ok(ConnectionDetail {
            id: conn_id.to_string(),
            name: component.name.unwrap_or_default(),
            source_id: source.id,
            source_name: source.name.unwrap_or_default(),
            source_type: source.r#type,
            source_group_id: source.group_id,
            destination_id: dest.id,
            destination_name: dest.name.unwrap_or_default(),
            destination_type: dest.r#type,
            destination_group_id: dest.group_id,
            selected_relationships: component.selected_relationships.unwrap_or_default(),
            available_relationships: component.available_relationships.unwrap_or_default(),
            back_pressure_object_threshold: component
                .back_pressure_object_threshold
                .unwrap_or(0)
                .max(0) as u64,
            back_pressure_data_size_threshold: component
                .back_pressure_data_size_threshold
                .unwrap_or_default(),
            flow_file_expiration: component.flow_file_expiration.unwrap_or_default(),
            load_balance_strategy: component.load_balance_strategy.unwrap_or_default(),
            fill_percent: by_count.max(by_bytes),
            flow_files_queued,
            bytes_queued,
            queued_display,
        })
    }
}

/// Detail snapshot for a Controller Service node in the Browser tab's
/// detail pane.
#[derive(Debug, Clone)]
pub struct ControllerServiceDetail {
    pub id: String,
    pub name: String,
    pub type_name: String,
    pub bundle: String,
    pub state: String,
    pub parent_group_id: Option<String>,
    pub properties: Vec<(String, String)>,
    pub validation_errors: Vec<String>,
    pub bulletin_level: String,
    pub comments: String,
    pub restricted: bool,
    pub deprecated: bool,
    pub persists_state: bool,
    pub referencing_components: Vec<ReferencingComponent>,
}

/// Kind of component that references a controller service. Raw NiFi
/// `referenceType` strings (which vary in case across versions) are
/// normalized into this enum; unknown values are preserved via
/// [`ReferencingKind::Other`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReferencingKind {
    Processor,
    ControllerService,
    ReportingTask,
    FlowRegistryClient,
    ParameterProvider,
    Other(String),
}

impl ReferencingKind {
    /// Normalize a raw `referenceType` string from the NiFi API.
    pub fn from_api(raw: &str) -> Self {
        match raw {
            "Processor" | "PROCESSOR" => Self::Processor,
            "ControllerService" | "CONTROLLER_SERVICE" => Self::ControllerService,
            "ReportingTask" | "REPORTING_TASK" => Self::ReportingTask,
            "FlowRegistryClient" | "FLOW_REGISTRY_CLIENT" => Self::FlowRegistryClient,
            "ParameterProvider" | "PARAMETER_PROVIDER" => Self::ParameterProvider,
            other => Self::Other(other.to_string()),
        }
    }
}

/// One component that references a controller service, as returned by
/// `GET /controller-services/{id}?includeReferencingComponents=true`.
#[derive(Debug, Clone)]
pub struct ReferencingComponent {
    pub id: String,
    pub name: String,
    pub kind: ReferencingKind,
    pub state: String,
    pub active_thread_count: u32,
    pub group_id: String,
}

impl NifiClient {
    /// Single-endpoint detail fetch for a controller service. Returns
    /// identity, state, parent scope, properties, validation errors,
    /// bulletin level, comments, restricted/deprecated/persists-state
    /// flags, and the list of referencing components (processors and
    /// other controller services that depend on this service).
    pub async fn browser_cs_detail(
        &self,
        cs_id: &str,
    ) -> Result<ControllerServiceDetail, NifiLensError> {
        tracing::debug!(context = %self.context_name(), %cs_id, "fetching CS detail");
        // NB: the generated `get_controller_service(id, ui_only)` has no
        // explicit `includeReferencingComponents` parameter — NiFi's
        // `GET /controller-services/{id}` returns `referencingComponents`
        // in the body by default. Passing `ui_only=true` would strip the
        // response, so we pass `None`.
        let entity = self
            .controller_services()
            .get_controller_service(cs_id, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ControllerServiceDetailFailed {
                        context: self.context_name().to_string(),
                        id: cs_id.to_string(),
                        source,
                    }
                })
            })?;

        let component = entity.component.unwrap_or_default();
        // Build bundle_str before moving out of component.
        let bundle_str = component
            .bundle
            .as_ref()
            .map(|b| {
                format!(
                    "{}:{}:{}",
                    b.group.as_deref().unwrap_or(""),
                    b.artifact.as_deref().unwrap_or(""),
                    b.version.as_deref().unwrap_or(""),
                )
            })
            .unwrap_or_default();

        let mut properties: Vec<(String, String)> = component
            .properties
            .unwrap_or_default()
            .into_iter()
            .map(|(k, v)| (k, v.unwrap_or_default()))
            .collect();
        properties.sort_by(|a, b| a.0.cmp(&b.0));

        let referencing_components: Vec<ReferencingComponent> = component
            .referencing_components
            .unwrap_or_default()
            .into_iter()
            .filter_map(|e| {
                let c = e.component?;
                Some(ReferencingComponent {
                    id: c.id.clone().unwrap_or_default(),
                    name: c.name.clone().unwrap_or_default(),
                    kind: ReferencingKind::from_api(c.reference_type.as_deref().unwrap_or("")),
                    state: c.state.clone().unwrap_or_default(),
                    active_thread_count: c.active_thread_count.unwrap_or(0).max(0) as u32,
                    group_id: c.group_id.clone().unwrap_or_default(),
                })
            })
            .collect();

        Ok(ControllerServiceDetail {
            id: cs_id.to_string(),
            name: component.name.unwrap_or_default(),
            type_name: component.r#type.unwrap_or_default(),
            bundle: bundle_str,
            state: component.state.unwrap_or_default(),
            parent_group_id: component.parent_group_id,
            properties,
            validation_errors: component.validation_errors.unwrap_or_default(),
            bulletin_level: component.bulletin_level.unwrap_or_default(),
            comments: component.comments.unwrap_or_default(),
            restricted: component.restricted.unwrap_or(false),
            deprecated: component.deprecated.unwrap_or(false),
            persists_state: component.persists_state.unwrap_or(false),
            referencing_components,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortKind {
    Input,
    Output,
}

impl PortKind {
    /// Lowercase wire-format label (`"input"` / `"output"`) used in
    /// log messages and error variants.
    pub fn label(self) -> &'static str {
        match self {
            Self::Input => "input",
            Self::Output => "output",
        }
    }
}

#[derive(Debug, Clone)]
pub struct PortDetail {
    pub id: String,
    pub name: String,
    pub kind: PortKind,
    pub state: String,
    pub comments: String,
    pub concurrent_tasks: u32,
    pub parent_group_id: Option<String>,
}

impl NifiClient {
    /// Fetch the full port detail (identity, state, comments, concurrent
    /// task count, parent group) for one input or output port. Used by
    /// the Browser detail-pane worker on port selection.
    pub async fn browser_port_detail(
        &self,
        port_id: &str,
        kind: PortKind,
    ) -> Result<PortDetail, NifiLensError> {
        tracing::debug!(context = %self.context_name(), %port_id, ?kind, "fetching port detail");
        let (name, state, comments, concurrent_tasks, parent_group_id) = match kind {
            PortKind::Input => {
                let entity = self
                    .inputports()
                    .get_input_port(port_id)
                    .await
                    .map_err(|err| {
                        classify_or_fallback(self.context_name(), Box::new(err), |source| {
                            NifiLensError::PortDetailFailed {
                                context: self.context_name().to_string(),
                                id: port_id.to_string(),
                                kind: "input",
                                source,
                            }
                        })
                    })?;
                let c = entity.component.unwrap_or_default();
                (
                    c.name.unwrap_or_default(),
                    c.state.unwrap_or_default(),
                    c.comments.unwrap_or_default(),
                    c.concurrently_schedulable_task_count.unwrap_or(0).max(0) as u32,
                    c.parent_group_id,
                )
            }
            PortKind::Output => {
                let entity = self
                    .outputports()
                    .get_output_port(port_id)
                    .await
                    .map_err(|err| {
                        classify_or_fallback(self.context_name(), Box::new(err), |source| {
                            NifiLensError::PortDetailFailed {
                                context: self.context_name().to_string(),
                                id: port_id.to_string(),
                                kind: "output",
                                source,
                            }
                        })
                    })?;
                let c = entity.component.unwrap_or_default();
                (
                    c.name.unwrap_or_default(),
                    c.state.unwrap_or_default(),
                    c.comments.unwrap_or_default(),
                    c.concurrently_schedulable_task_count.unwrap_or(0).max(0) as u32,
                    c.parent_group_id,
                )
            }
        };

        Ok(PortDetail {
            id: port_id.to_string(),
            name,
            kind,
            state,
            comments,
            concurrent_tasks,
            parent_group_id,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    #[allow(clippy::field_reassign_with_default)]
    fn walk_pg_nodes_emits_rpg_row_under_parent_pg() {
        use nifi_rust_client::dynamic::types::{
            ProcessGroupStatusSnapshotDto, RemoteProcessGroupStatusSnapshotDto,
            RemoteProcessGroupStatusSnapshotEntity,
        };
        let mut rpg_dto = RemoteProcessGroupStatusSnapshotDto::default();
        rpg_dto.id = Some("rpg-1".into());
        rpg_dto.group_id = Some("pg-1".into());
        rpg_dto.name = Some("MyRemoteSink".into());
        rpg_dto.transmission_status = Some("Transmitting".into());
        rpg_dto.active_thread_count = Some(2);
        rpg_dto.flow_files_received = Some(5);
        rpg_dto.flow_files_sent = Some(7);
        rpg_dto.bytes_received = Some(100);
        rpg_dto.bytes_sent = Some(200);
        rpg_dto.target_uri = Some("https://nifi-east:8443/nifi".into());

        let mut rpg_entity = RemoteProcessGroupStatusSnapshotEntity::default();
        rpg_entity.id = Some("rpg-1".into());
        rpg_entity.remote_process_group_status_snapshot = Some(rpg_dto);

        let mut pg = ProcessGroupStatusSnapshotDto::default();
        pg.id = Some("pg-1".into());
        pg.name = Some("root".into());
        pg.remote_process_group_status_snapshots = Some(vec![rpg_entity]);

        let mut out = Vec::new();
        walk_pg_nodes(&pg, None, &mut out);
        assert_eq!(out.len(), 2);
        assert!(matches!(out[0].kind, NodeKind::ProcessGroup));
        assert!(matches!(out[1].kind, NodeKind::RemoteProcessGroup));
        assert_eq!(out[1].id, "rpg-1");
        assert_eq!(out[1].group_id, "pg-1");
        assert_eq!(out[1].name, "MyRemoteSink");
        assert_eq!(out[1].parent_idx, Some(0));
        match &out[1].status_summary {
            NodeStatusSummary::RemoteProcessGroup {
                transmission_status,
                active_threads,
                flow_files_received,
                flow_files_sent,
                bytes_received,
                bytes_sent,
                target_uri,
            } => {
                assert_eq!(transmission_status, "Transmitting");
                assert_eq!(*active_threads, 2);
                assert_eq!(*flow_files_received, 5);
                assert_eq!(*flow_files_sent, 7);
                assert_eq!(*bytes_received, 100);
                assert_eq!(*bytes_sent, 200);
                assert_eq!(target_uri, "https://nifi-east:8443/nifi");
            }
            other => panic!("expected RemoteProcessGroup status_summary; got {other:?}"),
        }
    }

    #[test]
    fn connection_summary_carries_endpoint_ids_and_names() {
        let summary = NodeStatusSummary::Connection {
            fill_percent: 10,
            flow_files_queued: 1,
            queued_display: "1 / 1B".into(),
            source_id: "src-id".into(),
            source_name: "SrcProc".into(),
            destination_id: "dst-id".into(),
            destination_name: "DstProc".into(),
        };
        if let NodeStatusSummary::Connection {
            source_id,
            source_name,
            destination_id,
            destination_name,
            ..
        } = summary
        {
            assert_eq!(source_id, "src-id");
            assert_eq!(source_name, "SrcProc");
            assert_eq!(destination_id, "dst-id");
            assert_eq!(destination_name, "DstProc");
        } else {
            panic!("expected Connection variant");
        }
    }
}
