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
        })
    }

    pub fn context_name(&self) -> &str {
        &self.context_name
    }

    pub fn detected_version(&self) -> &Version {
        &self.detected_version
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

        let mut snapshot = RootPgStatusSnapshot::default();
        let Some(pg_dto) = entity.process_group_status else {
            return Ok(snapshot);
        };
        if let Some(agg) = &pg_dto.aggregate_snapshot {
            snapshot.flow_files_queued = agg.flow_files_queued.unwrap_or(0).max(0) as u32;
            snapshot.bytes_queued = agg.bytes_queued.unwrap_or(0).max(0) as u64;
            collect_queues(agg, &mut snapshot.connections);
        }
        snapshot
            .connections
            .sort_by(|a, b| b.fill_percent.cmp(&a.fill_percent));
        Ok(snapshot)
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

        let bulletins = board
            .bulletins
            .unwrap_or_default()
            .into_iter()
            .filter_map(|entity| {
                let dto = entity.bulletin?;
                Some(BulletinSnapshot {
                    id: dto.id.or(entity.id).unwrap_or(0),
                    level: dto.level.unwrap_or_default(),
                    message: dto.message.unwrap_or_default(),
                    source_id: dto.source_id.or(entity.source_id).unwrap_or_default(),
                    source_name: dto.source_name.unwrap_or_default(),
                    source_type: dto.source_type.unwrap_or_default(),
                    group_id: dto.group_id.or(entity.group_id).unwrap_or_default(),
                    timestamp_iso: dto.timestamp_iso.unwrap_or_default(),
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

/// Recursive process-group status, flattened for the Overview tab.
/// `connections` is sorted descending by `fill_percent`.
#[derive(Debug, Clone, Default)]
pub struct RootPgStatusSnapshot {
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub connections: Vec<QueueSnapshot>,
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
    /// RFC-3339 / ISO-8601 timestamp ("2026-04-11T10:14:22.123Z"). Empty if
    /// the server did not populate `timestampIso`.
    pub timestamp_iso: String,
    /// Human-readable timestamp ("04/12/2026 11:44:00 UTC"). Populated from
    /// the `timestamp` field of the NiFi API; used as fallback when
    /// `timestamp_iso` is empty (NiFi < 2.8.0).
    pub timestamp_human: String,
}

/// Bulletin-board snapshot: just the list of bulletins. The Overview reducer
/// bins them into time buckets; the Bulletins tab (Phase 2) will consume the
/// full payload including the `generated` cursor.
#[derive(Debug, Clone, Default)]
pub struct BulletinBoardSnapshot {
    pub bulletins: Vec<BulletinSnapshot>,
}

pub use browser::{
    ConnectionDetail, ControllerServiceDetail, ControllerServiceSummary, FolderKind, NodeKind,
    NodeStatusSummary, PortDetail, PortKind, ProcessGroupDetail, ProcessorDetail, RawNode,
    RecursiveSnapshot, ReferencingComponent, ReferencingKind,
};
pub use events::{ProvenancePollResult, ProvenanceQuery, ProvenanceQueryHandle};
pub use health::{
    GcSnapshot, NodeDiagnostics, NodeHealthRow, RepoUsage, Severity as HealthSeverity,
    SystemDiagAggregate, SystemDiagSnapshot,
};
pub use tracer::{
    AttributeTriple, ContentRender, ContentSide, ContentSnapshot, LatestEventsSnapshot,
    LineagePoll, LineageSnapshot, PREVIEW_CAP_BYTES, ProvenanceEventDetail, ProvenanceEventSummary,
};

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
