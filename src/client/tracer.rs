// Consumed by Tasks 5–8
#![allow(dead_code)]
//! Tracer-tab client wrappers and snapshot types.
//!
//! Phase 4 forensic flow: paste a flowfile UUID → submit a lineage
//! query → poll → render the event timeline → optionally fetch per-event
//! content. All helpers map errors via `classify_or_fallback` so the UI
//! layer never sees a raw `NifiError`.

use std::sync::Arc;
use std::time::SystemTime;

use nifi_rust_client::dynamic::traits::ProvenanceApi as _;
use nifi_rust_client::dynamic::traits::ProvenanceEventsApi as _;
use nifi_rust_client::dynamic::types::{LineageDto, LineageEntity, LineageRequestDto};

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

/// Direction of a content claim on a provenance event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSide {
    Input,
    Output,
}

impl ContentSide {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentSide::Input => "input",
            ContentSide::Output => "output",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LineageSnapshot {
    pub events: Vec<ProvenanceEventSummary>,
    pub percent_completed: u8,
    pub finished: bool,
}

#[derive(Debug, Clone)]
pub struct ProvenanceEventSummary {
    pub event_id: i64,
    pub event_time_iso: String,
    pub event_type: String,
    pub component_id: String,
    pub component_name: String,
    pub component_type: String,
    pub group_id: String,
    pub flow_file_uuid: String,
    pub relationship: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProvenanceEventDetail {
    pub summary: ProvenanceEventSummary,
    pub attributes: Vec<AttributeTriple>,
    pub transit_uri: Option<String>,
    pub input_available: bool,
    pub output_available: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeTriple {
    pub key: String,
    pub previous: Option<String>,
    pub current: Option<String>,
}

impl AttributeTriple {
    pub fn is_changed(&self) -> bool {
        self.previous != self.current
    }
}

#[derive(Debug, Clone)]
pub enum LineagePoll {
    Running { percent: u8 },
    Finished(LineageSnapshot),
}

#[derive(Debug, Clone)]
pub enum ContentRender {
    Text { pretty: String },
    Hex { first_4k: String },
    Empty,
}

#[derive(Debug, Clone)]
pub struct LatestEventsSnapshot {
    pub component_id: String,
    pub component_label: String,
    pub events: Vec<ProvenanceEventSummary>,
    pub fetched_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ContentSnapshot {
    pub event_id: i64,
    pub side: ContentSide,
    pub render: ContentRender,
    pub total_bytes: usize,
    pub raw: Arc<[u8]>,
}

impl NifiClient {
    /// Fetches the latest cached provenance events for a given component.
    ///
    /// Maps `GET /nifi-api/provenance-events/latest/{component_id}?limit={limit}`
    /// into a [`LatestEventsSnapshot`]. Errors are classified via
    /// `classify_or_fallback` so callers only see typed `NifiLensError` variants.
    pub async fn latest_events(
        &self,
        component_id: &str,
        limit: i32,
    ) -> Result<LatestEventsSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            component_id,
            limit,
            "fetching /provenance-events/latest",
        );

        let dto = self
            .inner
            .provenanceevents_api()
            .get_latest_provenance_events(component_id, Some(limit))
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LatestProvenanceEventsFailed {
                        context: self.context_name().to_string(),
                        component_id: component_id.to_string(),
                        source,
                    }
                })
            })?;

        let events = dto
            .provenance_events
            .unwrap_or_default()
            .into_iter()
            .map(summary_from_dto)
            .collect::<Vec<_>>();

        let component_label = events
            .first()
            .map(|e| format!("{} \u{00b7} {}", e.component_name, e.group_id))
            .unwrap_or_else(|| component_id.to_string());

        Ok(LatestEventsSnapshot {
            component_id: component_id.to_string(),
            component_label,
            events,
            fetched_at: SystemTime::now(),
        })
    }

    /// Submits a lineage query for the given flowfile UUID.
    ///
    /// Maps `POST /nifi-api/provenance/lineage` with a `FLOWFILE` request body
    /// and returns the opaque `query_id` string needed to poll or delete the
    /// query later. Errors are classified via `classify_or_fallback`.
    pub async fn submit_lineage(&self, flow_file_uuid: &str) -> Result<String, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            flow_file_uuid,
            "submitting lineage query",
        );

        let mut request = LineageRequestDto::default();
        request.lineage_request_type = Some("FLOWFILE".to_string());
        request.uuid = Some(flow_file_uuid.to_string());

        let mut lineage_dto = LineageDto::default();
        lineage_dto.request = Some(request);

        let body = LineageEntity {
            lineage: Some(lineage_dto),
        };

        let dto = self
            .inner
            .provenance_api()
            .submit_lineage_request(&body)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQuerySubmitFailed {
                        context: self.context_name().to_string(),
                        uuid: flow_file_uuid.to_string(),
                        source,
                    }
                })
            })?;

        dto.id
            .ok_or_else(|| NifiLensError::LineageQuerySubmitFailed {
                context: self.context_name().to_string(),
                uuid: flow_file_uuid.to_string(),
                source: "server returned no query id".into(),
            })
    }

    /// Polls a lineage query and returns [`LineagePoll::Running`] or
    /// [`LineagePoll::Finished`].
    ///
    /// Maps `GET /nifi-api/provenance/lineage/{id}`. Errors are classified via
    /// `classify_or_fallback`.
    pub async fn poll_lineage(&self, query_id: &str) -> Result<LineagePoll, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id,
            "polling lineage query",
        );

        let dto = self
            .inner
            .provenance_api()
            .get_lineage(query_id, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQueryPollFailed {
                        context: self.context_name().to_string(),
                        query_id: query_id.to_string(),
                        source,
                    }
                })
            })?;

        let percent = dto.percent_completed.unwrap_or(0).clamp(0, 100) as u8;
        let finished = dto.finished.unwrap_or(false);

        if finished {
            let nodes = dto.results.and_then(|r| r.nodes).unwrap_or_default();
            let events = nodes_to_events(nodes);
            Ok(LineagePoll::Finished(LineageSnapshot {
                events,
                percent_completed: percent,
                finished: true,
            }))
        } else {
            Ok(LineagePoll::Running { percent })
        }
    }

    /// Deletes a lineage query from the NiFi server.
    ///
    /// Maps `DELETE /nifi-api/provenance/lineage/{id}`. Errors are classified
    /// via `classify_or_fallback`. Delete failures are typically logged at warn
    /// level and never surfaced to the user.
    pub async fn delete_lineage(&self, query_id: &str) -> Result<(), NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id,
            "deleting lineage query",
        );

        self.inner
            .provenance_api()
            .delete_lineage(query_id, None)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQueryDeleteFailed {
                        context: self.context_name().to_string(),
                        query_id: query_id.to_string(),
                        source,
                    }
                })
            })?;

        Ok(())
    }
}

/// Converts lineage graph nodes into a chronological list of event summaries.
///
/// Filters to nodes whose `type` field equals `"EVENT"`, sorts ascending by
/// `millis`, and maps each to a [`ProvenanceEventSummary`].
pub(crate) fn nodes_to_events(
    nodes: Vec<nifi_rust_client::dynamic::types::ProvenanceNodeDto>,
) -> Vec<ProvenanceEventSummary> {
    let mut events: Vec<_> = nodes
        .into_iter()
        .filter(|n| n.r#type.as_deref() == Some("EVENT"))
        .collect();
    events.sort_by_key(|n| n.millis.unwrap_or(0));
    events
        .into_iter()
        .map(|n| ProvenanceEventSummary {
            event_id: 0,
            event_time_iso: n.timestamp.unwrap_or_default(),
            event_type: n.event_type.unwrap_or_default(),
            component_id: n.id.unwrap_or_default(),
            component_name: String::new(),
            component_type: n.component_type.unwrap_or_default(),
            group_id: String::new(),
            flow_file_uuid: n.flow_file_uuid.unwrap_or_default(),
            relationship: None,
            details: None,
        })
        .collect()
}

pub(crate) fn summary_from_dto(
    dto: nifi_rust_client::dynamic::types::ProvenanceEventDto,
) -> ProvenanceEventSummary {
    ProvenanceEventSummary {
        event_id: dto.event_id.unwrap_or(0),
        event_time_iso: dto.event_time.unwrap_or_default(),
        event_type: dto.event_type.unwrap_or_default(),
        component_id: dto.component_id.unwrap_or_default(),
        component_name: dto.component_name.unwrap_or_default(),
        component_type: dto.component_type.unwrap_or_default(),
        group_id: dto.group_id.unwrap_or_default(),
        flow_file_uuid: dto.flow_file_uuid.unwrap_or_default(),
        relationship: dto.relationship,
        details: dto.details,
    }
}
