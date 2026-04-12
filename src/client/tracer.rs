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
