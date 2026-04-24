//! High-level NiFi client wrapper used by nifi-lens.
//!
//! The wrapper owns a `nifi_rust_client::dynamic::DynamicClient`, the
//! originating context's name, and the version that the library detected at
//! login time. The wrapped client is exposed via `Deref` so callers can write
//! `client.flow().get_about_info()` without an explicit accessor.

pub mod browser;
pub mod build;
pub mod events;
pub mod health;
pub mod status;
pub mod tls_cert;
pub mod tracer;

use std::ops::{Deref, DerefMut};

use nifi_rust_client::NifiError;
use nifi_rust_client::dynamic::DynamicClient;
use semver::Version;

use crate::config::{ResolvedAuth, ResolvedContext};
use crate::error::NifiLensError;

/// Try to classify a boxed library error into a specific `NifiLensError`
/// variant with a targeted hint, falling back to a caller-provided
/// generic constructor when no specific match is found.
///
/// Downcasts the boxed source to `nifi_rust_client::NifiError` and matches
/// on the variant. Unclassified variants (network errors, 5xx responses,
/// etc.) pass through to `fallback`.
pub(crate) fn classify_or_fallback(
    context: &str,
    source: Box<dyn std::error::Error + Send + Sync>,
    fallback: impl FnOnce(Box<dyn std::error::Error + Send + Sync>) -> NifiLensError,
) -> NifiLensError {
    if let Some(nifi_err) = source.downcast_ref::<NifiError>() {
        match nifi_err {
            NifiError::UnsupportedVersion { detected } => {
                return NifiLensError::UnsupportedNifiVersion {
                    context: context.to_string(),
                    detected: detected.clone(),
                };
            }
            NifiError::InvalidCertificate { .. } => {
                return NifiLensError::TlsCertInvalid {
                    context: context.to_string(),
                    source,
                };
            }
            NifiError::Unauthorized { .. } | NifiError::Auth { .. } => {
                return NifiLensError::NifiUnauthorized {
                    context: context.to_string(),
                };
            }
            _ => {}
        }
    }
    fallback(source)
}

fn collect_queues(
    snapshot: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    out: &mut Vec<QueueSnapshot>,
) {
    if let Some(conns) = snapshot.connection_status_snapshots.as_ref() {
        for entity in conns {
            let Some(conn) = entity.connection_status_snapshot.as_ref() else {
                continue;
            };
            let by_count = conn.percent_use_count.unwrap_or(0).max(0) as u32;
            let by_bytes = conn.percent_use_bytes.unwrap_or(0).max(0) as u32;
            out.push(QueueSnapshot {
                id: conn.id.clone().unwrap_or_default(),
                group_id: conn.group_id.clone().unwrap_or_default(),
                name: conn.name.clone().unwrap_or_default(),
                source_name: conn.source_name.clone().unwrap_or_default(),
                destination_name: conn.destination_name.clone().unwrap_or_default(),
                fill_percent: by_count.max(by_bytes),
                flow_files_queued: conn.flow_files_queued.unwrap_or(0).max(0) as u32,
                bytes_queued: conn.bytes_queued.unwrap_or(0).max(0) as u64,
                queued_display: conn.queued.clone().unwrap_or_default(),
            });
        }
    }
    if let Some(children) = snapshot.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                collect_queues(child, out);
            }
        }
    }
}

/// Thin wrapper around `nifi_rust_client::dynamic::DynamicClient`.
///
/// Exposed via `Deref` so callers can invoke any NiFi API method directly.
pub struct NifiClient {
    inner: DynamicClient,
    context_name: String,
    detected_version: Version,
    base_url: String,
}

impl std::fmt::Debug for NifiClient {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // DynamicClient does not implement Debug; we emit just the fields we own.
        f.debug_struct("NifiClient")
            .field("context_name", &self.context_name)
            .field("detected_version", &self.detected_version)
            .finish_non_exhaustive()
    }
}

impl Deref for NifiClient {
    type Target = DynamicClient;
    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl DerefMut for NifiClient {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

impl NifiClient {
    /// Construct a `NifiClient` directly from parts.
    ///
    /// Only available in `#[cfg(test)]`. Production code always goes through
    /// `NifiClient::connect` so that authentication and version detection run.
    #[cfg(test)]
    pub(crate) fn from_parts(
        inner: DynamicClient,
        context_name: impl Into<String>,
        detected_version: Version,
    ) -> Self {
        Self {
            inner,
            context_name: context_name.into(),
            detected_version,
            base_url: "https://test:8443".to_string(),
        }
    }

    /// Build, authenticate, detect version, and return a connected client.
    pub async fn connect(ctx: &ResolvedContext) -> Result<Self, NifiLensError> {
        tracing::debug!(context = %ctx.name, url = %ctx.url, "connecting");

        let inner = build::build_dynamic_client(ctx)?;

        // Authenticate and detect the NiFi server version.
        //
        // - Password auth: DynamicClient::login() authenticates AND detects
        //   the version in one step.
        // - Token auth: install the pre-obtained JWT, then detect version
        //   explicitly (set_token does not trigger version detection).
        // - mTLS: the TLS handshake already authenticated; just detect version.
        match &ctx.auth {
            ResolvedAuth::Password { username, password } => {
                inner.login(username, password).await.map_err(|err| {
                    classify_or_fallback(&ctx.name, Box::new(err), |source| {
                        NifiLensError::LoginFailed {
                            context: ctx.name.clone(),
                            source,
                        }
                    })
                })?;
            }
            ResolvedAuth::Token { token } => {
                inner.inner().set_token(token.clone()).await;
                inner.detect_version().await.map_err(|err| {
                    classify_or_fallback(&ctx.name, Box::new(err), |source| {
                        NifiLensError::LoginFailed {
                            context: ctx.name.clone(),
                            source,
                        }
                    })
                })?;
            }
            ResolvedAuth::Mtls { .. } => {
                inner.detect_version().await.map_err(|err| {
                    classify_or_fallback(&ctx.name, Box::new(err), |source| {
                        NifiLensError::LoginFailed {
                            context: ctx.name.clone(),
                            source,
                        }
                    })
                })?;
            }
        }

        let detected =
            inner
                .detected_version()
                .ok_or_else(|| NifiLensError::VersionDetectionMissing {
                    context: ctx.name.clone(),
                })?;
        let version_str = detected.to_string();
        let detected_version =
            Version::parse(&version_str).map_err(|err| NifiLensError::LoginFailed {
                context: ctx.name.clone(),
                source: Box::new(err),
            })?;

        Ok(Self {
            inner,
            context_name: ctx.name.clone(),
            detected_version,
            base_url: ctx.url.clone(),
        })
    }

    pub fn context_name(&self) -> &str {
        &self.context_name
    }

    pub fn detected_version(&self) -> &Version {
        &self.detected_version
    }

    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Convenience wrapper around `flow().get_about_info()` that maps
    /// the error into `NifiLensError`.
    pub async fn about(&self) -> Result<AboutSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name, "fetching /flow/about");
        let about = self.inner.flow().get_about_info().await.map_err(|err| {
            classify_or_fallback(&self.context_name, Box::new(err), |source| {
                NifiLensError::AboutFailed {
                    context: self.context_name.clone(),
                    source,
                }
            })
        })?;

        Ok(AboutSnapshot {
            version: about.version.clone().unwrap_or_default(),
            title: about.title.clone().unwrap_or_default(),
        })
    }

    /// Calls `flow().get_controller_status()` and flattens the response.
    pub async fn controller_status(&self) -> Result<ControllerStatusSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name, "fetching /flow/status");
        let dto = self
            .inner
            .flow()
            .get_controller_status()
            .await
            .map_err(|err| {
                classify_or_fallback(&self.context_name, Box::new(err), |source| {
                    NifiLensError::ControllerStatusFailed {
                        context: self.context_name.clone(),
                        source,
                    }
                })
            })?;
        Ok(ControllerStatusSnapshot::from_dto(&dto))
    }

    /// Calls `flow().get_process_group_status("root", recursive=true)`
    /// and flattens every descendant connection into a sorted `QueueSnapshot`
    /// list.
    pub async fn root_pg_status(&self) -> Result<RootPgStatusSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name, "fetching /flow/process-groups/root/status");
        let entity = self
            .inner
            .flow()
            .get_process_group_status("root", Some(true), None, None)
            .await
            .map_err(|err| {
                classify_or_fallback(&self.context_name, Box::new(err), |source| {
                    NifiLensError::ProcessGroupStatusFailed {
                        context: self.context_name.clone(),
                        source,
                    }
                })
            })?;

        let snapshot = entity
            .process_group_status
            .and_then(|pg| pg.aggregate_snapshot)
            .map(|agg| RootPgStatusSnapshot::from_aggregate(&agg))
            .unwrap_or_default();
        Ok(snapshot)
    }

    /// Calls `flow().get_controller_services_from_group("root", false, true, false, None)`
    /// and collapses the listing into a combined counts + per-CS member
    /// list. Overview reads `.counts`; Browser reads `.members`. Shared
    /// between the two so only one round trip is made.
    pub async fn controller_services_snapshot(
        &self,
    ) -> Result<ControllerServicesSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name,
            "fetching /flow/process-groups/root/controller-services?descendant=true"
        );
        let listing = self
            .inner
            .flow()
            .get_controller_services_from_group("root", Some(false), Some(true), Some(false), None)
            .await
            .map_err(|err| {
                classify_or_fallback(&self.context_name, Box::new(err), |source| {
                    NifiLensError::ControllerServicesListFailed {
                        context: self.context_name.clone(),
                        id: "root".to_string(),
                        source,
                    }
                })
            })?;
        Ok(ControllerServicesSnapshot::from_listing(&listing))
    }

    /// Calls `flow().get_bulletin_board(after, None, None, None, None, limit)`
    /// and flattens the response for the Overview reducer.
    pub async fn bulletin_board(
        &self,
        after_id: Option<i64>,
        limit: Option<u32>,
    ) -> Result<BulletinBoardSnapshot, NifiLensError> {
        tracing::debug!(context = %self.context_name, "fetching /flow/bulletin-board");
        let after = after_id.map(|n| n.to_string());
        let limit_s = limit.map(|n| n.to_string());
        let board = self
            .inner
            .flow()
            .get_bulletin_board(after.as_deref(), None, None, None, None, limit_s.as_deref())
            .await
            .map_err(|err| {
                classify_or_fallback(&self.context_name, Box::new(err), |source| {
                    NifiLensError::BulletinBoardFailed {
                        context: self.context_name.clone(),
                        source,
                    }
                })
            })?;

        let now = time::OffsetDateTime::now_utc();
        let bulletins = board
            .bulletins
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entity| {
                let dto = entity.bulletin?;
                let timestamp_iso = match dto.timestamp_iso {
                    Some(iso) if !iso.is_empty() => iso,
                    _ => dto
                        .timestamp
                        .as_deref()
                        .and_then(|t| crate::timestamp::synthesize_iso_from_time_only(t, now))
                        .unwrap_or_default(),
                };
                Some(BulletinSnapshot {
                    id: dto.id.or(entity.id).unwrap_or(0),
                    level: dto.level.unwrap_or_default(),
                    message: dto.message.unwrap_or_default(),
                    source_id: dto.source_id.or(entity.source_id).unwrap_or_default(),
                    source_name: dto.source_name.unwrap_or_default(),
                    source_type: dto.source_type.unwrap_or_default(),
                    group_id: dto.group_id.or(entity.group_id).unwrap_or_default(),
                    timestamp_iso,
                    timestamp_human: dto.timestamp.unwrap_or_default(),
                })
            })
            .collect();

        Ok(BulletinBoardSnapshot { bulletins })
    }
}

/// Snapshot of the `/flow/about` endpoint used by the identity strip.
#[derive(Debug, Clone, Default)]
pub struct AboutSnapshot {
    pub version: String,
    pub title: String,
}

/// Global component counts pulled from `flow().get_controller_status()`.
/// Used by the Overview tab's "Components" panel.
#[derive(Debug, Clone, Default)]
pub struct ControllerStatusSnapshot {
    pub running: u32,
    pub stopped: u32,
    pub invalid: u32,
    pub disabled: u32,
    pub active_threads: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    /// Versioned PGs whose registry version has been superseded.
    pub stale: u32,
    /// Versioned PGs with uncommitted local edits.
    pub locally_modified: u32,
    /// Versioned PGs that failed to reach the registry on the last check.
    pub sync_failure: u32,
    /// Versioned PGs that match the registry version (kept for completeness; the panel
    /// derives this as a residual from `total - (stale + locally_modified + sync_failure)`).
    pub up_to_date: u32,
}

impl ControllerStatusSnapshot {
    /// Build a snapshot from the raw `ControllerStatusDto`. Pure; no I/O.
    /// All fields are read defensively because every field on the DTO is
    /// `Option<i32>` / `Option<i64>`.
    pub fn from_dto(dto: &nifi_rust_client::dynamic::types::ControllerStatusDto) -> Self {
        Self {
            running: dto.running_count.unwrap_or(0).max(0) as u32,
            stopped: dto.stopped_count.unwrap_or(0).max(0) as u32,
            invalid: dto.invalid_count.unwrap_or(0).max(0) as u32,
            disabled: dto.disabled_count.unwrap_or(0).max(0) as u32,
            active_threads: dto.active_thread_count.unwrap_or(0).max(0) as u32,
            flow_files_queued: dto.flow_files_queued.unwrap_or(0).max(0) as u32,
            bytes_queued: dto.bytes_queued.unwrap_or(0).max(0) as u64,
            stale: dto.stale_count.unwrap_or(0).max(0) as u32,
            locally_modified: dto.locally_modified_count.unwrap_or(0).max(0) as u32,
            sync_failure: dto.sync_failure_count.unwrap_or(0).max(0) as u32,
            up_to_date: dto.up_to_date_count.unwrap_or(0).max(0) as u32,
        }
    }
}

/// One connection (queue) row as surfaced to the Overview tab.
/// `fill_percent` is `max(percent_use_count, percent_use_bytes)` so the
/// leaderboard leads with the queue closest to back-pressure regardless of
/// whether the threshold is count- or byte-based.
#[derive(Debug, Clone)]
pub struct QueueSnapshot {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub source_name: String,
    pub destination_name: String,
    pub fill_percent: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub queued_display: String,
}

/// Per-state processor counts derived from a recursive
/// `ProcessGroupStatusSnapshotDto` walk. Unknown run-statuses
/// are silently dropped.
#[derive(Debug, Clone, Default)]
pub struct ProcessorStateCounts {
    pub running: u32,
    pub stopped: u32,
    pub invalid: u32,
    pub disabled: u32,
}

impl ProcessorStateCounts {
    pub fn total(&self) -> u32 {
        self.running + self.stopped + self.invalid + self.disabled
    }
}

/// Recursive process-group status, flattened for the Overview tab and
/// the Browser arena.
///
/// `connections` is sorted descending by `fill_percent` for the Overview
/// leaderboard; `nodes` is the flat arena-ready DFS walk the Browser tab
/// rebuilds its tree from (Task 6 of the central-cluster-store refactor —
/// Browser no longer fetches the recursive status itself).
#[derive(Debug, Clone, Default)]
pub struct RootPgStatusSnapshot {
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub connections: Vec<QueueSnapshot>,
    /// Total number of process groups in the flow, root inclusive.
    pub process_group_count: u32,
    /// Total input ports across every PG.
    pub input_port_count: u32,
    /// Total output ports across every PG.
    pub output_port_count: u32,
    /// Per-state processor tally across every PG.
    pub processors: ProcessorStateCounts,
    /// Every PG id in the flow in DFS order (root first, then each
    /// subtree). Empty / missing ids are skipped. `ClusterStore` reads
    /// this to drive the connections-by-PG fetcher.
    pub process_group_ids: Vec<String>,
    /// Flat arena-ready node list — one `RawNode` per PG / processor /
    /// connection / port in DFS order, same shape the old
    /// `browser_tree` fan-out produced. Does NOT include controller
    /// services (those are attached by the Browser reducer from the
    /// separate controller-services snapshot) and does NOT include
    /// connection endpoint ids (those are backfilled from
    /// `snapshot.connections_by_pg`).
    pub nodes: Vec<crate::client::browser::RawNode>,
}

impl RootPgStatusSnapshot {
    /// Build a snapshot from the recursive aggregate snapshot returned by
    /// `flow().get_process_group_status("root", recursive=true)`. Pure; no I/O.
    pub fn from_aggregate(
        agg: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    ) -> Self {
        let mut snap = Self {
            flow_files_queued: agg.flow_files_queued.unwrap_or(0).max(0) as u32,
            bytes_queued: agg.bytes_queued.unwrap_or(0).max(0) as u64,
            ..Self::default()
        };
        collect_queues(agg, &mut snap.connections);
        snap.connections
            .sort_by(|a, b| b.fill_percent.cmp(&a.fill_percent));
        collect_counts(agg, &mut snap);
        collect_pg_ids(agg, &mut snap.process_group_ids);
        crate::client::browser::walk_pg_nodes(agg, None, &mut snap.nodes);
        snap
    }

    /// Return every PG id in the flow in DFS order (root first, then
    /// each subtree). Consumed by `ClusterStore::publish_pg_ids` to
    /// drive the connections-by-PG fetcher fan-out.
    pub fn pg_ids(&self) -> Vec<String> {
        self.process_group_ids.clone()
    }
}

/// Walks the PG tree and tallies PGs, ports, and processors per state into `out`.
fn collect_counts(
    snapshot: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    out: &mut RootPgStatusSnapshot,
) {
    out.process_group_count += 1;
    if let Some(ports) = snapshot.input_port_status_snapshots.as_ref() {
        out.input_port_count += ports.len() as u32;
    }
    if let Some(ports) = snapshot.output_port_status_snapshots.as_ref() {
        out.output_port_count += ports.len() as u32;
    }
    if let Some(procs) = snapshot.processor_status_snapshots.as_ref() {
        for entity in procs {
            let Some(snap) = entity.processor_status_snapshot.as_ref() else {
                continue;
            };
            // Normalize case — NiFi's recursive status endpoint emits title-case,
            // but the component endpoint emits uppercase. Match codebase convention
            // (see widget/run_icon.rs).
            match snap
                .run_status
                .as_deref()
                .map(str::to_ascii_uppercase)
                .as_deref()
            {
                Some("RUNNING") => out.processors.running += 1,
                Some("STOPPED") => out.processors.stopped += 1,
                Some("INVALID") => out.processors.invalid += 1,
                Some("DISABLED") => out.processors.disabled += 1,
                _ => { /* unknown ("VALIDATING") or null — drop silently */ }
            }
        }
    }
    if let Some(children) = snapshot.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                collect_counts(child, out);
            }
        }
    }
}

/// DFS-walks the PG tree collecting the `id` of every non-empty PG.
/// Root is emitted first, then each subtree in order. Consumed by
/// `RootPgStatusSnapshot::from_aggregate` to populate `process_group_ids`.
fn collect_pg_ids(
    snapshot: &nifi_rust_client::dynamic::types::ProcessGroupStatusSnapshotDto,
    out: &mut Vec<String>,
) {
    if let Some(id) = snapshot.id.as_deref()
        && !id.is_empty()
    {
        out.push(id.to_string());
    }
    if let Some(children) = snapshot.process_group_status_snapshots.as_ref() {
        for entity in children {
            if let Some(child) = entity.process_group_status_snapshot.as_ref() {
                collect_pg_ids(child, out);
            }
        }
    }
}

/// Bulletin severity in sort order: Info < Warning < Error. `Unknown`
/// covers everything NiFi sends outside of the standard three.
///
/// This type lives in the `client` module because severity is a property of
/// bulletins, and every view that processes bulletins (Overview, Bulletins,
/// future tabs) needs to parse it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum Severity {
    #[default]
    Unknown,
    Info,
    Warning,
    Error,
}

impl Severity {
    pub fn parse(level: &str) -> Self {
        match level.to_ascii_uppercase().as_str() {
            "ERROR" => Self::Error,
            "WARN" | "WARNING" => Self::Warning,
            "INFO" => Self::Info,
            _ => Self::Unknown,
        }
    }
}

/// One bulletin row as surfaced to the Overview tab.
#[derive(Debug, Clone)]
pub struct BulletinSnapshot {
    pub id: i64,
    pub level: String,
    pub message: String,
    pub source_id: String,
    pub source_name: String,
    pub source_type: String,
    pub group_id: String,
    /// RFC-3339 / ISO-8601 timestamp ("2026-04-11T10:14:22.123Z"). On
    /// NiFi < 2.7.2 the server does not send `timestampIso` at all and
    /// `timestamp` is time-only (`HH:MM:SS UTC`); the client synthesizes
    /// an ISO string by combining it with the fetch-time UTC date
    /// (see `crate::timestamp::synthesize_iso_from_time_only`). Empty
    /// only when neither field was parseable.
    pub timestamp_iso: String,
    /// Human-readable timestamp. Full date on NiFi >= 2.7.2
    /// ("04/12/2026 11:44:00 UTC"); time-only on NiFi < 2.7.2
    /// ("11:44:00 UTC"). Kept verbatim from the server.
    pub timestamp_human: String,
}

/// Bulletin-board snapshot: just the list of bulletins. The Overview reducer
/// bins them into time buckets; the Bulletins tab (Phase 2) will consume the
/// full payload including the `generated` cursor.
#[derive(Debug, Clone, Default)]
pub struct BulletinBoardSnapshot {
    pub bulletins: Vec<BulletinSnapshot>,
}

/// Per-state controller-service counts pulled from
/// `flow().get_controller_services_from_group(root, ancestors=false, descendants=true)`.
/// Classification priority is `validation_status == "INVALID"` first, then `state`.
#[derive(Debug, Clone, Default)]
pub struct ControllerServiceCounts {
    pub enabled: u32,
    pub disabled: u32,
    pub invalid: u32,
}

impl ControllerServiceCounts {
    pub fn total(&self) -> u32 {
        self.enabled + self.disabled + self.invalid
    }

    /// Tally the listing into per-state buckets. Pure; no I/O.
    pub fn from_listing(
        listing: &nifi_rust_client::dynamic::types::ControllerServicesEntity,
    ) -> Self {
        let mut out = Self::default();
        let Some(items) = listing.controller_services.as_ref() else {
            return out;
        };
        for entity in items {
            let Some(c) = entity.component.as_ref() else {
                continue;
            };
            // Match codebase convention (widget/run_icon.rs): normalize via uppercase.
            let validation_upper = c.validation_status.as_deref().map(str::to_ascii_uppercase);
            let state_upper = c.state.as_deref().map(str::to_ascii_uppercase);
            if validation_upper.as_deref() == Some("INVALID") {
                out.invalid += 1;
            } else if state_upper.as_deref() == Some("ENABLED") {
                out.enabled += 1;
            } else {
                // DISABLED, ENABLING, DISABLING, missing state — all collapse here.
                out.disabled += 1;
            }
        }
        out
    }
}

/// One controller service's identity plus owning PG. Consumed by the
/// Browser reducer to attach CS rows to the tree arena; Overview only
/// reads the `counts` sibling so this list is invisible there.
#[derive(Debug, Clone, Default)]
pub struct ControllerServiceMember {
    pub id: String,
    pub name: String,
    pub state: String,
    pub parent_group_id: String,
}

/// Combined controller-services payload stored in `ClusterSnapshot`:
/// the aggregate counts for Overview plus the per-CS member list the
/// Browser reducer splices into the arena. Extracted from a single
/// `/flow/controller-services` call so no extra round trips are
/// required.
#[derive(Debug, Clone, Default)]
pub struct ControllerServicesSnapshot {
    pub counts: ControllerServiceCounts,
    pub members: Vec<ControllerServiceMember>,
}

impl ControllerServicesSnapshot {
    /// Build a snapshot from the raw listing entity. Pure; no I/O.
    /// Each component without a `parent_group_id` is dropped from
    /// `members` — the reducer cannot attach it to any PG — but still
    /// contributes to `counts`.
    pub fn from_listing(
        listing: &nifi_rust_client::dynamic::types::ControllerServicesEntity,
    ) -> Self {
        let counts = ControllerServiceCounts::from_listing(listing);
        let mut members = Vec::new();
        if let Some(items) = listing.controller_services.as_ref() {
            for entity in items {
                let Some(c) = entity.component.as_ref() else {
                    continue;
                };
                let Some(pg_id) = c.parent_group_id.clone() else {
                    continue;
                };
                members.push(ControllerServiceMember {
                    id: c.id.clone().unwrap_or_default(),
                    name: c.name.clone().unwrap_or_default(),
                    state: c.state.clone().unwrap_or_default(),
                    parent_group_id: pg_id,
                });
            }
        }
        Self { counts, members }
    }
}

pub use browser::{
    ConnectionDetail, ConnectionEndpointIds, ConnectionEndpoints, ControllerServiceDetail,
    ControllerServiceSummary, FolderKind, NodeKind, NodeStatusSummary, PortDetail, PortKind,
    ProcessGroupDetail, ProcessorDetail, RawNode, RecursiveSnapshot, ReferencingComponent,
    ReferencingKind,
};
pub use events::{ProvenancePollResult, ProvenanceQuery, ProvenanceQueryHandle};
pub use health::{
    GcSnapshot, NodeDiagnostics, NodeHealthRow, RepoUsage, Severity as HealthSeverity,
    SystemDiagAggregate, SystemDiagSnapshot,
};
pub use tls_cert::{CertEntry, NodeCertChain, TlsCertsSnapshot, TlsProbeError};
pub use tracer::{
    AttributeTriple, ContentRangeSnapshot, ContentRender, ContentSide, ContentSnapshot,
    INLINE_PREVIEW_BYTES, LatestEventsSnapshot, LineagePoll, LineageSnapshot,
    ProvenanceEventDetail, ProvenanceEventSummary,
};

#[cfg(test)]
mod root_pg_status_snapshot_tests {
    use super::*;
    use nifi_rust_client::dynamic::types::{
        PortStatusSnapshotEntity, ProcessGroupStatusSnapshotDto, ProcessGroupStatusSnapshotEntity,
        ProcessorStatusSnapshotDto, ProcessorStatusSnapshotEntity,
    };

    fn proc(state: &str) -> ProcessorStatusSnapshotEntity {
        let mut snap = ProcessorStatusSnapshotDto::default();
        snap.run_status = Some(state.into());
        let mut entity = ProcessorStatusSnapshotEntity::default();
        entity.processor_status_snapshot = Some(snap);
        entity
    }

    fn port() -> PortStatusSnapshotEntity {
        PortStatusSnapshotEntity::default()
    }

    #[test]
    fn walker_tallies_descendants() {
        // root has: 2 procs (Running, Stopped), 1 input port, 0 output, 1 child PG.
        // child PG has: 1 proc (Invalid), 0 ports, 0 children.
        let mut child_pg = ProcessGroupStatusSnapshotDto::default();
        child_pg.processor_status_snapshots = Some(vec![proc("Invalid")]);

        let mut child_entity = ProcessGroupStatusSnapshotEntity::default();
        child_entity.process_group_status_snapshot = Some(child_pg);

        let mut root = ProcessGroupStatusSnapshotDto::default();
        root.processor_status_snapshots = Some(vec![proc("Running"), proc("Stopped")]);
        root.input_port_status_snapshots = Some(vec![port()]);
        root.output_port_status_snapshots = None;
        root.process_group_status_snapshots = Some(vec![child_entity]);

        let snap = RootPgStatusSnapshot::from_aggregate(&root);
        assert_eq!(snap.process_group_count, 2, "root + 1 child");
        assert_eq!(snap.input_port_count, 1);
        assert_eq!(snap.output_port_count, 0);
        assert_eq!(snap.processors.running, 1);
        assert_eq!(snap.processors.stopped, 1);
        assert_eq!(snap.processors.invalid, 1);
        assert_eq!(snap.processors.disabled, 0);
    }

    #[test]
    fn walker_handles_unknown_run_status_gracefully() {
        let mut root = ProcessGroupStatusSnapshotDto::default();
        // Test case-insensitivity: "disabled" lowercase should still count.
        // "Validating" should drop silently.
        root.processor_status_snapshots = Some(vec![proc("Validating"), proc("disabled")]);

        let snap = RootPgStatusSnapshot::from_aggregate(&root);
        assert_eq!(snap.processors.disabled, 1);
        // Unknown states ("Validating") are silently dropped — they're rare and not surfaced.
        assert_eq!(
            snap.processors.running + snap.processors.stopped + snap.processors.invalid,
            0
        );
    }

    fn pg_with_id(id: &str) -> ProcessGroupStatusSnapshotDto {
        let mut dto = ProcessGroupStatusSnapshotDto::default();
        dto.id = Some(id.into());
        dto
    }

    fn pg_entity(dto: ProcessGroupStatusSnapshotDto) -> ProcessGroupStatusSnapshotEntity {
        let mut entity = ProcessGroupStatusSnapshotEntity::default();
        entity.process_group_status_snapshot = Some(dto);
        entity
    }

    #[test]
    fn pg_ids_flattens_recursive_snapshot() {
        // Shape: root → childA → grandchild; root → childB.
        let grandchild = pg_with_id("grandchild");
        let mut child_a = pg_with_id("childA");
        child_a.process_group_status_snapshots = Some(vec![pg_entity(grandchild)]);
        let child_b = pg_with_id("childB");

        let mut root = pg_with_id("root");
        root.process_group_status_snapshots = Some(vec![pg_entity(child_a), pg_entity(child_b)]);

        let snap = RootPgStatusSnapshot::from_aggregate(&root);
        assert_eq!(
            snap.pg_ids(),
            vec![
                "root".to_string(),
                "childA".to_string(),
                "grandchild".to_string(),
                "childB".to_string(),
            ],
            "DFS order: root, each subtree in order"
        );
    }

    #[test]
    fn pg_ids_skips_empty_and_missing_ids() {
        // Root has empty id, child is missing id entirely, grandchild has one.
        let mut grandchild = ProcessGroupStatusSnapshotDto::default();
        grandchild.id = Some("gc".into());
        let mut child = ProcessGroupStatusSnapshotDto::default();
        child.id = None;
        child.process_group_status_snapshots = Some(vec![pg_entity(grandchild)]);

        let mut root = ProcessGroupStatusSnapshotDto::default();
        root.id = Some(String::new());
        root.process_group_status_snapshots = Some(vec![pg_entity(child)]);

        let snap = RootPgStatusSnapshot::from_aggregate(&root);
        assert_eq!(snap.pg_ids(), vec!["gc".to_string()]);
    }
}

#[cfg(test)]
mod controller_status_snapshot_tests {
    use super::*;

    #[test]
    fn from_dto_clamps_negatives_and_handles_missing() {
        // Test with a default DTO and manually set fields via builder pattern
        // since ControllerStatusDto is marked #[non_exhaustive].
        let mut dto = nifi_rust_client::dynamic::types::ControllerStatusDto::default();
        dto.running_count = Some(7);
        dto.stopped_count = Some(-1); // NiFi shouldn't, but clamp anyway
        dto.invalid_count = None;
        dto.disabled_count = Some(2);
        dto.active_thread_count = Some(3);
        dto.flow_files_queued = Some(100);
        dto.bytes_queued = Some(2048);

        let snap = ControllerStatusSnapshot::from_dto(&dto);
        assert_eq!(snap.running, 7);
        assert_eq!(snap.stopped, 0);
        assert_eq!(snap.invalid, 0);
        assert_eq!(snap.disabled, 2);
        assert_eq!(snap.active_threads, 3);
        assert_eq!(snap.flow_files_queued, 100);
        assert_eq!(snap.bytes_queued, 2048);
    }

    #[test]
    fn from_dto_populates_version_sync_counts() {
        let mut dto = nifi_rust_client::dynamic::types::ControllerStatusDto::default();
        dto.stale_count = Some(1);
        dto.locally_modified_count = Some(2);
        dto.sync_failure_count = Some(0);
        dto.up_to_date_count = Some(4);

        let snap = ControllerStatusSnapshot::from_dto(&dto);
        assert_eq!(snap.stale, 1);
        assert_eq!(snap.locally_modified, 2);
        assert_eq!(snap.sync_failure, 0);
        assert_eq!(snap.up_to_date, 4);
    }
}

#[cfg(test)]
mod controller_service_counts_tests {
    use super::*;
    use nifi_rust_client::dynamic::types::{
        ControllerServiceDto, ControllerServiceEntity, ControllerServicesEntity,
    };

    fn cs(state: &str, validation: &str) -> ControllerServiceEntity {
        let mut dto = ControllerServiceDto::default();
        dto.state = Some(state.into());
        dto.validation_status = Some(validation.into());
        let mut entity = ControllerServiceEntity::default();
        entity.component = Some(dto);
        entity
    }

    #[test]
    fn tally_classifies_invalid_first_then_state() {
        let mut listing = ControllerServicesEntity::default();
        listing.controller_services = Some(vec![
            cs("ENABLED", "VALID"),
            cs("ENABLED", "VALID"),
            cs("DISABLED", "VALID"),
            cs("DISABLED", "INVALID"), // counted as INVALID, not DISABLED
            cs("ENABLED", "INVALID"),  // counted as INVALID, not ENABLED
            cs("ENABLING", "VALIDATING"), // falls through to "other" -> disabled bucket
        ]);
        let counts = ControllerServiceCounts::from_listing(&listing);
        assert_eq!(counts.enabled, 2);
        assert_eq!(counts.disabled, 2, "1 truly disabled + 1 ENABLING/other");
        assert_eq!(counts.invalid, 2);
        assert_eq!(counts.total(), 6);
    }
}
