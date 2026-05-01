//! Pure state for the Events tab.

use crate::client::{AttributeTriple, Predicate, ProvenanceEventSummary, ProvenanceQuery};
use std::collections::VecDeque;
use std::time::{Duration, SystemTime};

/// Default provenance query result cap.
pub(crate) const DEFAULT_RESULT_CAP: u32 = 500;

/// Expanded provenance query result cap, used after the `L` hotkey.
pub(crate) const EXPANDED_RESULT_CAP: u32 = 5000;

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
    /// Max results cap. Starts at `DEFAULT_RESULT_CAP`; `L` raises to `EXPANDED_RESULT_CAP`.
    pub cap: u32,
    /// When `Some`, the user is editing one of the filter fields.
    /// The wrapped `String` is the in-progress text buffer; the
    /// corresponding `EventsFilters` field is updated live on each
    /// keystroke for render feedback.
    pub filter_edit: Option<(FilterField, String)>,
    /// Snapshot of the filter value captured on `enter_filter_edit`,
    /// so `cancel_filter_edit` can restore it.
    pub pre_edit_value: Option<String>,
    /// Discriminator for one-shot vs watch sub-mode. The legacy
    /// per-query fields (`events`, `status`, `selected_row`, `cap`,
    /// `filter_edit`, `pre_edit_value`) live alongside this — they
    /// are inactive in `Watch` mode but not duplicated into the
    /// variant. Watch-only state lives on `WatchSession` inside
    /// `Watch(_)`.
    pub mode: EventsMode,
    /// True iff focus is in the watch-strip predicate input.
    /// One-shot mode keeps this `false`. Tasks beyond 16 may flip it
    /// programmatically (e.g., the cross-link arm focusing on entry).
    pub predicate_focus: bool,
    /// True iff a `discard N watched events?` confirm modal is visible.
    /// Set by `request_exit_watch` when the buffer is non-empty;
    /// cleared by `confirm_exit_watch` / `cancel_exit_watch`.
    pub exit_watch_pending: bool,
}

impl EventsState {
    pub fn new() -> Self {
        Self {
            filters: EventsFilters::default(),
            status: EventsQueryStatus::Idle,
            events: Vec::new(),
            selected_row: None,
            cap: DEFAULT_RESULT_CAP,
            filter_edit: None,
            pre_edit_value: None,
            mode: EventsMode::OneShot,
            predicate_focus: false,
            exit_watch_pending: false,
        }
    }

    /// If the current query status is `Failed { .. }`, reset it to
    /// `Idle`. Called by the app-level reducer when the user navigates
    /// away from the Events tab so returning to the tab shows a clean
    /// slate instead of a stale error.
    pub fn clear_failed_status(&mut self) {
        if matches!(self.status, EventsQueryStatus::Failed { .. }) {
            self.status = EventsQueryStatus::Idle;
        }
    }

    /// Replace the current mode with `Watch(session)`. Existing
    /// one-shot fields are left untouched — the user can return to
    /// them by exiting watch mode.
    pub fn enter_watch_mode(&mut self, session: WatchSession) {
        self.mode = EventsMode::Watch(session);
    }

    /// Drop the watch session (and its buffer) and return to
    /// `OneShot`. Caller is responsible for aborting the worker
    /// before calling this.
    pub fn exit_watch_mode(&mut self) {
        self.mode = EventsMode::OneShot;
    }

    /// Borrow the active [`WatchSession`], if any.
    pub fn watch(&self) -> Option<&WatchSession> {
        match &self.mode {
            EventsMode::Watch(s) => Some(s),
            EventsMode::OneShot => None,
        }
    }

    /// Mutable borrow of the active [`WatchSession`], if any.
    pub fn watch_mut(&mut self) -> Option<&mut WatchSession> {
        match &mut self.mode {
            EventsMode::Watch(s) => Some(s),
            EventsMode::OneShot => None,
        }
    }

    /// True iff focus is currently in the watch-strip predicate input.
    pub fn predicate_input_focused(&self) -> bool {
        self.predicate_focus
    }

    /// Move focus to the watch-strip predicate input. No-op when the
    /// tab isn't in watch mode.
    pub fn focus_predicate(&mut self) {
        if matches!(self.mode, EventsMode::Watch(_)) {
            self.predicate_focus = true;
        }
    }

    /// Drop predicate-input focus (returns row navigation).
    pub fn unfocus_predicate(&mut self) {
        self.predicate_focus = false;
    }

    /// Append a character to the predicate-input buffer. No-op when
    /// focus is elsewhere.
    pub fn push_predicate_char(&mut self, ch: char) {
        if !self.predicate_focus {
            return;
        }
        if let Some(w) = self.watch_mut() {
            w.predicate_input.push(ch);
        }
    }

    /// Pop the last character from the predicate-input buffer. No-op
    /// when focus is elsewhere or the buffer is empty.
    pub fn pop_predicate_char(&mut self) {
        if !self.predicate_focus {
            return;
        }
        if let Some(w) = self.watch_mut() {
            w.predicate_input.pop();
        }
    }

    /// Parse `predicate_input` into the active predicate. On parse
    /// error, returns the error and leaves the previous predicate in
    /// place. On success, the predicate replaces the active one;
    /// existing buffer rows are NOT re-filtered (forward-only).
    pub fn commit_predicate(&mut self) -> Result<(), crate::client::PredicateParseError> {
        let Some(w) = self.watch_mut() else {
            return Ok(());
        };
        let parsed = crate::client::Predicate::parse(&w.predicate_input)?;
        w.predicate = parsed;
        Ok(())
    }

    /// User asked to leave watch mode. If the buffer is empty (or the
    /// tab is not in watch mode), exit immediately and return `false`
    /// to indicate no confirmation was needed. If the buffer has any
    /// matched events, arm the confirm modal and return `true` so the
    /// caller can render it.
    pub fn request_exit_watch(&mut self) -> bool {
        let needs_confirm = match self.watch() {
            Some(w) => !w.buffer.is_empty(),
            None => false,
        };
        if !needs_confirm {
            self.exit_watch_mode();
            self.exit_watch_pending = false;
            return false;
        }
        self.exit_watch_pending = true;
        true
    }

    /// User answered `y` — drop the buffer and return to one-shot.
    pub fn confirm_exit_watch(&mut self) {
        self.exit_watch_pending = false;
        self.exit_watch_mode();
    }

    /// User answered `n` / Esc — keep the session, disarm the modal.
    pub fn cancel_exit_watch(&mut self) {
        self.exit_watch_pending = false;
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

    /// Returns the current filter-edit buffer value, or `None` when no field
    /// is being edited.
    pub fn current_filter_value(&self) -> Option<&str> {
        self.filter_edit.as_ref().map(|(_, s)| s.as_str())
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
        self.cap = DEFAULT_RESULT_CAP;
        self.filter_edit = None;
        self.pre_edit_value = None;
    }

    /// Raise the cap (`L` key) from `DEFAULT_RESULT_CAP` to `EXPANDED_RESULT_CAP`. Idempotent once at `EXPANDED_RESULT_CAP`.
    pub fn raise_cap(&mut self) {
        if self.cap < EXPANDED_RESULT_CAP {
            self.cap = EXPANDED_RESULT_CAP;
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
    /// `MM/dd/yyyy HH:mm:ss UTC` start date (best-effort — empty /
    /// unparseable input falls back to no start filter).
    ///
    /// NiFi 2.x rejects start/end dates without a named timezone suffix
    /// with `400 "Message body is malformed"`, and it parses the suffix
    /// via `java.util.TimeZone.getTimeZone`, which accepts `UTC`,
    /// `GMT`, or any IANA zone name. We always emit UTC because the
    /// window math is computed in UTC internally — converting to local
    /// would require re-embedding a timezone the server also has to
    /// recognize.
    pub fn build_query(&self) -> crate::client::ProvenanceQuery {
        use time::OffsetDateTime;
        use time::macros::format_description;

        let nifi_fmt = format_description!("[month]/[day]/[year] [hour]:[minute]:[second] UTC");
        let now = OffsetDateTime::now_utc();
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
            event_types: Vec::new(),
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
        // Watch-mode payloads — delegated to the watch reducer in
        // `crate::view::events::handle_watch_payload`.
        EventsPayload::WatchMatch { .. }
        | EventsPayload::WatchTick { .. }
        | EventsPayload::WatchFailed { .. } => {
            let selected = state.selected_row;
            crate::view::events::handle_watch_payload(state, payload, selected);
        }
    }
}

/// One event that matched the active watch predicate, with its full
/// attribute map captured at fetch time.
#[derive(Debug, Clone)]
pub struct MatchedEvent {
    pub summary: ProvenanceEventSummary,
    pub attrs: Vec<AttributeTriple>,
}

/// Cursor advanced after each tail iteration so the next poll only
/// returns events strictly newer than what we've already scanned.
#[derive(Debug, Clone, Copy)]
pub struct TailCursor {
    pub last_event_id: i64,
    pub last_event_time: SystemTime,
}

/// Status of the watch worker, surfaced as a chip on the Watch strip.
#[derive(Debug, Clone)]
pub enum WatchStatus {
    Tailing,
    Paused,
    NarrowRequired,
    Waiting,
    Failed { error: String, retry_in: Duration },
}

/// Rolling stats for the Watch strip.
#[derive(Debug, Clone, Default)]
pub struct WatchStats {
    pub events_per_sec_ewma: f32,
    pub last_poll_latency: Option<Duration>,
    pub trimmed_total: u64,
    pub detail_fetch_errors: u64,
}

/// Mode discriminator for the Events tab. `OneShot` is the historical
/// behaviour — submit a query, await results, render. `Watch` enters
/// the live-tail sub-mode and carries a [`WatchSession`].
///
/// The size disparity between `OneShot` (zero-sized) and
/// `Watch(WatchSession)` (~320 B) is acceptable: there is exactly one
/// `EventsMode` per `EventsState`, and `EventsState` itself lives in
/// a single owned slot on `AppState` — boxing would add a heap hop
/// for every watch-mode access without saving meaningful memory.
#[allow(clippy::large_enum_variant)]
#[derive(Debug)]
pub enum EventsMode {
    OneShot,
    Watch(WatchSession),
}

/// Full state for one active watch — predicate, narrow, rolling
/// buffer, cursor, status. Lives on `EventsState::mode` when the
/// tab is in watch mode (mode wrapping arrives in Task 9).
#[derive(Debug)]
pub struct WatchSession {
    pub narrow: ProvenanceQuery,
    pub predicate: Predicate,
    pub predicate_input: String,
    pub buffer: VecDeque<MatchedEvent>,
    pub buffer_cap: usize,
    pub cursor: Option<TailCursor>,
    pub status: WatchStatus,
    pub stats: WatchStats,
}

impl WatchSession {
    /// Cost guard: refuse to spawn the worker unless the narrow has
    /// at least one of (component, flow-file UUID, non-empty event
    /// types, non-blank start time). `now` is injected to keep the
    /// function pure for tests and to leave room for a future
    /// "start_time within 24h" gate (the spec mentions 24h as a UX
    /// hint; the runtime gate accepts any non-blank start_time as
    /// deliberate).
    pub fn can_start(narrow: &ProvenanceQuery, now: SystemTime) -> bool {
        if narrow.component_id.is_some() {
            return true;
        }
        if narrow.flow_file_uuid.is_some() {
            return true;
        }
        if !narrow.event_types.is_empty() {
            return true;
        }
        if let Some(s) = narrow.start_time_iso.as_deref()
            && !s.trim().is_empty()
        {
            return true;
        }
        let _ = now;
        false
    }

    /// Append a new matched event to the rolling buffer. If the buffer
    /// is at `buffer_cap`, drop the oldest — *unless* `focused_row`
    /// names index 0, in which case drop the second-oldest. This
    /// prevents the user's cursor from jumping under their finger
    /// while they investigate a row.
    ///
    /// When `buffer_cap` is 1 there is no second-oldest entry to fall
    /// back to, so the focused row is dropped normally.
    pub fn push_event(&mut self, ev: MatchedEvent, focused_row: Option<usize>) {
        if self.buffer.len() >= self.buffer_cap {
            let trim_idx = if focused_row == Some(0) && self.buffer.len() >= 2 {
                1
            } else {
                0
            };
            self.buffer.remove(trim_idx);
            self.stats.trimmed_total = self.stats.trimmed_total.saturating_add(1);
        }
        self.buffer.push_back(ev);
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
    fn build_query_start_date_has_named_timezone_suffix() {
        // NiFi 2.x rejects start/end dates without a named timezone
        // suffix with `400 "Message body is malformed"`. Pin the
        // `MM/dd/yyyy HH:mm:ss UTC` shape so a future refactor does
        // not regress this.
        let s = EventsState::new();
        let q = s.build_query();
        let start = q.start_time_iso.expect("default produces a start date");
        assert!(
            start.ends_with(" UTC"),
            "start date must end with ' UTC' — NiFi rejects bare dates, got {start:?}"
        );
        // Shape check: 19 chars of `MM/dd/yyyy HH:mm:ss` + ` UTC` = 23.
        assert_eq!(
            start.len(),
            23,
            "start date must be exactly `MM/dd/yyyy HH:mm:ss UTC`, got {start:?}"
        );
    }

    use crate::client::ProvenanceQuery;
    use std::time::{Duration, SystemTime};

    fn empty_query() -> ProvenanceQuery {
        ProvenanceQuery::default()
    }

    fn query_with_component() -> ProvenanceQuery {
        ProvenanceQuery {
            component_id: Some("abc".into()),
            ..Default::default()
        }
    }

    fn query_with_event_types() -> ProvenanceQuery {
        ProvenanceQuery {
            event_types: vec!["DROP".into()],
            ..Default::default()
        }
    }

    #[test]
    fn watch_session_can_start_requires_a_narrow() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        assert!(!WatchSession::can_start(&empty_query(), now));
    }

    #[test]
    fn watch_session_can_start_with_component() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        assert!(WatchSession::can_start(&query_with_component(), now));
    }

    #[test]
    fn watch_session_can_start_with_event_types() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        assert!(WatchSession::can_start(&query_with_event_types(), now));
    }

    #[test]
    fn watch_session_can_start_with_flow_file_uuid() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        let q = ProvenanceQuery {
            flow_file_uuid: Some("ff-1".into()),
            ..Default::default()
        };
        assert!(WatchSession::can_start(&q, now));
    }

    #[test]
    fn watch_session_can_start_with_explicit_start_time() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        let q = ProvenanceQuery {
            start_time_iso: Some("2026-04-30T22:00:00Z".into()),
            ..Default::default()
        };
        assert!(WatchSession::can_start(&q, now));
    }

    #[test]
    fn watch_session_can_start_rejects_blank_start_time() {
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_777_640_000);
        let q = ProvenanceQuery {
            start_time_iso: Some("   ".into()),
            ..Default::default()
        };
        assert!(!WatchSession::can_start(&q, now));
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

    fn matched(id: i64) -> MatchedEvent {
        MatchedEvent {
            summary: ProvenanceEventSummary {
                event_id: id,
                event_time_iso: format!("t{id}"),
                event_type: "SEND".into(),
                component_id: "c".into(),
                component_name: "n".into(),
                component_type: "U".into(),
                group_id: "g".into(),
                flow_file_uuid: format!("ff{id}"),
                relationship: None,
                details: None,
            },
            attrs: vec![],
        }
    }

    fn empty_session(buffer_cap: usize) -> WatchSession {
        WatchSession {
            narrow: ProvenanceQuery::default(),
            predicate: Predicate::default(),
            predicate_input: String::new(),
            buffer: VecDeque::new(),
            buffer_cap,
            cursor: None,
            status: WatchStatus::Paused,
            stats: WatchStats::default(),
        }
    }

    #[test]
    fn push_event_appends_when_buffer_has_room() {
        let mut s = empty_session(3);
        s.push_event(matched(1), None);
        s.push_event(matched(2), None);
        let ids: Vec<i64> = s.buffer.iter().map(|m| m.summary.event_id).collect();
        assert_eq!(ids, vec![1, 2]);
        assert_eq!(s.stats.trimmed_total, 0);
    }

    #[test]
    fn push_event_trims_oldest_when_full() {
        let mut s = empty_session(3);
        for id in 1..=4 {
            s.push_event(matched(id), None);
        }
        let ids: Vec<i64> = s.buffer.iter().map(|m| m.summary.event_id).collect();
        assert_eq!(ids, vec![2, 3, 4]);
        assert_eq!(s.stats.trimmed_total, 1);
    }

    #[test]
    fn push_event_protects_focused_oldest() {
        let mut s = empty_session(3);
        for id in 1..=3 {
            s.push_event(matched(id), None);
        }
        // Buffer is full: [1, 2, 3]; focused index 0 (event 1).
        s.push_event(matched(4), Some(0));
        let ids: Vec<i64> = s.buffer.iter().map(|m| m.summary.event_id).collect();
        // Should drop event 2 (second-oldest) instead of event 1.
        assert_eq!(ids, vec![1, 3, 4]);
        assert_eq!(s.stats.trimmed_total, 1);
    }

    #[test]
    fn push_event_unfocused_or_other_row_drops_oldest_normally() {
        let mut s = empty_session(3);
        for id in 1..=3 {
            s.push_event(matched(id), None);
        }
        // Focused index is 2 (newest) — not the oldest, so normal trim.
        s.push_event(matched(4), Some(2));
        let ids: Vec<i64> = s.buffer.iter().map(|m| m.summary.event_id).collect();
        assert_eq!(ids, vec![2, 3, 4]);
        assert_eq!(s.stats.trimmed_total, 1);
    }

    #[test]
    fn push_event_buffer_cap_one_focused_still_drops_focused() {
        // Edge case: with buffer_cap = 1, there's no second-oldest to fall
        // back to — the focused row is the only row, so trim must drop it.
        let mut s = empty_session(1);
        s.push_event(matched(1), None);
        s.push_event(matched(2), Some(0));
        let ids: Vec<i64> = s.buffer.iter().map(|m| m.summary.event_id).collect();
        assert_eq!(ids, vec![2]);
        assert_eq!(s.stats.trimmed_total, 1);
    }

    #[test]
    fn events_state_default_is_oneshot_mode() {
        let s = EventsState::new();
        assert!(matches!(s.mode, EventsMode::OneShot));
    }

    #[test]
    fn events_state_enter_watch_replaces_mode() {
        let mut s = EventsState::new();
        let session = empty_session(100);
        s.enter_watch_mode(session);
        assert!(matches!(s.mode, EventsMode::Watch(_)));
    }

    #[test]
    fn events_state_exit_watch_restores_oneshot() {
        let mut s = EventsState::new();
        s.enter_watch_mode(empty_session(100));
        s.exit_watch_mode();
        assert!(matches!(s.mode, EventsMode::OneShot));
    }

    #[test]
    fn watch_helpers_borrow_active_session() {
        let mut s = EventsState::new();
        assert!(s.watch().is_none());
        assert!(s.watch_mut().is_none());

        s.enter_watch_mode(empty_session(100));
        assert!(s.watch().is_some());

        // Mutate via watch_mut to confirm it returns the live session.
        if let Some(w) = s.watch_mut() {
            w.predicate_input = "x".into();
        }
        assert_eq!(s.watch().unwrap().predicate_input, "x");
    }

    use crate::client::AttributeTriple;
    use crate::event::EventsPayload;

    fn watch_state() -> EventsState {
        let mut s = EventsState::new();
        s.enter_watch_mode(WatchSession {
            narrow: ProvenanceQuery {
                component_id: Some("c".into()),
                ..Default::default()
            },
            predicate: Predicate::parse("filename =~ /^x/").unwrap(),
            predicate_input: "filename =~ /^x/".into(),
            buffer: VecDeque::new(),
            buffer_cap: 100,
            cursor: None,
            status: WatchStatus::Tailing,
            stats: WatchStats::default(),
        });
        s
    }

    fn summary(id: i64) -> ProvenanceEventSummary {
        ProvenanceEventSummary {
            event_id: id,
            event_time_iso: format!("t{id}"),
            event_type: "SEND".into(),
            component_id: "c".into(),
            component_name: "n".into(),
            component_type: "T".into(),
            group_id: "g".into(),
            flow_file_uuid: format!("ff{id}"),
            relationship: None,
            details: None,
        }
    }

    #[test]
    fn handle_watch_match_appends_to_buffer() {
        let mut s = watch_state();
        let attrs = vec![AttributeTriple {
            key: "filename".into(),
            previous: None,
            current: Some("xy".into()),
        }];
        crate::view::events::handle_watch_payload(
            &mut s,
            EventsPayload::WatchMatch {
                summary: summary(1),
                attrs,
            },
            None,
        );
        let watch = s.watch().expect("still in watch mode");
        assert_eq!(watch.buffer.len(), 1);
        assert_eq!(watch.buffer.front().unwrap().summary.event_id, 1);
    }

    #[test]
    fn handle_watch_tick_updates_stats() {
        let mut s = watch_state();
        crate::view::events::handle_watch_payload(
            &mut s,
            EventsPayload::WatchTick {
                events_per_sec_ewma: 12.5,
                last_poll_latency_ms: 250,
                scanned: 50,
                matched: 3,
                detail_fetch_errors: 1,
            },
            None,
        );
        let stats = &s.watch().unwrap().stats;
        assert!((stats.events_per_sec_ewma - 12.5).abs() < f32::EPSILON);
        assert_eq!(stats.last_poll_latency.unwrap().as_millis(), 250);
        assert_eq!(stats.detail_fetch_errors, 1);
    }

    #[test]
    fn handle_watch_tick_promotes_waiting_to_tailing() {
        let mut s = watch_state();
        // Force into Waiting state.
        if let Some(w) = s.watch_mut() {
            w.status = WatchStatus::Waiting;
        }
        crate::view::events::handle_watch_payload(
            &mut s,
            EventsPayload::WatchTick {
                events_per_sec_ewma: 0.0,
                last_poll_latency_ms: 100,
                scanned: 0,
                matched: 0,
                detail_fetch_errors: 0,
            },
            None,
        );
        assert!(matches!(s.watch().unwrap().status, WatchStatus::Tailing));
    }

    #[test]
    fn handle_watch_failed_flips_status() {
        let mut s = watch_state();
        crate::view::events::handle_watch_payload(
            &mut s,
            EventsPayload::WatchFailed {
                error: "boom".into(),
                retry_in_ms: 5_000,
            },
            None,
        );
        match &s.watch().unwrap().status {
            WatchStatus::Failed { error, retry_in } => {
                assert_eq!(error, "boom");
                assert_eq!(retry_in.as_millis(), 5_000);
            }
            other => panic!("expected Failed, got {other:?}"),
        }
    }

    #[test]
    fn handle_watch_payload_no_op_when_not_in_watch_mode() {
        let mut s = EventsState::new();
        // Not in watch mode — payload should be a no-op.
        crate::view::events::handle_watch_payload(
            &mut s,
            EventsPayload::WatchMatch {
                summary: summary(1),
                attrs: vec![],
            },
            None,
        );
        assert!(matches!(s.mode, EventsMode::OneShot));
        assert!(s.watch().is_none());
    }

    fn matched_with_attrs(id: i64) -> MatchedEvent {
        MatchedEvent {
            summary: ProvenanceEventSummary {
                event_id: id,
                event_time_iso: format!("t{id}"),
                event_type: "SEND".into(),
                component_id: "c".into(),
                component_name: "n".into(),
                component_type: "T".into(),
                group_id: "g".into(),
                flow_file_uuid: format!("ff{id}"),
                relationship: None,
                details: None,
            },
            attrs: vec![AttributeTriple {
                key: "filename".into(),
                previous: None,
                current: Some(format!("file-{id}.json")),
            }],
        }
    }

    #[test]
    fn pause_watch_flips_status_and_keeps_buffer() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.buffer.push_back(matched_with_attrs(1));
        session.status = WatchStatus::Tailing;
        s.enter_watch_mode(session);

        crate::view::events::pause_watch(&mut s);

        assert!(matches!(s.watch().unwrap().status, WatchStatus::Paused));
        assert_eq!(s.watch().unwrap().buffer.len(), 1);
        assert_eq!(s.watch().unwrap().buffer[0].summary.event_id, 1);
    }

    #[test]
    fn pause_watch_no_op_when_not_in_watch_mode() {
        let mut s = EventsState::new();
        crate::view::events::pause_watch(&mut s);
        assert!(matches!(s.mode, EventsMode::OneShot));
    }

    #[test]
    fn pause_watch_preserves_existing_failed_status() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.status = WatchStatus::Failed {
            error: "boom".into(),
            retry_in: std::time::Duration::from_secs(5),
        };
        s.enter_watch_mode(session);
        crate::view::events::pause_watch(&mut s);
        // Pause should override Failed → Paused, since the worker is now gone.
        assert!(matches!(s.watch().unwrap().status, WatchStatus::Paused));
    }

    #[test]
    fn resume_watch_promotes_paused_to_waiting() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.status = WatchStatus::Paused;
        s.enter_watch_mode(session);
        crate::view::events::resume_watch(&mut s);
        assert!(matches!(s.watch().unwrap().status, WatchStatus::Waiting));
    }

    #[test]
    fn resume_watch_no_op_when_already_tailing() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.status = WatchStatus::Tailing;
        s.enter_watch_mode(session);
        crate::view::events::resume_watch(&mut s);
        // Tailing should not be touched by resume.
        assert!(matches!(s.watch().unwrap().status, WatchStatus::Tailing));
    }

    #[test]
    fn predicate_focus_lifecycle() {
        let mut s = EventsState::new();
        s.enter_watch_mode(empty_session(100));
        assert!(!s.predicate_input_focused());
        s.focus_predicate();
        assert!(s.predicate_input_focused());
        s.push_predicate_char('f');
        s.push_predicate_char('=');
        assert_eq!(s.watch().unwrap().predicate_input, "f=");
        s.unfocus_predicate();
        assert!(!s.predicate_input_focused());
    }

    #[test]
    fn focus_predicate_no_op_in_oneshot_mode() {
        let mut s = EventsState::new();
        s.focus_predicate();
        assert!(!s.predicate_input_focused());
    }

    #[test]
    fn push_predicate_char_no_op_when_unfocused() {
        let mut s = EventsState::new();
        s.enter_watch_mode(empty_session(100));
        // Not focused.
        s.push_predicate_char('x');
        assert_eq!(s.watch().unwrap().predicate_input, "");
    }

    #[test]
    fn pop_predicate_char_removes_last() {
        let mut s = EventsState::new();
        s.enter_watch_mode(empty_session(100));
        s.focus_predicate();
        s.push_predicate_char('a');
        s.push_predicate_char('b');
        s.pop_predicate_char();
        assert_eq!(s.watch().unwrap().predicate_input, "a");
    }

    #[test]
    fn commit_predicate_replaces_active_predicate() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.predicate_input = "filename =~ /^x/".into();
        s.enter_watch_mode(session);
        let res = s.commit_predicate();
        assert!(res.is_ok());
        assert_eq!(s.watch().unwrap().predicate.clauses().len(), 1);
    }

    #[test]
    fn commit_predicate_parse_error_keeps_old_predicate() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.predicate = Predicate::parse("filename = ok").unwrap(); // existing predicate
        session.predicate_input = "garbage".into(); // bad input
        s.enter_watch_mode(session);
        let res = s.commit_predicate();
        assert!(res.is_err());
        // The previous predicate is unchanged.
        let watch = s.watch().unwrap();
        assert_eq!(watch.predicate.clauses().len(), 1);
        assert_eq!(watch.predicate.clauses()[0].attribute, "filename");
    }

    #[test]
    fn commit_predicate_empty_input_clears_predicate() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.predicate = Predicate::parse("filename = ok").unwrap();
        session.predicate_input = "".into();
        s.enter_watch_mode(session);
        let res = s.commit_predicate();
        assert!(res.is_ok());
        // Empty predicate matches anything.
        assert!(s.watch().unwrap().predicate.is_empty());
    }

    #[test]
    fn leaving_watch_with_buffered_events_arms_confirm() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.buffer.push_back(matched(1));
        s.enter_watch_mode(session);

        let armed = s.request_exit_watch();
        assert!(armed, "non-empty buffer must arm the confirm modal");
        assert!(s.exit_watch_pending);
        // Mode unchanged until confirmed.
        assert!(matches!(s.mode, EventsMode::Watch(_)));
    }

    #[test]
    fn leaving_watch_with_empty_buffer_exits_immediately() {
        let mut s = EventsState::new();
        s.enter_watch_mode(empty_session(100));

        let armed = s.request_exit_watch();
        assert!(!armed, "empty buffer should not need confirmation");
        assert!(matches!(s.mode, EventsMode::OneShot));
    }

    #[test]
    fn request_exit_watch_no_op_in_oneshot() {
        let mut s = EventsState::new();
        let armed = s.request_exit_watch();
        assert!(!armed);
        assert!(matches!(s.mode, EventsMode::OneShot));
        assert!(!s.exit_watch_pending);
    }

    #[test]
    fn confirm_exit_watch_drops_buffer_and_returns_to_oneshot() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.buffer.push_back(matched(1));
        s.enter_watch_mode(session);

        s.request_exit_watch();
        s.confirm_exit_watch();
        assert!(matches!(s.mode, EventsMode::OneShot));
        assert!(!s.exit_watch_pending);
    }

    #[test]
    fn cancel_exit_watch_keeps_session_and_disarms() {
        let mut s = EventsState::new();
        let mut session = empty_session(100);
        session.buffer.push_back(matched(1));
        s.enter_watch_mode(session);

        s.request_exit_watch();
        s.cancel_exit_watch();
        assert!(matches!(s.mode, EventsMode::Watch(_)));
        assert!(!s.exit_watch_pending);
        assert_eq!(s.watch().unwrap().buffer.len(), 1);
    }
}
