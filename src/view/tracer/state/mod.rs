// Consumed by Tasks 10–13
//! TracerState — pure data skeleton for the Tracer tab.
//!
//! The sum type `TracerMode` drives which sub-view is rendered and which
//! key bindings are active. All fields are mutated exclusively on the UI
//! task via `apply_payload`.

use std::time::SystemTime;

use tokio::task::AbortHandle;

use crate::app::navigation::ListNavigation;
use crate::client::{AttributeTriple, ContentRender, ContentSide};

use crate::event::TracerPayload;

mod entry_types;
mod lineage_types;

pub use entry_types::*;
pub use lineage_types::*;

// ── Top-level state ──────────────────────────────────────────────────────────

/// Full mutable state for the Tracer tab.
#[derive(Debug)]
pub struct TracerState {
    /// Which sub-view is currently active.
    pub mode: TracerMode,
    /// Last error message from any async operation in this tab.
    pub last_error: Option<String>,
    /// Open content viewer modal, if any.
    pub content_modal: Option<ContentModalState>,
}

impl TracerState {
    /// Creates a fresh `TracerState` starting in the UUID entry screen.
    pub fn new() -> Self {
        Self {
            mode: TracerMode::Entry(EntryState::default()),
            last_error: None,
            content_modal: None,
        }
    }

    /// Returns the component ID that currently has focus in Tracer, or `None`
    /// when no component is selected.  In `LatestEvents` mode the view itself
    /// carries the component; in `Lineage` mode the selected event's
    /// `component_id` is used.
    pub fn selected_component_id(&self) -> Option<String> {
        match &self.mode {
            TracerMode::LatestEvents(v) => Some(v.component_id.clone()),
            TracerMode::Lineage(v) => v
                .snapshot
                .events
                .get(v.selected_event)
                .map(|e| e.component_id.clone()),
            _ => None,
        }
    }

    /// Returns the speaking component name for the currently-focused
    /// component in Tracer, or `None` when nothing is selected.
    /// Parallel to [`Self::selected_component_id`]:
    /// * `LatestEvents` → `view.component_label`
    /// * `Lineage` → the selected event's `component_name`
    /// * other modes → `None`
    pub fn selected_component_label(&self) -> Option<String> {
        match &self.mode {
            TracerMode::LatestEvents(v) => Some(v.component_label.clone()),
            TracerMode::Lineage(v) => v
                .snapshot
                .events
                .get(v.selected_event)
                .map(|e| e.component_name.clone()),
            _ => None,
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

// ── Followup ─────────────────────────────────────────────────────────────────

/// Side-effect requests that `apply_payload` may return to the caller.
///
/// These are one-shot requests that require an async operation (e.g. deleting
/// a server-side query after it has been consumed). The app loop processes
/// them after the state mutation.
#[derive(Debug)]
pub enum Followup {
    /// Ask the server to delete a completed lineage query.
    DeleteLineageQuery {
        query_id: String,
        cluster_node_id: Option<String>,
    },
}

// ── Reducer ───────────────────────────────────────────────────────────────────

/// Moves the selection down by one row in Lineage mode, wrapping at the end.
///
/// Resets `event_detail` to [`EventDetail::NotLoaded`] and focus to
/// [`LineageFocus::Timeline`] on any selection change.
pub fn lineage_move_down(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let prev = view.selected_event;
        ListNavigation::move_down(view.as_mut());
        if view.selected_event != prev {
            view.event_detail = EventDetail::NotLoaded;
            view.focus = LineageFocus::Timeline;
            view.active_detail_tab = DetailTab::default();
        }
    }
}

/// Moves the selection up by one row in Lineage mode, wrapping at the start.
///
/// Resets `event_detail` to [`EventDetail::NotLoaded`] and focus to
/// [`LineageFocus::Timeline`] on any selection change.
pub fn lineage_move_up(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let prev = view.selected_event;
        ListNavigation::move_up(view.as_mut());
        if view.selected_event != prev {
            view.event_detail = EventDetail::NotLoaded;
            view.focus = LineageFocus::Timeline;
            view.active_detail_tab = DetailTab::default();
        }
    }
}

/// Sets `event_detail` to [`EventDetail::Loading`] in Lineage mode.
///
/// Also resets focus to [`LineageFocus::Timeline`] — the attribute
/// table doesn't exist yet.
pub fn lineage_mark_detail_loading(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.event_detail = EventDetail::Loading;
    }
}

/// Returns the attributes currently visible in the detail pane, after
/// applying [`AttributeDiffMode`] filtering. Returns an empty vec when
/// the event detail hasn't loaded yet.
pub fn lineage_visible_attributes(view: &LineageView) -> Vec<&AttributeTriple> {
    match &view.event_detail {
        EventDetail::Loaded { event, .. } => event
            .attributes
            .iter()
            .filter(|a| view.diff_mode.matches(a))
            .collect(),
        _ => Vec::new(),
    }
}

/// Attempts to move keyboard focus from the timeline into the attribute
/// table. Only acts when an event detail is currently [`EventDetail::Loaded`]
/// and has at least one visible attribute under the current diff mode.
///
/// Returns `true` on a successful transition, `false` otherwise (the
/// caller can use the return value to surface an info banner).
pub fn lineage_focus_attributes(state: &mut TracerState) -> bool {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && matches!(view.event_detail, EventDetail::Loaded { .. })
        && !lineage_visible_attributes(view).is_empty()
    {
        view.focus = LineageFocus::Attributes { row: 0 };
        view.active_detail_tab = DetailTab::Attributes;
        return true;
    }
    false
}

/// Returns keyboard focus to the timeline. Idempotent.
pub fn lineage_focus_timeline(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.focus = LineageFocus::Timeline;
    }
}

/// Moves the attribute-table row cursor down, wrapping at the end.
/// No-op unless focus is [`LineageFocus::Attributes`].
pub fn lineage_attr_move_down(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let LineageFocus::Attributes { row } = view.focus
    {
        let visible = lineage_visible_attributes(view).len();
        if visible == 0 {
            view.focus = LineageFocus::Timeline;
            return;
        }
        view.focus = LineageFocus::Attributes {
            row: (row + 1) % visible,
        };
    }
}

/// Moves the attribute-table row cursor up, wrapping at the start.
/// No-op unless focus is [`LineageFocus::Attributes`].
pub fn lineage_attr_move_up(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let LineageFocus::Attributes { row } = view.focus
    {
        let visible = lineage_visible_attributes(view).len();
        if visible == 0 {
            view.focus = LineageFocus::Timeline;
            return;
        }
        let new_row = if row == 0 { visible - 1 } else { row - 1 };
        view.focus = LineageFocus::Attributes { row: new_row };
    }
}

/// Returns the focused attribute's current value, for clipboard copy.
/// When the current value is absent (deleted rows), returns the previous
/// value so `c` still yields something useful. Returns `None` unless the
/// focus is on a valid attribute row.
pub fn lineage_focused_attribute_value(state: &TracerState) -> Option<String> {
    if let TracerMode::Lineage(ref view) = state.mode
        && let LineageFocus::Attributes { row } = view.focus
    {
        let visible = lineage_visible_attributes(view);
        if let Some(attr) = visible.get(row) {
            return attr
                .current
                .clone()
                .or_else(|| attr.previous.clone())
                .or_else(|| Some(String::new()));
        }
    }
    None
}

/// Returns the number of rendered lines for the currently shown content
/// pane in the detail view, or 0 when nothing is displayable.
pub fn lineage_content_line_count(view: &LineageView) -> usize {
    if let EventDetail::Loaded {
        content: ContentPane::Shown { render, .. },
        ..
    } = &view.event_detail
    {
        match render {
            ContentRender::Text { text, .. } => text.lines().count().max(1),
            ContentRender::Hex { first_4k } => first_4k.lines().count().max(1),
            ContentRender::Empty => 1,
            ContentRender::Tabular {
                schema_summary,
                body,
                ..
            } => (schema_summary.lines().count() + 1 + body.lines().count()).max(1),
        }
    } else {
        0
    }
}

/// Attempts to move keyboard focus into the content pane. Only acts
/// when the current content pane is in [`ContentPane::Shown`] state.
///
/// Returns `true` on a successful transition.
pub fn lineage_focus_content(state: &mut TracerState) -> bool {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let EventDetail::Loaded {
            content: ContentPane::Shown { side, .. },
            ..
        } = &view.event_detail
    {
        view.active_detail_tab = match side {
            ContentSide::Input => DetailTab::Input,
            ContentSide::Output => DetailTab::Output,
        };
        view.focus = LineageFocus::Content { scroll: 0 };
        return true;
    }
    false
}

/// Scrolls the focused content pane down by `by` lines, clamped at the
/// last visible line. No-op unless focus is [`LineageFocus::Content`].
pub fn lineage_content_scroll_down(state: &mut TracerState, by: u16) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let max = lineage_content_line_count(view).saturating_sub(1) as u16;
        if let LineageFocus::Content { ref mut scroll } = view.focus {
            *scroll = scroll.saturating_add(by).min(max);
        }
    }
}

/// Scrolls the focused content pane up by `by` lines, saturating at 0.
/// No-op unless focus is [`LineageFocus::Content`].
pub fn lineage_content_scroll_up(state: &mut TracerState, by: u16) {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let LineageFocus::Content { ref mut scroll } = view.focus
    {
        *scroll = scroll.saturating_sub(by);
    }
}

/// Sets the content-pane scroll to the first line (`Home`).
pub fn lineage_content_scroll_home(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let LineageFocus::Content { ref mut scroll } = view.focus
    {
        *scroll = 0;
    }
}

/// Sets the content-pane scroll to the last line (`End`).
pub fn lineage_content_scroll_end(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let max = lineage_content_line_count(view).saturating_sub(1) as u16;
        if let LineageFocus::Content { ref mut scroll } = view.focus {
            *scroll = max;
        }
    }
}

/// Returns `(has_input, has_output)` for the currently loaded event, or
/// `(false, false)` when the detail is not loaded.
pub fn lineage_content_availability(view: &LineageView) -> (bool, bool) {
    if let EventDetail::Loaded { ref event, .. } = view.event_detail {
        (event.input_available, event.output_available)
    } else {
        (false, false)
    }
}

/// Cycles `active_detail_tab` to the right, skipping disabled tabs.
/// Also adjusts `focus` to match the new tab.
pub fn lineage_cycle_detail_tab_right(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let (has_input, has_output) = lineage_content_availability(view);
        let new_tab = view.active_detail_tab.cycle_right(has_input, has_output);
        view.active_detail_tab = new_tab;
        view.focus = match new_tab {
            DetailTab::Attributes => LineageFocus::Attributes { row: 0 },
            DetailTab::Input | DetailTab::Output => LineageFocus::Content { scroll: 0 },
        };
    }
}

/// Cycles `active_detail_tab` to the left, skipping disabled tabs.
/// Also adjusts `focus` to match the new tab.
pub fn lineage_cycle_detail_tab_left(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        let (has_input, has_output) = lineage_content_availability(view);
        let new_tab = view.active_detail_tab.cycle_left(has_input, has_output);
        view.active_detail_tab = new_tab;
        view.focus = match new_tab {
            DetailTab::Attributes => LineageFocus::Attributes { row: 0 },
            DetailTab::Input | DetailTab::Output => LineageFocus::Content { scroll: 0 },
        };
    }
}

/// Transitions the content pane to the appropriate loading state in Lineage mode.
///
/// Only acts when `event_detail` is [`EventDetail::Loaded`]. Sets `content` to
/// [`ContentPane::LoadingInput`] or [`ContentPane::LoadingOutput`] depending on `side`.
pub fn lineage_mark_content_loading(state: &mut TracerState, side: ContentSide) {
    if let TracerMode::Lineage(ref mut view) = state.mode
        && let EventDetail::Loaded {
            ref mut content, ..
        } = view.event_detail
    {
        *content = match side {
            ContentSide::Input => ContentPane::LoadingInput,
            ContentSide::Output => ContentPane::LoadingOutput,
        };
    }
}

/// Toggles the attribute diff mode between [`AttributeDiffMode::All`] and
/// [`AttributeDiffMode::Changed`] in Lineage mode.
pub fn lineage_toggle_diff_mode(state: &mut TracerState) {
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.diff_mode = view.diff_mode.toggle();
    }
}

/// Returns the `event_id` of the currently selected event in Lineage mode,
/// or `None` when not in that mode or the event list is empty.
pub fn lineage_selected_event_id(state: &TracerState) -> Option<i64> {
    if let TracerMode::Lineage(ref view) = state.mode {
        view.snapshot
            .events
            .get(view.selected_event)
            .map(|e| e.event_id)
    } else {
        None
    }
}

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
        ListNavigation::move_down(view);
    }
}

/// Moves the selection up by one row in LatestEvents mode, wrapping at the start.
pub fn latest_events_move_up(state: &mut TracerState) {
    if let TracerMode::LatestEvents(ref mut view) = state.mode {
        ListNavigation::move_up(view);
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
        cluster_node_id: None,
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
        query_id,
        cluster_node_id,
        abort,
        ..
    }) = &mut state.mode
    {
        if let Some(handle) = abort.take() {
            handle.abort();
        }
        if !query_id.is_empty() {
            followup = Some(Followup::DeleteLineageQuery {
                query_id: std::mem::take(query_id),
                cluster_node_id: cluster_node_id.take(),
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
        TracerPayload::LineageSubmitted {
            uuid,
            query_id,
            cluster_node_id,
        } => {
            if let TracerMode::LineageRunning(ref mut running) = state.mode
                && running.uuid == uuid
                && running.query_id.is_empty()
            {
                running.query_id = query_id;
                running.cluster_node_id = cluster_node_id;
            }
            None
        }
        TracerPayload::LineagePartial { query_id, percent } => {
            if let TracerMode::LineageRunning(ref mut running) = state.mode
                && (running.query_id == query_id || running.query_id.is_empty())
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
                && (running.query_id == query_id || running.query_id.is_empty())
            {
                let cluster_node_id = running.cluster_node_id.clone();
                state.mode = TracerMode::Lineage(Box::new(LineageView {
                    uuid,
                    snapshot,
                    selected_event: 0,
                    event_detail: EventDetail::default(),
                    loaded_details: std::collections::HashMap::new(),
                    diff_mode: AttributeDiffMode::default(),
                    fetched_at,
                    focus: LineageFocus::default(),
                    active_detail_tab: DetailTab::default(),
                }));
                return Some(Followup::DeleteLineageQuery {
                    query_id,
                    cluster_node_id,
                });
            }
            // Stale query_id — still emit delete so it gets cleaned up.
            // No cluster_node_id available for stale queries.
            Some(Followup::DeleteLineageQuery {
                query_id,
                cluster_node_id: None,
            })
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
        TracerPayload::EventDetail { event_id, detail } => {
            if let TracerMode::Lineage(ref mut view) = state.mode {
                // Always cache — used to enrich all timeline rows with
                // attribute-change and content indicators as the user scrolls.
                view.loaded_details.insert(event_id, (*detail).clone());

                let selected_id = view
                    .snapshot
                    .events
                    .get(view.selected_event)
                    .map(|e| e.event_id);
                if selected_id == Some(event_id) {
                    view.event_detail = EventDetail::Loaded {
                        event: detail,
                        content: ContentPane::default(),
                    };
                }
            }
            None
        }
        TracerPayload::EventDetailFailed { event_id, error } => {
            if let TracerMode::Lineage(ref mut view) = state.mode {
                let selected_id = view
                    .snapshot
                    .events
                    .get(view.selected_event)
                    .map(|e| e.event_id);
                if selected_id == Some(event_id) {
                    view.event_detail = EventDetail::Failed(error);
                    view.focus = LineageFocus::Timeline;
                }
            }
            None
        }
        TracerPayload::Content(snap) => {
            if let TracerMode::Lineage(ref mut view) = state.mode
                && let EventDetail::Loaded {
                    ref event,
                    ref mut content,
                } = view.event_detail
                && event.summary.event_id == snap.event_id
            {
                *content = ContentPane::Shown {
                    side: snap.side,
                    render: snap.render,
                    bytes_fetched: snap.bytes_fetched,
                    truncated: snap.truncated,
                };
                // Reset scroll if the user was already focused on the
                // content pane — the new payload replaces the old.
                if let LineageFocus::Content { ref mut scroll } = view.focus {
                    *scroll = 0;
                }
            }
            None
        }
        TracerPayload::ContentFailed {
            event_id,
            side: _,
            error,
        } => {
            if let TracerMode::Lineage(ref mut view) = state.mode
                && let EventDetail::Loaded {
                    ref event,
                    ref mut content,
                } = view.event_detail
                && event.summary.event_id == event_id
            {
                *content = ContentPane::Failed(error);
            }
            None
        }
        TracerPayload::ContentSaved { .. } => None,
        TracerPayload::ContentSaveFailed { .. } => None,
        TracerPayload::ModalChunk { .. } => None,
        TracerPayload::ModalChunkFailed { .. } => None,
        // Handled upstream in state/mod.rs before reaching apply_payload.
        TracerPayload::ContentDecoded { .. } => None,
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

/// Returns the current UUID input field value, or an empty string when not
/// in Entry mode.
pub fn entry_value(state: &TracerState) -> &str {
    if let TracerMode::Entry(EntryState { input }) = &state.mode {
        input.as_str()
    } else {
        ""
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

// Modal types, constants, and reducers live in `modal_state`. Re-exported
// here so existing `crate::view::tracer::state::*` import paths still work.
pub use crate::view::tracer::modal_state::*;

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests;
