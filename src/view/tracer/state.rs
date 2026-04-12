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

/// Folds a [`TracerPayload`] into `state`.
///
/// Returns an optional [`Followup`] when an async side-effect is needed.
pub fn apply_payload(_state: &mut TracerState, _payload: TracerPayload) -> Option<Followup> {
    // Arms filled in by Tasks 11–13.
    None
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
}
