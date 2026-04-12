// Consumed by Tasks 10–13
#![allow(dead_code)]
//! TracerState — pure data skeleton for the Tracer tab.
//!
//! The sum type `TracerMode` drives which sub-view is rendered and which
//! key bindings are active. All fields are mutated exclusively on the UI
//! task via `apply_payload`.

use std::time::SystemTime;

use tokio::task::AbortHandle;

use crate::client::{
    AttributeTriple, ContentRender, ContentSide, LatestEventsSnapshot, LineageSnapshot,
    ProvenanceEventDetail, ProvenanceEventSummary,
};

use crate::event::TracerPayload;

// ── Top-level state ──────────────────────────────────────────────────────────

/// Full mutable state for the Tracer tab.
#[derive(Debug)]
pub struct TracerState {
    /// Which sub-view is currently active.
    pub mode: TracerMode,
    /// Last error message from any async operation in this tab.
    pub last_error: Option<String>,
}

impl TracerState {
    /// Creates a fresh `TracerState` starting in the UUID entry screen.
    pub fn new() -> Self {
        Self {
            mode: TracerMode::Entry(EntryState::default()),
            last_error: None,
        }
    }
}

impl Default for TracerState {
    fn default() -> Self {
        Self::new()
    }
}

// ── Mode enum ────────────────────────────────────────────────────────────────

/// Discriminated union of the four Tracer sub-views.
#[derive(Debug)]
pub enum TracerMode {
    /// UUID entry field — user pastes or types a flowfile UUID.
    Entry(EntryState),
    /// Lineage query is being submitted / polled.
    LineageRunning(LineageRunningState),
    /// Lineage query finished; showing the event timeline.
    Lineage(Box<LineageView>),
    /// Cross-linked from Bulletins; showing latest events for one component.
    LatestEvents(LatestEventsView),
}

// ── Entry ────────────────────────────────────────────────────────────────────

/// State for the UUID entry sub-view.
#[derive(Debug, Default)]
pub struct EntryState {
    /// Current contents of the UUID input field.
    pub input: String,
}

// ── LatestEvents ─────────────────────────────────────────────────────────────

/// Shows the most-recent provenance events for a single component.
#[derive(Debug)]
pub struct LatestEventsView {
    /// NiFi component UUID that was cross-linked.
    pub component_id: String,
    /// Human-readable label assembled from the first event's metadata.
    pub component_label: String,
    /// Ordered list of event summaries (newest last, as returned by the API).
    pub events: Vec<ProvenanceEventSummary>,
    /// Index of the currently highlighted row.
    pub selected: usize,
    /// When the snapshot was fetched.
    pub fetched_at: SystemTime,
    /// True while an async fetch is in flight.
    pub loading: bool,
}

impl LatestEventsView {
    /// Constructs a view pre-populated from a [`LatestEventsSnapshot`].
    pub fn from_snapshot(snap: LatestEventsSnapshot) -> Self {
        Self {
            component_id: snap.component_id,
            component_label: snap.component_label,
            events: snap.events,
            selected: 0,
            fetched_at: snap.fetched_at,
            loading: false,
        }
    }
}

// ── LineageRunning ───────────────────────────────────────────────────────────

/// State while a lineage query is being polled.
#[derive(Debug)]
pub struct LineageRunningState {
    /// The flowfile UUID being traced.
    pub uuid: String,
    /// Opaque query ID returned by the NiFi server.
    pub query_id: String,
    /// Last reported completion percentage (0–100).
    pub percent: u8,
    /// Wall-clock time when the query was submitted.
    pub started_at: SystemTime,
    /// Handle to cancel the polling task if the user presses Escape.
    pub abort: Option<AbortHandle>,
}

// ── LineageView ──────────────────────────────────────────────────────────────

/// State after the lineage query has finished.
#[derive(Debug)]
pub struct LineageView {
    /// The flowfile UUID that was traced.
    pub uuid: String,
    /// Complete lineage snapshot returned by the server.
    pub snapshot: LineageSnapshot,
    /// Index of the currently selected event row.
    pub selected_event: usize,
    /// Detail pane for the selected event (loaded on demand).
    pub event_detail: EventDetail,
    /// Whether to show all attributes or only changed ones.
    pub diff_mode: AttributeDiffMode,
    /// When the lineage snapshot was last fetched.
    pub fetched_at: SystemTime,
}

// ── EventDetail ──────────────────────────────────────────────────────────────

/// Load state of the per-event detail pane.
#[derive(Debug, Default)]
pub enum EventDetail {
    /// No fetch has been requested yet.
    #[default]
    NotLoaded,
    /// A fetch is in flight.
    Loading,
    /// Detail loaded successfully; content may be separately loaded.
    Loaded {
        event: Box<ProvenanceEventDetail>,
        content: ContentPane,
    },
    /// The fetch failed.
    Failed(String),
}

// ── ContentPane ──────────────────────────────────────────────────────────────

/// Load state of the content preview within an event detail pane.
#[derive(Debug, Default)]
pub enum ContentPane {
    /// Not yet requested; user must press a keybind.
    #[default]
    Collapsed,
    /// Input-side fetch is in flight.
    LoadingInput,
    /// Output-side fetch is in flight.
    LoadingOutput,
    /// Content loaded and ready to display.
    Shown {
        side: ContentSide,
        render: ContentRender,
        total_bytes: usize,
        /// Raw bytes retained for the optional save-to-file flow.
        raw: std::sync::Arc<[u8]>,
    },
    /// The content fetch failed.
    Failed(String),
}

// ── AttributeDiffMode ────────────────────────────────────────────────────────

/// Controls which attributes are shown in the detail pane.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum AttributeDiffMode {
    /// Show all attributes regardless of whether they changed.
    #[default]
    All,
    /// Show only attributes whose `previous` differs from `current`.
    Changed,
}

impl AttributeDiffMode {
    /// Cycles between `All` and `Changed`.
    pub fn toggle(self) -> Self {
        match self {
            Self::All => Self::Changed,
            Self::Changed => Self::All,
        }
    }

    /// Returns true if `triple` should be shown under this mode.
    pub fn matches(self, triple: &AttributeTriple) -> bool {
        match self {
            Self::All => true,
            Self::Changed => triple.is_changed(),
        }
    }
}

// ── Followup ─────────────────────────────────────────────────────────────────

/// Side-effect requests that `apply_payload` may return to the caller.
///
/// These are one-shot requests that require an async operation (e.g. deleting
/// a server-side query after it has been consumed). The app loop processes
/// them after the state mutation.
#[derive(Debug)]
pub enum Followup {
    /// Ask the server to delete a completed lineage query.
    DeleteLineageQuery { query_id: String },
}

// ── Reducer ───────────────────────────────────────────────────────────────────

/// Transitions to LatestEvents mode with loading=true and an empty event list.
///
/// `component_label` is initially set to `component_id` and updated when the
/// first [`TracerPayload::LatestEvents`] snapshot arrives.
pub fn start_latest_events(state: &mut TracerState, component_id: String) {
    state.mode = TracerMode::LatestEvents(LatestEventsView {
        component_label: component_id.clone(),
        component_id,
        events: Vec::new(),
        selected: 0,
        fetched_at: SystemTime::now(),
        loading: true,
    });
    state.last_error = None;
}

/// Moves the selection down by one row in LatestEvents mode, wrapping at the end.
pub fn latest_events_move_down(state: &mut TracerState) {
    if let TracerMode::LatestEvents(ref mut view) = state.mode {
        let len = view.events.len();
        if len > 0 {
            view.selected = (view.selected + 1) % len;
        }
    }
}

/// Moves the selection up by one row in LatestEvents mode, wrapping at the start.
pub fn latest_events_move_up(state: &mut TracerState) {
    if let TracerMode::LatestEvents(ref mut view) = state.mode {
        let len = view.events.len();
        if len > 0 {
            view.selected = view.selected.checked_sub(1).unwrap_or(len - 1);
        }
    }
}

/// Returns the `flow_file_uuid` of the currently selected row in LatestEvents mode,
/// or `None` when not in that mode or the event list is empty.
pub fn latest_events_selected_uuid(state: &TracerState) -> Option<String> {
    if let TracerMode::LatestEvents(ref view) = state.mode {
        view.events
            .get(view.selected)
            .map(|e| e.flow_file_uuid.clone())
    } else {
        None
    }
}

/// Transitions from Entry to LineageRunning with an empty query_id.
pub fn start_lineage(state: &mut TracerState, uuid: String, abort: Option<AbortHandle>) {
    state.mode = TracerMode::LineageRunning(LineageRunningState {
        uuid,
        query_id: String::new(),
        percent: 0,
        started_at: SystemTime::now(),
        abort,
    });
    state.last_error = None;
}

/// Cancels a running lineage query, returning to Entry mode.
///
/// If a query_id has been received, emits a [`Followup::DeleteLineageQuery`]
/// so the caller can clean it up on the server.
pub fn cancel_lineage(state: &mut TracerState) -> Option<Followup> {
    let mut followup = None;
    if let TracerMode::LineageRunning(LineageRunningState {
        query_id, abort, ..
    }) = &mut state.mode
    {
        if let Some(handle) = abort.take() {
            handle.abort();
        }
        if !query_id.is_empty() {
            followup = Some(Followup::DeleteLineageQuery {
                query_id: std::mem::take(query_id),
            });
        }
    }
    state.mode = TracerMode::Entry(EntryState::default());
    followup
}

/// Folds a [`TracerPayload`] into `state`.
///
/// Returns an optional [`Followup`] when an async side-effect is needed.
pub fn apply_payload(state: &mut TracerState, payload: TracerPayload) -> Option<Followup> {
    match payload {
        TracerPayload::LineageSubmitted { uuid, query_id } => {
            if let TracerMode::LineageRunning(ref mut running) = state.mode
                && running.uuid == uuid
                && running.query_id.is_empty()
            {
                running.query_id = query_id;
            }
            None
        }
        TracerPayload::LineagePartial { query_id, percent } => {
            if let TracerMode::LineageRunning(ref mut running) = state.mode
                && running.query_id == query_id
            {
                running.percent = percent;
            }
            None
        }
        TracerPayload::LineageDone {
            uuid,
            query_id,
            snapshot,
            fetched_at,
        } => {
            if let TracerMode::LineageRunning(ref running) = state.mode
                && running.query_id == query_id
            {
                state.mode = TracerMode::Lineage(Box::new(LineageView {
                    uuid,
                    snapshot,
                    selected_event: 0,
                    event_detail: EventDetail::default(),
                    diff_mode: AttributeDiffMode::default(),
                    fetched_at,
                }));
                return Some(Followup::DeleteLineageQuery { query_id });
            }
            // Stale query_id — still emit delete so it gets cleaned up.
            Some(Followup::DeleteLineageQuery { query_id })
        }
        TracerPayload::LineageFailed {
            query_id, error, ..
        } => {
            if let TracerMode::LineageRunning(ref running) = state.mode
                && (running.query_id == query_id || running.query_id.is_empty())
            {
                state.last_error = Some(error);
                state.mode = TracerMode::Entry(EntryState::default());
            }
            None
        }
        TracerPayload::LatestEvents(snap) => {
            if let TracerMode::LatestEvents(ref mut view) = state.mode
                && view.component_id == snap.component_id
            {
                view.component_label = snap.component_label;
                view.events = snap.events;
                view.fetched_at = snap.fetched_at;
                view.loading = false;
            }
            None
        }
        TracerPayload::LatestEventsFailed {
            component_id,
            error,
        } => {
            if let TracerMode::LatestEvents(ref mut view) = state.mode
                && view.component_id == component_id
            {
                view.loading = false;
                state.last_error = Some(error);
            }
            None
        }
        // Arms filled in by Task 13.
        _ => None,
    }
}

// ── Entry-mode helpers ────────────────────────────────────────────────────────

/// Appends `ch` to the UUID input field when in Entry mode.
pub fn handle_entry_char(state: &mut TracerState, ch: char) {
    if let TracerMode::Entry(EntryState { input }) = &mut state.mode {
        state.last_error = None;
        input.push(ch);
    }
}

/// Removes the last character from the UUID input field when in Entry mode.
pub fn handle_entry_backspace(state: &mut TracerState) {
    if let TracerMode::Entry(EntryState { input }) = &mut state.mode {
        state.last_error = None;
        input.pop();
    }
}

/// Clears the UUID input field when in Entry mode.
pub fn handle_entry_clear(state: &mut TracerState) {
    if let TracerMode::Entry(EntryState { input }) = &mut state.mode {
        state.last_error = None;
        input.clear();
    }
}

/// Validates the current input as a UUID.
///
/// Returns `Some(uuid_string)` on success (normalised to lowercase hyphenated
/// form), or `None` after setting `state.last_error` when the input is not a
/// valid UUID. Returns `None` immediately when not in Entry mode.
pub fn entry_submit(state: &mut TracerState) -> Option<String> {
    let TracerMode::Entry(EntryState { input }) = &state.mode else {
        return None;
    };
    let trimmed = input.trim();
    match uuid::Uuid::parse_str(trimmed) {
        Ok(u) => {
            state.last_error = None;
            Some(u.to_string())
        }
        Err(_) => {
            state.last_error = Some("invalid UUID: expected 8-4-4-4-12 hex".to_string());
            None
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_state_is_entry_empty() {
        let state = TracerState::new();
        assert!(matches!(state.mode, TracerMode::Entry(ref e) if e.input.is_empty()));
        assert!(state.last_error.is_none());
    }

    #[test]
    fn entry_typing_accumulates() {
        let mut state = TracerState::new();
        handle_entry_char(&mut state, 'a');
        handle_entry_char(&mut state, 'b');
        handle_entry_char(&mut state, 'c');
        let TracerMode::Entry(EntryState { input }) = &state.mode else {
            panic!("expected Entry mode");
        };
        assert_eq!(input, "abc");
    }

    #[test]
    fn entry_backspace_removes_last_char() {
        let mut state = TracerState::new();
        handle_entry_char(&mut state, 'a');
        handle_entry_char(&mut state, 'b');
        handle_entry_backspace(&mut state);
        let TracerMode::Entry(EntryState { input }) = &state.mode else {
            panic!("expected Entry mode");
        };
        assert_eq!(input, "a");
    }

    #[test]
    fn entry_ctrl_u_clears() {
        let mut state = TracerState::new();
        handle_entry_char(&mut state, 'a');
        handle_entry_char(&mut state, 'b');
        handle_entry_clear(&mut state);
        let TracerMode::Entry(EntryState { input }) = &state.mode else {
            panic!("expected Entry mode");
        };
        assert!(input.is_empty());
    }

    #[test]
    fn entry_submit_valid_uuid_returns_validated() {
        let mut state = TracerState::new();
        for ch in "7a2e8b9c-1234-4abc-9def-0123456789ab".chars() {
            handle_entry_char(&mut state, ch);
        }
        let result = entry_submit(&mut state);
        assert_eq!(
            result.as_deref(),
            Some("7a2e8b9c-1234-4abc-9def-0123456789ab")
        );
        assert!(state.last_error.is_none());
    }

    #[test]
    fn entry_submit_invalid_uuid_sets_banner_and_returns_none() {
        let mut state = TracerState::new();
        for ch in "not-a-uuid".chars() {
            handle_entry_char(&mut state, ch);
        }
        let result = entry_submit(&mut state);
        assert!(result.is_none());
        assert_eq!(
            state.last_error.as_deref(),
            Some("invalid UUID: expected 8-4-4-4-12 hex")
        );
    }

    #[test]
    fn attribute_diff_mode_toggles() {
        let mode = AttributeDiffMode::All;
        let toggled = mode.toggle();
        assert_eq!(toggled, AttributeDiffMode::Changed);
        assert_eq!(toggled.toggle(), AttributeDiffMode::All);
    }

    // ── LineageRunning reducer tests ────────────────────────────────────────

    const TEST_UUID: &str = "7a2e8b9c-1234-4abc-9def-0123456789ab";

    #[test]
    fn start_lineage_transitions_entry_to_running_with_empty_query_id() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        let TracerMode::LineageRunning(ref running) = state.mode else {
            panic!("expected LineageRunning mode");
        };
        assert_eq!(running.uuid, TEST_UUID);
        assert!(running.query_id.is_empty());
        assert_eq!(running.percent, 0);
        assert!(running.abort.is_none());
        assert!(state.last_error.is_none());
    }

    #[test]
    fn lineage_submitted_fills_query_id_when_uuid_matches() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);

        let followup = apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );
        assert!(followup.is_none());

        let TracerMode::LineageRunning(ref running) = state.mode else {
            panic!("expected LineageRunning mode");
        };
        assert_eq!(running.query_id, "q-42");
    }

    #[test]
    fn lineage_submitted_stale_uuid_is_dropped() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);

        let followup = apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: "stale-uuid".to_string(),
                query_id: "q-99".to_string(),
            },
        );
        assert!(followup.is_none());

        let TracerMode::LineageRunning(ref running) = state.mode else {
            panic!("expected LineageRunning mode");
        };
        assert!(running.query_id.is_empty(), "query_id should remain empty");
    }

    #[test]
    fn lineage_partial_updates_percent_when_query_id_matches() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        let followup = apply_payload(
            &mut state,
            TracerPayload::LineagePartial {
                query_id: "q-42".to_string(),
                percent: 55,
            },
        );
        assert!(followup.is_none());

        let TracerMode::LineageRunning(ref running) = state.mode else {
            panic!("expected LineageRunning mode");
        };
        assert_eq!(running.percent, 55);
    }

    #[test]
    fn lineage_partial_stale_query_id_is_dropped() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        apply_payload(
            &mut state,
            TracerPayload::LineagePartial {
                query_id: "q-stale".to_string(),
                percent: 99,
            },
        );

        let TracerMode::LineageRunning(ref running) = state.mode else {
            panic!("expected LineageRunning mode");
        };
        assert_eq!(running.percent, 0, "percent should stay at 0");
    }

    #[test]
    fn lineage_done_transitions_to_lineage_view() {
        use crate::client::LineageSnapshot;

        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        let snapshot = LineageSnapshot {
            events: vec![],
            percent_completed: 100,
            finished: true,
        };
        let followup = apply_payload(
            &mut state,
            TracerPayload::LineageDone {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
                snapshot,
                fetched_at: SystemTime::now(),
            },
        );

        assert!(matches!(state.mode, TracerMode::Lineage(_)));
        assert!(
            matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id }) if query_id == "q-42")
        );
    }

    #[test]
    fn lineage_done_stale_query_id_emits_delete_followup() {
        use crate::client::LineageSnapshot;

        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        let snapshot = LineageSnapshot {
            events: vec![],
            percent_completed: 100,
            finished: true,
        };
        let followup = apply_payload(
            &mut state,
            TracerPayload::LineageDone {
                uuid: TEST_UUID.to_string(),
                query_id: "q-stale".to_string(),
                snapshot,
                fetched_at: SystemTime::now(),
            },
        );

        // State should remain LineageRunning (stale query_id doesn't match)
        assert!(matches!(state.mode, TracerMode::LineageRunning(_)));
        // But we still emit delete to clean up the stale query on the server
        assert!(
            matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id }) if query_id == "q-stale")
        );
    }

    #[test]
    fn lineage_failed_returns_to_entry_with_error() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        apply_payload(
            &mut state,
            TracerPayload::LineageFailed {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
                error: "server error".to_string(),
            },
        );

        assert!(matches!(state.mode, TracerMode::Entry(_)));
        assert_eq!(state.last_error.as_deref(), Some("server error"));
    }

    #[test]
    fn cancel_lineage_transitions_to_entry_and_emits_delete_when_query_id_known() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);
        apply_payload(
            &mut state,
            TracerPayload::LineageSubmitted {
                uuid: TEST_UUID.to_string(),
                query_id: "q-42".to_string(),
            },
        );

        let followup = cancel_lineage(&mut state);
        assert!(matches!(state.mode, TracerMode::Entry(_)));
        assert!(
            matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id }) if query_id == "q-42")
        );
    }

    #[test]
    fn cancel_lineage_before_submission_does_not_emit_delete() {
        let mut state = TracerState::new();
        start_lineage(&mut state, TEST_UUID.to_string(), None);

        let followup = cancel_lineage(&mut state);
        assert!(matches!(state.mode, TracerMode::Entry(_)));
        assert!(followup.is_none());
    }

    // ── LatestEvents reducer tests ──────────────────────────────────────────

    const COMP_ID: &str = "comp-aaaa-bbbb-cccc-dddddddddddd";

    fn fake_summary(id: i64, uuid: &str) -> ProvenanceEventSummary {
        ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: COMP_ID.to_string(),
            component_name: "MyProcessor".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: uuid.to_string(),
            relationship: None,
            details: None,
        }
    }

    #[test]
    fn start_latest_events_transitions_into_loading_view() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        let TracerMode::LatestEvents(ref view) = state.mode else {
            panic!("expected LatestEvents mode");
        };
        assert_eq!(view.component_id, COMP_ID);
        assert_eq!(view.component_label, COMP_ID);
        assert!(view.events.is_empty());
        assert_eq!(view.selected, 0);
        assert!(view.loading);
        assert!(state.last_error.is_none());
    }

    #[test]
    fn latest_events_payload_populates_matching_component() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        let snap = LatestEventsSnapshot {
            component_id: COMP_ID.to_string(),
            component_label: "MyProcessor".to_string(),
            events: vec![fake_summary(1, "uuid-1111"), fake_summary(2, "uuid-2222")],
            fetched_at: SystemTime::now(),
        };
        let followup = apply_payload(&mut state, TracerPayload::LatestEvents(snap));
        assert!(followup.is_none());

        let TracerMode::LatestEvents(ref view) = state.mode else {
            panic!("expected LatestEvents mode");
        };
        assert_eq!(view.component_label, "MyProcessor");
        assert_eq!(view.events.len(), 2);
        assert!(!view.loading);
    }

    #[test]
    fn latest_events_payload_with_mismatched_component_is_dropped() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        let snap = LatestEventsSnapshot {
            component_id: "other-component".to_string(),
            component_label: "Other".to_string(),
            events: vec![fake_summary(99, "uuid-9999")],
            fetched_at: SystemTime::now(),
        };
        apply_payload(&mut state, TracerPayload::LatestEvents(snap));

        let TracerMode::LatestEvents(ref view) = state.mode else {
            panic!("expected LatestEvents mode");
        };
        assert!(view.events.is_empty(), "events should remain empty");
        assert!(view.loading, "loading should remain true");
    }

    #[test]
    fn latest_events_j_k_moves_selection_and_wraps() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        // Populate with 3 events via payload
        let snap = LatestEventsSnapshot {
            component_id: COMP_ID.to_string(),
            component_label: "MyProcessor".to_string(),
            events: vec![
                fake_summary(1, "uuid-1111"),
                fake_summary(2, "uuid-2222"),
                fake_summary(3, "uuid-3333"),
            ],
            fetched_at: SystemTime::now(),
        };
        apply_payload(&mut state, TracerPayload::LatestEvents(snap));

        // Move down: 0 → 1 → 2 → wraps to 0
        latest_events_move_down(&mut state);
        assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 1));
        latest_events_move_down(&mut state);
        latest_events_move_down(&mut state); // wraps
        assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 0));

        // Move up from 0 wraps to last (2)
        latest_events_move_up(&mut state);
        assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 2));
    }

    #[test]
    fn latest_events_selected_uuid_returns_row_uuid() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        let snap = LatestEventsSnapshot {
            component_id: COMP_ID.to_string(),
            component_label: "MyProcessor".to_string(),
            events: vec![fake_summary(1, "uuid-1111"), fake_summary(2, "uuid-2222")],
            fetched_at: SystemTime::now(),
        };
        apply_payload(&mut state, TracerPayload::LatestEvents(snap));

        assert_eq!(
            latest_events_selected_uuid(&state).as_deref(),
            Some("uuid-1111")
        );

        latest_events_move_down(&mut state);
        assert_eq!(
            latest_events_selected_uuid(&state).as_deref(),
            Some("uuid-2222")
        );
    }

    #[test]
    fn latest_events_failed_payload_clears_loading_and_sets_banner() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, COMP_ID.to_string());

        let followup = apply_payload(
            &mut state,
            TracerPayload::LatestEventsFailed {
                component_id: COMP_ID.to_string(),
                error: "connection refused".to_string(),
            },
        );
        assert!(followup.is_none());

        let TracerMode::LatestEvents(ref view) = state.mode else {
            panic!("expected LatestEvents mode");
        };
        assert!(!view.loading);
        assert_eq!(state.last_error.as_deref(), Some("connection refused"));
    }
}
