//! Pure state for the Events tab.

use crate::client::ProvenanceEventSummary;
use std::time::SystemTime;

/// Which filter field is currently being edited, if any.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilterField {
    /// `t` — time range (e.g. `"last 15m"`, `"last 1h"`, ISO-8601 range).
    Time,
    /// `T` — event type list (comma-separated, e.g. `"DROP,EXPIRE"`).
    Types,
    /// `s` — source component (id or display name).
    Source,
    /// `u` — flowfile UUID.
    Uuid,
    /// `a` — attribute filter (`key=value`).
    Attr,
}

impl FilterField {
    pub fn key(self) -> char {
        match self {
            Self::Time => 't',
            Self::Types => 'T',
            Self::Source => 's',
            Self::Uuid => 'u',
            Self::Attr => 'a',
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Time => "time",
            Self::Types => "type",
            Self::Source => "source",
            Self::Uuid => "file uuid",
            Self::Attr => "attr",
        }
    }
}

/// Filter state for a provenance query. Empty strings mean "no filter"
/// (treated as server-side wildcard / default).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventsFilters {
    pub time: String,
    pub types: String,
    pub source: String,
    pub uuid: String,
    pub attr: String,
}

impl Default for EventsFilters {
    fn default() -> Self {
        Self {
            time: "last 15m".to_string(),
            types: String::new(),
            source: String::new(),
            uuid: String::new(),
            attr: String::new(),
        }
    }
}

impl EventsFilters {
    /// Immutable field accessor matching `FilterField`.
    pub fn get(&self, field: FilterField) -> &str {
        match field {
            FilterField::Time => &self.time,
            FilterField::Types => &self.types,
            FilterField::Source => &self.source,
            FilterField::Uuid => &self.uuid,
            FilterField::Attr => &self.attr,
        }
    }

    /// Mutable field accessor.
    pub fn get_mut(&mut self, field: FilterField) -> &mut String {
        match field {
            FilterField::Time => &mut self.time,
            FilterField::Types => &mut self.types,
            FilterField::Source => &mut self.source,
            FilterField::Uuid => &mut self.uuid,
            FilterField::Attr => &mut self.attr,
        }
    }
}

/// Status of the current provenance query.
#[derive(Debug, Clone)]
pub enum EventsQueryStatus {
    /// No query has run yet, or results have been cleared.
    Idle,
    /// Query is in flight. `submitted_at` is wall-clock.
    Running {
        query_id: Option<String>,
        submitted_at: SystemTime,
        percent: u8,
    },
    /// Query completed successfully.
    Done {
        fetched_at: SystemTime,
        truncated: bool,
        took_ms: u64,
    },
    /// Query failed. The error message is shown in the banner and
    /// as a `status ● failed` chip in the filter bar.
    Failed { error: String },
}

/// Full state for the Events tab.
#[derive(Debug)]
pub struct EventsState {
    pub filters: EventsFilters,
    pub status: EventsQueryStatus,
    /// Most recent query's results. Cleared on `n` (new query).
    pub events: Vec<ProvenanceEventSummary>,
    /// Selected row in the results list, or `None` when no row is
    /// selected (filter-bar focus, Mode A).
    pub selected_row: Option<usize>,
    /// Max results cap. Default 500; `L` raises to 5000.
    pub cap: u32,
    /// When `Some`, the user is editing one of the filter fields.
    /// The wrapped `String` is the in-progress text buffer; the
    /// corresponding `EventsFilters` field is updated live on each
    /// keystroke for render feedback.
    pub filter_edit: Option<(FilterField, String)>,
    /// Snapshot of the filter value captured on `enter_filter_edit`,
    /// so `cancel_filter_edit` can restore it.
    pub pre_edit_value: Option<String>,
}

impl EventsState {
    pub fn new() -> Self {
        Self {
            filters: EventsFilters::default(),
            status: EventsQueryStatus::Idle,
            events: Vec::new(),
            selected_row: None,
            cap: 500,
            filter_edit: None,
            pre_edit_value: None,
        }
    }
}

impl Default for EventsState {
    fn default() -> Self {
        Self::new()
    }
}

/// Reducer: fold an [`EventsPayload`](crate::event::EventsPayload) into state.
pub fn apply_payload(state: &mut EventsState, payload: crate::event::EventsPayload) {
    use crate::event::EventsPayload;

    match payload {
        EventsPayload::QueryStarted { query_id } => {
            state.status = EventsQueryStatus::Running {
                query_id: Some(query_id),
                submitted_at: SystemTime::now(),
                percent: 0,
            };
            state.events.clear();
            state.selected_row = None;
        }
        EventsPayload::QueryProgress { query_id, percent } => {
            // Only update if the id matches the in-flight query.
            if let EventsQueryStatus::Running {
                query_id: current,
                percent: p,
                ..
            } = &mut state.status
                && current.as_deref() == Some(query_id.as_str())
            {
                *p = percent;
            }
        }
        EventsPayload::QueryDone {
            query_id,
            events,
            fetched_at,
            truncated,
        } => {
            // Only apply if the id matches. Late payloads from cancelled
            // queries are silently dropped.
            let matches = matches!(
                &state.status,
                EventsQueryStatus::Running { query_id: Some(current), .. }
                    if current.as_str() == query_id.as_str()
            );
            if !matches {
                return;
            }
            let took_ms = match &state.status {
                EventsQueryStatus::Running { submitted_at, .. } => fetched_at
                    .duration_since(*submitted_at)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0),
                _ => 0,
            };
            state.events = events;
            state.status = EventsQueryStatus::Done {
                fetched_at,
                truncated,
                took_ms,
            };
            state.selected_row = None;
        }
        EventsPayload::QueryFailed { query_id, error } => {
            // Only apply if the id matches the in-flight query, or if
            // we're still Running with no id yet (startup race).
            let matches = match (&state.status, query_id.as_deref()) {
                (
                    EventsQueryStatus::Running {
                        query_id: current, ..
                    },
                    Some(id),
                ) => current.as_deref() == Some(id),
                (EventsQueryStatus::Running { .. }, None) => true,
                _ => false,
            };
            if !matches {
                return;
            }
            state.status = EventsQueryStatus::Failed { error };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_state_new_is_idle_with_default_filters() {
        let s = EventsState::new();
        assert!(matches!(s.status, EventsQueryStatus::Idle));
        assert_eq!(s.filters, EventsFilters::default());
        assert!(s.events.is_empty());
        assert_eq!(s.selected_row, None);
        assert_eq!(s.cap, 500);
        assert!(s.filter_edit.is_none());
    }

    #[test]
    fn apply_query_started_transitions_to_running() {
        use crate::event::EventsPayload;
        let mut s = EventsState::new();
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            },
        );
        match &s.status {
            EventsQueryStatus::Running {
                query_id, percent, ..
            } => {
                assert_eq!(query_id.as_deref(), Some("q-1"));
                assert_eq!(*percent, 0);
            }
            other => panic!("expected Running, got {other:?}"),
        }
    }

    #[test]
    fn apply_query_progress_updates_percent() {
        use crate::event::EventsPayload;
        let mut s = EventsState::new();
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryProgress {
                query_id: "q-1".into(),
                percent: 45,
            },
        );
        match &s.status {
            EventsQueryStatus::Running { percent, .. } => assert_eq!(*percent, 45),
            other => panic!("expected Running, got {other:?}"),
        }
    }

    #[test]
    fn apply_query_done_transitions_to_done_and_stores_events() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            },
        );
        let sample = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "proc-1".into(),
            component_name: "ControlRate".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g-1".into(),
            flow_file_uuid: "abc-123".into(),
            relationship: Some("failure".into()),
            details: None,
        };
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q-1".into(),
                events: vec![sample.clone()],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        assert!(matches!(
            s.status,
            EventsQueryStatus::Done {
                truncated: false,
                ..
            }
        ));
        assert_eq!(s.events.len(), 1);
        assert_eq!(s.events[0].event_id, 1);
    }

    #[test]
    fn apply_query_failed_transitions_to_failed_and_preserves_filters() {
        use crate::event::EventsPayload;
        let mut s = EventsState::new();
        s.filters.source = "proc-7".into();
        // Need to be in Running state for QueryFailed to apply — reducer
        // drops mismatched payloads.
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryFailed {
                query_id: Some("q-1".into()),
                error: "timeout".into(),
            },
        );
        match &s.status {
            EventsQueryStatus::Failed { error } => assert_eq!(error, "timeout"),
            other => panic!("expected Failed, got {other:?}"),
        }
        assert_eq!(s.filters.source, "proc-7");
    }

    #[test]
    fn apply_query_done_sets_selected_row_to_none_when_results_empty() {
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q-1".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q-1".into(),
                events: vec![],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        assert_eq!(s.selected_row, None);
    }

    #[test]
    fn filter_field_labels_match_spec() {
        assert_eq!(FilterField::Time.label(), "time");
        assert_eq!(FilterField::Types.label(), "type");
        assert_eq!(FilterField::Source.label(), "source");
        assert_eq!(FilterField::Uuid.label(), "file uuid");
        assert_eq!(FilterField::Attr.label(), "attr");
    }

    #[test]
    fn filter_field_keys_match_spec() {
        assert_eq!(FilterField::Time.key(), 't');
        assert_eq!(FilterField::Types.key(), 'T');
        assert_eq!(FilterField::Source.key(), 's');
        assert_eq!(FilterField::Uuid.key(), 'u');
        assert_eq!(FilterField::Attr.key(), 'a');
    }

    #[test]
    fn events_filters_default_is_last_15m() {
        let f = EventsFilters::default();
        assert_eq!(f.time, "last 15m");
        assert!(f.types.is_empty());
        assert!(f.source.is_empty());
        assert!(f.uuid.is_empty());
        assert!(f.attr.is_empty());
    }

    #[test]
    fn events_filters_get_and_get_mut_round_trip() {
        let mut f = EventsFilters::default();
        *f.get_mut(FilterField::Source) = "proc-1".into();
        assert_eq!(f.get(FilterField::Source), "proc-1");
    }
}
