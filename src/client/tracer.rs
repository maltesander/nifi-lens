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

use nifi_rust_client::dynamic::traits::ProvenanceEventsApi as _;

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
