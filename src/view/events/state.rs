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

impl EventsState {
    /// Enter filter-edit mode for the given field. Captures the
    /// current field value into `pre_edit_value` so `cancel_filter_edit`
    /// can restore it. If another field was being edited, its current
    /// buffer is committed first (same as pressing Enter on it).
    pub fn enter_filter_edit(&mut self, field: FilterField) {
        if self.filter_edit.is_some() {
            self.commit_filter_edit();
        }
        let current = self.filters.get(field).to_string();
        self.pre_edit_value = Some(current.clone());
        self.filter_edit = Some((field, current));
    }

    /// Append a character to the active filter-edit buffer.
    /// No-op if no field is being edited.
    pub fn push_filter_char(&mut self, ch: char) {
        if let Some((field, buf)) = self.filter_edit.as_mut() {
            buf.push(ch);
            let new_buf = buf.clone();
            let field_copy = *field;
            *self.filters.get_mut(field_copy) = new_buf;
        }
    }

    /// Pop the last character from the active filter-edit buffer.
    /// No-op if no field is being edited or the buffer is empty.
    pub fn pop_filter_char(&mut self) {
        if let Some((field, buf)) = self.filter_edit.as_mut() {
            buf.pop();
            let new_buf = buf.clone();
            let field_copy = *field;
            *self.filters.get_mut(field_copy) = new_buf;
        }
    }

    /// Commit the active filter-edit: leaves `filters.<field>` with
    /// the live buffer value and clears edit state.
    pub fn commit_filter_edit(&mut self) {
        self.filter_edit = None;
        self.pre_edit_value = None;
    }

    /// Cancel the active filter-edit: restores the pre-edit value.
    pub fn cancel_filter_edit(&mut self) {
        if let Some((field, _)) = self.filter_edit.take() {
            let restored = self.pre_edit_value.take().unwrap_or_default();
            *self.filters.get_mut(field) = restored;
        }
    }

    /// Reset filters to defaults (`r` key). Does not touch cap, status,
    /// or results.
    pub fn reset_filters(&mut self) {
        self.filters = EventsFilters::default();
    }

    /// New query (`n` key): clears filters, results, and status back
    /// to idle. Cap resets to default.
    pub fn new_query(&mut self) {
        self.filters = EventsFilters::default();
        self.events.clear();
        self.selected_row = None;
        self.status = EventsQueryStatus::Idle;
        self.cap = 500;
        self.filter_edit = None;
        self.pre_edit_value = None;
    }

    /// Raise the cap (`L` key) from 500 to 5000. Idempotent once at 5000.
    pub fn raise_cap(&mut self) {
        if self.cap < 5000 {
            self.cap = 5000;
        }
    }

    /// Enter Mode B (row navigation). Does nothing if the results list
    /// is empty.
    pub fn enter_row_nav(&mut self) {
        if !self.events.is_empty() {
            self.selected_row = Some(0);
        }
    }

    /// Leave Mode B back to Mode A (filter bar). Clears selection.
    pub fn leave_row_nav(&mut self) {
        self.selected_row = None;
    }

    pub fn move_selection_down(&mut self) {
        if let Some(idx) = self.selected_row {
            let max = self.events.len().saturating_sub(1);
            self.selected_row = Some((idx + 1).min(max));
        }
    }

    pub fn move_selection_up(&mut self) {
        if let Some(idx) = self.selected_row {
            self.selected_row = Some(idx.saturating_sub(1));
        }
    }

    /// Accessor for the currently-selected event, if any.
    pub fn selected_event(&self) -> Option<&crate::client::ProvenanceEventSummary> {
        self.selected_row.and_then(|i| self.events.get(i))
    }

    /// Build a [`ProvenanceQuery`](crate::client::ProvenanceQuery) from the
    /// current filter state. Projects `filters.time` into a NiFi-native
    /// `MM/dd/yyyy HH:mm:ss` start date (best-effort — empty /
    /// unparseable input falls back to no start filter). The time is
    /// computed in the local timezone so the server interprets it in
    /// whatever zone both sides share (typical for dev fixtures);
    /// falls back to UTC if the local offset is indeterminate.
    pub fn build_query(&self) -> crate::client::ProvenanceQuery {
        use time::OffsetDateTime;
        use time::macros::format_description;

        let nifi_fmt = format_description!("[month]/[day]/[year] [hour]:[minute]:[second]");
        let now = OffsetDateTime::now_local().unwrap_or_else(|_| OffsetDateTime::now_utc());
        let start = parse_time_window(&self.filters.time)
            .and_then(|d| now.checked_sub(d))
            .and_then(|dt| dt.format(&nifi_fmt).ok());

        crate::client::ProvenanceQuery {
            component_id: if self.filters.source.is_empty() {
                None
            } else {
                Some(self.filters.source.clone())
            },
            flow_file_uuid: if self.filters.uuid.is_empty() {
                None
            } else {
                Some(self.filters.uuid.clone())
            },
            start_time_iso: start,
            end_time_iso: None,
            max_results: self.cap,
        }
    }
}

/// Parse a `"last <N><unit>"` window string into a `time::Duration`.
///
/// Supported units: `m` (minutes), `h` (hours), `d` (days). Returns
/// `None` on unparseable input. Empty / whitespace strings return
/// `None` — the caller treats that as "no start filter".
fn parse_time_window(s: &str) -> Option<time::Duration> {
    let s = s.trim();
    let rest = s.strip_prefix("last ")?;
    // Last char is the unit; everything before is the number.
    let unit_char = rest.chars().last()?;
    let num_str = &rest[..rest.len() - unit_char.len_utf8()];
    let n: i64 = num_str.trim().parse().ok()?;
    match unit_char {
        'm' => Some(time::Duration::minutes(n)),
        'h' => Some(time::Duration::hours(n)),
        'd' => Some(time::Duration::days(n)),
        _ => None,
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

    #[test]
    fn enter_filter_edit_captures_pre_edit_value() {
        let mut s = EventsState::new();
        s.filters.source = "old".into();
        s.enter_filter_edit(FilterField::Source);
        assert_eq!(s.filter_edit, Some((FilterField::Source, "old".into())));
        assert_eq!(s.pre_edit_value.as_deref(), Some("old"));
    }

    #[test]
    fn push_filter_char_appends_to_buffer_and_live_updates_field() {
        let mut s = EventsState::new();
        s.enter_filter_edit(FilterField::Source);
        s.push_filter_char('a');
        s.push_filter_char('b');
        s.push_filter_char('c');
        assert_eq!(s.filter_edit.as_ref().unwrap().1, "abc");
        assert_eq!(s.filters.source, "abc");
    }

    #[test]
    fn pop_filter_char_removes_last_and_live_updates_field() {
        let mut s = EventsState::new();
        s.enter_filter_edit(FilterField::Source);
        s.push_filter_char('a');
        s.push_filter_char('b');
        s.pop_filter_char();
        assert_eq!(s.filter_edit.as_ref().unwrap().1, "a");
        assert_eq!(s.filters.source, "a");
    }

    #[test]
    fn commit_filter_edit_clears_edit_state_and_keeps_value() {
        let mut s = EventsState::new();
        s.enter_filter_edit(FilterField::Source);
        s.push_filter_char('x');
        s.commit_filter_edit();
        assert!(s.filter_edit.is_none());
        assert!(s.pre_edit_value.is_none());
        assert_eq!(s.filters.source, "x");
    }

    #[test]
    fn cancel_filter_edit_restores_pre_edit_value() {
        let mut s = EventsState::new();
        s.filters.source = "old".into();
        s.enter_filter_edit(FilterField::Source);
        s.push_filter_char('X');
        assert_eq!(s.filters.source, "oldX");
        s.cancel_filter_edit();
        assert!(s.filter_edit.is_none());
        assert_eq!(s.filters.source, "old");
    }

    #[test]
    fn enter_filter_edit_replaces_existing_edit() {
        let mut s = EventsState::new();
        s.enter_filter_edit(FilterField::Source);
        s.push_filter_char('a');
        // Switching to a different field without commit commits the
        // current one and starts the new one fresh.
        s.enter_filter_edit(FilterField::Uuid);
        assert_eq!(s.filter_edit.as_ref().unwrap().0, FilterField::Uuid);
        // Source retains its edit.
        assert_eq!(s.filters.source, "a");
    }

    #[test]
    fn reset_filters_restores_defaults_only() {
        let mut s = EventsState::new();
        s.filters.source = "proc-1".into();
        s.filters.uuid = "abc".into();
        s.cap = 5000;
        s.reset_filters();
        assert_eq!(s.filters, EventsFilters::default());
        // cap is NOT reset by `r` — only filters. Cap is changed by `L`
        // and by explicit interactions.
        assert_eq!(s.cap, 5000);
    }

    #[test]
    fn new_query_clears_filters_results_and_status() {
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        s.filters.source = "proc-1".into();
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        s.new_query();
        assert_eq!(s.filters, EventsFilters::default());
        assert!(matches!(s.status, EventsQueryStatus::Idle));
        assert!(s.events.is_empty());
        assert_eq!(s.selected_row, None);
    }

    #[test]
    fn raise_cap_toggles_between_500_and_5000() {
        let mut s = EventsState::new();
        assert_eq!(s.cap, 500);
        s.raise_cap();
        assert_eq!(s.cap, 5000);
        // Pressing L again clamps to 5000 (no further raise).
        s.raise_cap();
        assert_eq!(s.cap, 5000);
    }

    #[test]
    fn enter_row_nav_from_empty_results_is_noop() {
        let mut s = EventsState::new();
        s.enter_row_nav();
        assert_eq!(s.selected_row, None);
    }

    #[test]
    fn enter_row_nav_with_results_selects_first_row() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: "u".into(),
            relationship: None,
            details: None,
        };
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e.clone(), e.clone(), e],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        s.enter_row_nav();
        assert_eq!(s.selected_row, Some(0));
    }

    #[test]
    fn move_selection_down_and_up_cycles_within_results() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: "u".into(),
            relationship: None,
            details: None,
        };
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e.clone(), e.clone(), e],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        s.enter_row_nav();
        assert_eq!(s.selected_row, Some(0));
        s.move_selection_down();
        assert_eq!(s.selected_row, Some(1));
        s.move_selection_down();
        assert_eq!(s.selected_row, Some(2));
        s.move_selection_down();
        // Saturates at end.
        assert_eq!(s.selected_row, Some(2));
        s.move_selection_up();
        assert_eq!(s.selected_row, Some(1));
        s.move_selection_up();
        s.move_selection_up();
        // Saturates at start.
        assert_eq!(s.selected_row, Some(0));
    }

    #[test]
    fn parse_time_window_supports_minutes_hours_days() {
        assert_eq!(
            parse_time_window("last 15m"),
            Some(time::Duration::minutes(15))
        );
        assert_eq!(parse_time_window("last 2h"), Some(time::Duration::hours(2)));
        assert_eq!(parse_time_window("last 7d"), Some(time::Duration::days(7)));
    }

    #[test]
    fn parse_time_window_returns_none_on_junk() {
        assert_eq!(parse_time_window(""), None);
        assert_eq!(parse_time_window("whenever"), None);
        assert_eq!(parse_time_window("last xx"), None);
    }

    #[test]
    fn build_query_respects_filters_and_cap() {
        let mut s = EventsState::new();
        s.filters.source = "proc-1".into();
        s.cap = 1000;
        let q = s.build_query();
        assert_eq!(q.component_id.as_deref(), Some("proc-1"));
        assert_eq!(q.flow_file_uuid, None);
        assert_eq!(q.max_results, 1000);
        assert!(
            q.start_time_iso.is_some(),
            "last 15m default should parse into a start date"
        );
    }

    #[test]
    fn leave_row_nav_clears_selection() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::EventsPayload;
        use std::time::SystemTime;
        let mut s = EventsState::new();
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: "u".into(),
            relationship: None,
            details: None,
        };
        apply_payload(
            &mut s,
            EventsPayload::QueryStarted {
                query_id: "q".into(),
            },
        );
        apply_payload(
            &mut s,
            EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            },
        );
        s.enter_row_nav();
        s.leave_row_nav();
        assert_eq!(s.selected_row, None);
    }
}
