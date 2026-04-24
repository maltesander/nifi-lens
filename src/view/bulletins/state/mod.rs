//! Pure state for the Bulletins tab.
//!
//! Everything here is synchronous and no-I/O. The fetcher lives in
//! `ClusterStore` — `redraw_bulletins` mirrors its ring + meta into
//! `BulletinsState` on every `ClusterChanged(Bulletins)` event.

use std::collections::{HashSet, VecDeque};
use std::time::{Duration, SystemTime};

use crate::app::navigation::ListNavigation;
use crate::app::state::AppState;
use crate::client::BulletinSnapshot;
use crate::widget::scroll::VerticalScrollState;
pub use crate::widget::search::{MatchSpan, SearchState, compute_matches};

/// Strip NiFi's `ComponentName[id=<uuid>] ` boilerplate prefix from a
/// bulletin message. NiFi emits this prefix on every bulletin; it eats
/// ~50 characters of horizontal space and hides the signal.
///
/// Returns the suffix after the first `[id=<anything>] ` group found at
/// the start of the string (after an arbitrary name). On any mismatch
/// (no `[id=`, no matching `]`, no trailing space) the full original
/// string is returned.
pub fn strip_component_prefix(msg: &str) -> &str {
    let Some(id_start) = msg.find("[id=") else {
        return msg;
    };
    // Everything before `[id=` is the component name. Any content is fine;
    // we don't validate it.
    let after_id = &msg[id_start + "[id=".len()..];
    let Some(close_rel) = after_id.find(']') else {
        return msg;
    };
    let rest = &after_id[close_rel + 1..];
    // Require exactly one trailing ASCII whitespace after the `]`.
    let mut chars = rest.chars();
    match chars.next() {
        Some(c) if c.is_ascii_whitespace() => chars.as_str(),
        _ => msg,
    }
}

/// Replace each `[...]` region in `s` with `[…]` (U+2026). Used to
/// normalize dynamic content like `FlowFile[filename=...]` and
/// `StandardFlowFileRecord[uuid=...]` so that bulletins from the same
/// processor with the same message *shape* collapse into a single
/// dedup bucket regardless of per-flowfile attribute values.
///
/// Non-nesting: only the first `]` after each `[` closes the region.
/// If any `[` in the input has no matching `]` in the remainder, the
/// full original string is returned unchanged — a conservative choice
/// matching the "return verbatim on malformed input" style of
/// `strip_component_prefix`.
pub fn normalize_dynamic_brackets(s: &str) -> String {
    let ellipsis = "[\u{2026}]";
    // Fast path: nothing to do if there's no `[` at all.
    if !s.contains('[') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len());
    let mut i = 0;
    let bytes = s.as_bytes();
    while i < bytes.len() {
        if bytes[i] == b'[' {
            match s[i + 1..].find(']') {
                Some(rel) => {
                    out.push_str(ellipsis);
                    i = i + 1 + rel + 1;
                }
                None => {
                    // Unclosed bracket — bail out, return the original
                    // input verbatim.
                    return s.to_string();
                }
            }
            continue;
        }
        // Append up to the next `[` or end-of-string in one slice.
        let next = s[i..].find('[').map(|rel| i + rel).unwrap_or(bytes.len());
        out.push_str(&s[i..next]);
        i = next;
    }
    out
}

/// Return up to `limit` most-recent bulletins from `ring` whose
/// `source_id` matches `source_id`, in newest-first order.
///
/// The ring is monotonically append-only, so the back is newest.
/// This walks in reverse and filters.
pub fn recent_for_source_id<'a>(
    ring: &'a VecDeque<BulletinSnapshot>,
    source_id: &str,
    limit: usize,
) -> Vec<&'a BulletinSnapshot> {
    if limit == 0 {
        return Vec::new();
    }
    ring.iter()
        .rev()
        .filter(|b| b.source_id == source_id)
        .take(limit)
        .collect()
}

/// Return up to `limit` most-recent bulletins from `ring` whose
/// `group_id` matches `group_id`, in newest-first order. Same
/// iteration pattern as [`recent_for_source_id`].
pub fn recent_for_group_id<'a>(
    ring: &'a VecDeque<BulletinSnapshot>,
    group_id: &str,
    limit: usize,
) -> Vec<&'a BulletinSnapshot> {
    if limit == 0 {
        return Vec::new();
    }
    ring.iter()
        .rev()
        .filter(|b| b.group_id == group_id)
        .take(limit)
        .collect()
}

#[derive(Debug)]
pub struct BulletinsState {
    /// Rendered view of the cluster-owned `BulletinRing`. Mirrored by
    /// `redraw_bulletins` on every `ClusterChanged(Bulletins)` event —
    /// the canonical ring lives at `AppState.cluster.snapshot.bulletins`.
    /// The copy lets render helpers and Browser's detail sub-panels
    /// keep a `&VecDeque<BulletinSnapshot>` handle (via `&state.bulletins.ring`)
    /// without needing to take an extra parameter.
    pub ring: VecDeque<BulletinSnapshot>,
    pub ring_capacity: usize,
    pub last_fetched_at: Option<SystemTime>,
    pub filters: FilterState,
    /// Session-scoped mute list. `row_matches` filters out any bulletin
    /// whose `source_id` is in this set. Toggled by `m` in the key
    /// handler; not persisted to config.
    pub mutes: HashSet<String>,
    /// `Some(buf)` while in text-input mode. Every keystroke mutates the
    /// buffer and live-updates `filters.text`. On commit, the buffer is
    /// copied into `filters.text`. On cancel, `pre_input_text` is restored.
    pub text_input: Option<String>,
    /// Snapshot of `filters.text` captured on `enter_text_input_mode`, so
    /// `cancel_text_input` can undo the live edits. `None` when not in mode.
    pub pre_input_text: Option<String>,
    /// Selection within the *filtered* list.
    pub selected: usize,
    pub auto_scroll: bool,
    pub new_since_pause: u32,
    pub group_mode: GroupMode,
    /// `Some(modal)` while the detail modal is open. Opened by `i`,
    /// closed by `Esc` or by `Enter` (which also emits the cross-link).
    pub detail_modal: Option<DetailModalState>,
}

#[derive(Debug, Clone)]
pub struct FilterState {
    pub show_error: bool,
    pub show_warning: bool,
    pub show_info: bool,
    pub component_type: Option<ComponentType>,
    pub text: String,
}

impl Default for FilterState {
    fn default() -> Self {
        Self {
            show_error: true,
            show_warning: true,
            show_info: true,
            component_type: None,
            text: String::new(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ComponentType {
    Processor,
    ControllerService,
    ReportingTask,
    Other,
}

impl ComponentType {
    /// Bucket a NiFi `source_type` string into a chip.
    pub fn classify(source_type: &str) -> Self {
        match source_type.to_ascii_uppercase().as_str() {
            "PROCESSOR" => Self::Processor,
            "CONTROLLER_SERVICE" => Self::ControllerService,
            "REPORTING_TASK" => Self::ReportingTask,
            _ => Self::Other,
        }
    }
}

/// How rows are folded in the Bulletins list.
///
/// Cycle order is `SourceAndMessage` → `Source` → `Off` → wrap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupMode {
    /// Dedup by `(source_id, strip_component_prefix(message))`. Default.
    /// This is the noise-killer: one row per unique error message from
    /// each source, with a count column.
    SourceAndMessage,
    /// Collapse every bulletin from a single source into one row. The
    /// displayed message is the stripped form of the most recent
    /// bulletin.
    Source,
    /// No folding. One row per bulletin in chronological order.
    Off,
}

impl GroupMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::SourceAndMessage => "source+msg",
            Self::Source => "source",
            Self::Off => "off",
        }
    }

    pub fn cycle(self) -> Self {
        match self {
            Self::SourceAndMessage => Self::Source,
            Self::Source => Self::Off,
            Self::Off => Self::SourceAndMessage,
        }
    }
}

/// A row in the grouped display. Produced by
/// [`BulletinsState::grouped_view`] when `group_mode` is not `Off`,
/// or (implicitly, as a vec of `count = 1` singletons) when flat.
///
/// Ring indices are stable for the lifetime of the ring buffer — the
/// render layer dereferences them via `state.ring[latest_ring_idx]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupedRow {
    /// Ring index of the first bulletin in the run (oldest in the
    /// group). Used by `cycle_group_mode` to find which group contains
    /// a given pre-toggle ring index.
    pub first_ring_idx: usize,
    /// Ring index of the most-recent bulletin in the run. Render uses
    /// this to fetch the displayed timestamp, source, and message.
    pub latest_ring_idx: usize,
    /// Number of bulletins folded into this group. `1` means no
    /// grouping occurred; render skips the `[×N]` prefix.
    pub count: usize,
}

/// Raw counts by severity for the current ring. Used to render the
/// count-carrying chips (`[E 87]`). Ignores component-type, text, and
/// mute filters by design — the chip tells the user how many rows of
/// that severity *exist*, not how many are currently visible.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SeverityCounts {
    pub error: usize,
    pub warning: usize,
    pub info: usize,
}

/// Rendered data for the currently selected group. Returned by
/// [`BulletinsState::group_details`]; rendered in the bottom detail
/// pane.
#[derive(Debug, Clone)]
pub struct GroupDetails {
    pub count: usize,
    pub first_seen_iso: String,
    pub last_seen_iso: String,
    pub source_name: String,
    pub source_id: String,
    pub source_type: String,
    pub group_id: String,
    /// Most-recent stripped message (`strip_component_prefix(raw_message)`).
    pub stripped_message: String,
    /// Most-recent raw message, prefix included. The detail pane
    /// shows this verbatim so the user can inspect what NiFi emitted.
    pub raw_message: String,
    pub severity: crate::client::Severity,
}

/// A group's identity, captured when the user opens the detail modal
/// so we can re-resolve the same bulletin against a ring that may
/// have shifted (eviction, new arrivals).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupKey {
    pub source_id: String,
    /// `strip_component_prefix(message)` + `normalize_dynamic_brackets`.
    /// Empty string when `mode == Off` or `mode == Source` (dedup key
    /// doesn't use the message).
    pub message_stem: String,
    pub mode: GroupMode,
}

#[derive(Debug, Clone)]
pub struct DetailModalState {
    /// Identity of the group at the time of open. Used to re-resolve
    /// and defensive-close if the ring no longer contains it.
    pub group_key: GroupKey,
    /// Snapshot of the group's details at open time. If the ring
    /// changes the underlying data, the snapshot keeps the modal
    /// coherent until the user closes it.
    pub details: GroupDetails,
    /// Vertical scroll state (offset + last viewport rows). Scroll
    /// offset is in wrapped-line units and saturates on render if
    /// the renderer's clamped max is smaller. `last_viewport_rows`
    /// is written by the renderer each frame; reducers read it to
    /// compute page-size scrolls. Initial value `0` means page
    /// scrolls behave like line scrolls until the first render —
    /// harmless because the modal renders before the user can press
    /// a key.
    pub scroll: VerticalScrollState,
    pub search: Option<SearchState>,
}

impl BulletinsState {
    pub fn with_capacity(ring_capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(ring_capacity),
            ring_capacity,
            last_fetched_at: None,
            filters: FilterState::default(),
            mutes: HashSet::new(),
            text_input: None,
            pre_input_text: None,
            selected: 0,
            auto_scroll: true,
            new_since_pause: 0,
            group_mode: GroupMode::SourceAndMessage,
            detail_modal: None,
        }
    }

    /// Walk the ring once; return ring indices where the row matches all
    /// active filters.
    pub fn filtered_indices(&self) -> Vec<usize> {
        self.ring
            .iter()
            .enumerate()
            .filter(|(_, b)| self.row_matches(b))
            .map(|(i, _)| i)
            .collect()
    }

    /// Fold the filtered ring into display rows according to `group_mode`.
    ///
    /// - `Off`: one row per filtered bulletin.
    /// - `SourceAndMessage`: dedup by `(source_id, strip_component_prefix(message))`.
    ///   Groups appear in order of first-seen ring index.
    /// - `Source`: fold all bulletins from a single `source_id`. Groups
    ///   appear in order of first-seen source.
    pub fn grouped_view(&self) -> Vec<GroupedRow> {
        let filtered = self.filtered_indices();
        if self.group_mode == GroupMode::Off {
            return filtered
                .iter()
                .map(|&ring_idx| GroupedRow {
                    first_ring_idx: ring_idx,
                    latest_ring_idx: ring_idx,
                    count: 1,
                })
                .collect();
        }
        // For both dedup modes we walk filtered rows in order and keep a
        // map from key → position-in-`out` so duplicates fold into the
        // existing row while preserving first-seen order.
        let mut out: Vec<GroupedRow> = Vec::new();
        let mut positions: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for &ring_idx in &filtered {
            let b = &self.ring[ring_idx];
            let key = match self.group_mode {
                GroupMode::SourceAndMessage => {
                    let stem = strip_component_prefix(&b.message);
                    let normalized = normalize_dynamic_brackets(stem);
                    format!("{}\x1f{normalized}", b.source_id)
                }
                GroupMode::Source => b.source_id.clone(),
                GroupMode::Off => unreachable!("handled above"),
            };
            match positions.get(&key) {
                Some(&pos) => {
                    let group = &mut out[pos];
                    group.latest_ring_idx = ring_idx;
                    group.count += 1;
                }
                None => {
                    positions.insert(key, out.len());
                    out.push(GroupedRow {
                        first_ring_idx: ring_idx,
                        latest_ring_idx: ring_idx,
                        count: 1,
                    });
                }
            }
        }
        out
    }

    pub fn selected_ring_index(&self) -> Option<usize> {
        self.grouped_view()
            .get(self.selected)
            .map(|g| g.latest_ring_idx)
    }

    fn row_matches(&self, b: &BulletinSnapshot) -> bool {
        // Severity. `Unknown` rides with the Info chip by design.
        let sev = crate::client::Severity::parse(&b.level);
        let severity_ok = match sev {
            crate::client::Severity::Error => self.filters.show_error,
            crate::client::Severity::Warning => self.filters.show_warning,
            crate::client::Severity::Info | crate::client::Severity::Unknown => {
                self.filters.show_info
            }
        };
        if !severity_ok {
            return false;
        }
        if self.mutes.contains(&b.source_id) {
            return false;
        }
        if let Some(want) = self.filters.component_type
            && ComponentType::classify(&b.source_type) != want
        {
            return false;
        }
        if !self.filters.text.is_empty() {
            let needle = self.filters.text.to_lowercase();
            let hay_message = b.message.to_lowercase();
            let hay_source = b.source_name.to_lowercase();
            if !hay_message.contains(&needle) && !hay_source.contains(&needle) {
                return false;
            }
        }
        true
    }

    // ---- filter mutations ----
    //
    // Every mutator that changes filter visibility accepts or captures a
    // `prev_ring_index` — the ring index the user's selection pointed at
    // BEFORE the mutation. Callers that construct intents from key events
    // must capture this value (via `self.selected_ring_index()`) *before*
    // they invoke the mutator, not after. `reconcile_selection` uses it
    // to snap the visible-list selection to the nearest still-visible row
    // when the previously selected row has been filtered out.

    pub fn toggle_error(&mut self) {
        let prev = self.selected_ring_index();
        self.filters.show_error = !self.filters.show_error;
        self.reconcile_selection(prev);
    }
    pub fn toggle_warning(&mut self) {
        let prev = self.selected_ring_index();
        self.filters.show_warning = !self.filters.show_warning;
        self.reconcile_selection(prev);
    }
    pub fn toggle_info(&mut self) {
        let prev = self.selected_ring_index();
        self.filters.show_info = !self.filters.show_info;
        self.reconcile_selection(prev);
    }
    pub fn cycle_component_type(&mut self) {
        let prev = self.selected_ring_index();
        self.filters.component_type = match self.filters.component_type {
            None => Some(ComponentType::Processor),
            Some(ComponentType::Processor) => Some(ComponentType::ControllerService),
            Some(ComponentType::ControllerService) => Some(ComponentType::ReportingTask),
            Some(ComponentType::ReportingTask) => Some(ComponentType::Other),
            Some(ComponentType::Other) => None,
        };
        self.reconcile_selection(prev);
    }
    pub fn clear_filters(&mut self) {
        let prev = self.selected_ring_index();
        self.filters = FilterState::default();
        self.mutes.clear();
        self.reconcile_selection(prev);
    }

    // ---- text input mode ----

    pub fn enter_text_input_mode(&mut self) {
        self.pre_input_text = Some(self.filters.text.clone());
        self.text_input = Some(self.filters.text.clone());
    }
    /// Append `ch` to the text-input buffer, live-updating `filters.text`
    /// and snapping `selected` via `reconcile_selection`.
    ///
    /// # Preconditions
    /// `prev_ring_index` must be captured from `selected_ring_index()`
    /// BEFORE this call. Passing a value computed after mutation will
    /// snap to the wrong row.
    pub fn push_text_input(&mut self, ch: char, prev_ring_index: Option<usize>) {
        if let Some(buf) = self.text_input.as_mut() {
            buf.push(ch);
            self.filters.text = buf.clone();
            self.reconcile_selection(prev_ring_index);
        }
    }
    /// Remove the last character from the text-input buffer, live-updating
    /// `filters.text` and snapping `selected` via `reconcile_selection`.
    ///
    /// # Preconditions
    /// `prev_ring_index` must be captured from `selected_ring_index()`
    /// BEFORE this call. Passing a value computed after mutation will
    /// snap to the wrong row.
    pub fn pop_text_input(&mut self, prev_ring_index: Option<usize>) {
        if let Some(buf) = self.text_input.as_mut() {
            buf.pop();
            self.filters.text = buf.clone();
            self.reconcile_selection(prev_ring_index);
        }
    }
    /// Commit the text-input buffer as the active filter and exit input mode.
    ///
    /// # Preconditions
    /// `prev_ring_index` must be captured from `selected_ring_index()`
    /// BEFORE this call. Passing a value computed after mutation will
    /// snap to the wrong row.
    pub fn commit_text_input(&mut self, prev_ring_index: Option<usize>) {
        if let Some(buf) = self.text_input.take() {
            self.pre_input_text = None;
            self.filters.text = buf.trim().to_string();
            self.reconcile_selection(prev_ring_index);
        }
    }
    /// Discard the text-input buffer and restore the pre-input filter text.
    ///
    /// # Preconditions
    /// `prev_ring_index` must be captured from `selected_ring_index()`
    /// BEFORE this call. Passing a value computed after mutation will
    /// snap to the wrong row.
    pub fn cancel_text_input(&mut self, prev_ring_index: Option<usize>) {
        if self.text_input.take().is_some() {
            let restored = self.pre_input_text.take().unwrap_or_default();
            self.filters.text = restored;
            self.reconcile_selection(prev_ring_index);
        }
    }

    /// Returns the current text-input buffer value, or `None` when not in
    /// text-input mode.
    pub fn text_input_value(&self) -> Option<&str> {
        self.text_input.as_deref()
    }

    // ---- navigation / pause ----

    pub fn move_selection_up(&mut self) {
        let prev = self.selected;
        ListNavigation::move_up(self);
        if self.selected != prev {
            self.auto_scroll = false;
        }
    }
    pub fn move_selection_down(&mut self) {
        ListNavigation::move_down(self);
        let vis_len = self.grouped_view().len();
        if vis_len > 0 && self.selected == vis_len - 1 {
            self.auto_scroll = true;
            self.new_since_pause = 0;
        }
    }
    pub fn goto_oldest(&mut self) {
        ListNavigation::goto_first(self);
        self.auto_scroll = false;
    }
    pub fn goto_newest(&mut self) {
        ListNavigation::goto_last(self);
        self.auto_scroll = true;
        self.new_since_pause = 0;
    }
    pub fn toggle_pause(&mut self) {
        self.auto_scroll = !self.auto_scroll;
        if self.auto_scroll {
            let max = self.grouped_view().len().saturating_sub(1);
            self.selected = max;
            self.new_since_pause = 0;
        }
    }

    /// Called whenever filters or grouping change. Snap `selected` to
    /// the group that contains `prev_ring_idx`, or the nearest surviving
    /// group if that bulletin has been filtered out. Callers capture the
    /// ring index *before* mutating filters/grouping and pass it in.
    pub fn reconcile_selection(&mut self, prev_ring_index: Option<usize>) {
        let groups = self.grouped_view();
        if groups.is_empty() {
            self.selected = 0;
            return;
        }
        if let Some(prev) = prev_ring_index {
            // Exact: any group whose run contains `prev`.
            if let Some(pos) = groups
                .iter()
                .position(|g| g.first_ring_idx <= prev && prev <= g.latest_ring_idx)
            {
                self.selected = pos;
                return;
            }
            // Nearest older: last group entirely before `prev`.
            if let Some(pos) = groups.iter().rposition(|g| g.latest_ring_idx < prev) {
                self.selected = pos;
                return;
            }
            // Nearest newer: first group entirely after `prev`.
            if let Some(pos) = groups.iter().position(|g| g.first_ring_idx > prev) {
                self.selected = pos;
                return;
            }
        }
        self.selected = if self.auto_scroll {
            groups.len() - 1
        } else {
            0
        };
    }

    /// Cycle through the group-by modes. Captures the previously selected
    /// ring index so `reconcile_selection` can keep the user on the same
    /// logical bulletin across the toggle.
    pub fn cycle_group_mode(&mut self) {
        let prev = self.selected_ring_index();
        self.group_mode = self.group_mode.cycle();
        self.reconcile_selection(prev);
    }

    /// Toggle the mute state for the currently selected row's
    /// `source_id`. If the row is already muted this unmutes it
    /// (should be unreachable while the row is hidden, but defensive).
    pub fn mute_selected_source(&mut self) {
        let Some(ring_idx) = self.selected_ring_index() else {
            return;
        };
        let prev = Some(ring_idx);
        let source_id = self.ring[ring_idx].source_id.clone();
        if !self.mutes.insert(source_id.clone()) {
            self.mutes.remove(&source_id);
        }
        self.reconcile_selection(prev);
    }
}

impl ListNavigation for BulletinsState {
    fn list_len(&self) -> usize {
        self.grouped_view().len()
    }

    fn selected(&self) -> Option<usize> {
        if self.grouped_view().is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }
}

impl BulletinsState {
    pub fn severity_counts(&self) -> SeverityCounts {
        let mut out = SeverityCounts::default();
        for b in &self.ring {
            match crate::client::Severity::parse(&b.level) {
                crate::client::Severity::Error => out.error += 1,
                crate::client::Severity::Warning => out.warning += 1,
                crate::client::Severity::Info | crate::client::Severity::Unknown => out.info += 1,
            }
        }
        out
    }

    pub fn group_details(&self) -> Option<GroupDetails> {
        let group = self.grouped_view().into_iter().nth(self.selected)?;
        let first = &self.ring[group.first_ring_idx];
        let latest = &self.ring[group.latest_ring_idx];
        let stripped = strip_component_prefix(&latest.message).to_string();
        Some(GroupDetails {
            count: group.count,
            first_seen_iso: first.timestamp_iso.clone(),
            last_seen_iso: latest.timestamp_iso.clone(),
            source_name: latest.source_name.clone(),
            source_id: latest.source_id.clone(),
            source_type: latest.source_type.clone(),
            group_id: latest.group_id.clone(),
            stripped_message: stripped,
            raw_message: latest.message.clone(),
            severity: crate::client::Severity::parse(&latest.level),
        })
    }

    /// Build a `GroupKey` for the currently selected group, or `None`
    /// when the list is empty.
    pub fn selected_group_key(&self) -> Option<GroupKey> {
        let group = self.grouped_view().into_iter().nth(self.selected)?;
        let latest = &self.ring[group.latest_ring_idx];
        let message_stem = match self.group_mode {
            GroupMode::SourceAndMessage => {
                normalize_dynamic_brackets(strip_component_prefix(&latest.message))
            }
            GroupMode::Source | GroupMode::Off => String::new(),
        };
        Some(GroupKey {
            source_id: latest.source_id.clone(),
            message_stem,
            mode: self.group_mode,
        })
    }

    /// Open the detail modal for the currently selected group.
    /// Returns `true` if opened, `false` if there is nothing selected.
    pub fn open_detail_modal(&mut self) -> bool {
        let Some(details) = self.group_details() else {
            return false;
        };
        let Some(group_key) = self.selected_group_key() else {
            return false;
        };
        self.detail_modal = Some(DetailModalState {
            group_key,
            details,
            scroll: VerticalScrollState::default(),
            search: None,
        });
        true
    }

    /// Close the detail modal, clearing all modal-local state.
    pub fn close_detail_modal(&mut self) {
        self.detail_modal = None;
    }

    /// Scroll the modal body by `delta` lines (negative = up). Clamps
    /// the lower bound at 0; does not clamp upward (renderer clamps
    /// against the real wrap-aware max each frame).
    pub fn modal_scroll_by(&mut self, delta: i32) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        // `usize::MAX` as content_rows effectively disables upward
        // clamping — the renderer performs the real wrap-aware clamp.
        modal.scroll.scroll_by(delta, usize::MAX);
    }

    pub fn modal_page_down(&mut self) {
        if let Some(modal) = self.detail_modal.as_mut() {
            // `usize::MAX` as content_rows disables the widget's upper
            // clamp; the renderer performs the real wrap-aware clamp.
            modal.scroll.page_down(usize::MAX);
        }
    }

    pub fn modal_page_up(&mut self) {
        if let Some(modal) = self.detail_modal.as_mut() {
            modal.scroll.page_up();
        }
    }

    pub fn modal_jump_top(&mut self) {
        if let Some(modal) = self.detail_modal.as_mut() {
            modal.scroll.jump_top();
        }
    }

    /// Passes `usize::MAX` as content_rows so the offset lands beyond
    /// any real maximum; the renderer clamps against the true
    /// wrap-aware maximum on the next frame. State-level reducer has
    /// no access to viewport-derived maxima.
    pub fn modal_jump_bottom(&mut self) {
        if let Some(modal) = self.detail_modal.as_mut() {
            modal.scroll.jump_bottom(usize::MAX);
        }
    }

    /// Returns the full raw message for the currently open modal, or
    /// `None` if no modal is open. Caller is responsible for pushing
    /// to the clipboard and posting a status banner.
    pub fn modal_copy_message(&self) -> Option<String> {
        self.detail_modal
            .as_ref()
            .map(|m| m.details.raw_message.clone())
    }

    /// Open the search input inside the detail modal. Initialises
    /// `modal.search` with an empty query and `input_active = true`.
    /// No-op if no modal is open.
    pub fn modal_search_open(&mut self) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        modal.search = Some(SearchState {
            query: String::new(),
            input_active: true,
            committed: false,
            matches: Vec::new(),
            current: None,
        });
    }

    /// Append a character to the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn modal_search_push(&mut self, ch: char) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.push(ch);
        search.matches = compute_matches(&modal.details.raw_message, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Remove the last character from the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn modal_search_pop(&mut self) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.pop();
        search.matches = compute_matches(&modal.details.raw_message, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Commit the current query. If the query is empty, closes search
    /// (sets `modal.search = None`). Otherwise flips `input_active` to
    /// false and `committed` to true.
    pub fn modal_search_commit(&mut self) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if search.query.is_empty() {
            modal.search = None;
            return;
        }
        search.input_active = false;
        search.committed = true;
    }

    /// Cancel search and clear all search state from the modal.
    pub fn modal_search_cancel(&mut self) {
        if let Some(modal) = self.detail_modal.as_mut() {
            modal.search = None;
        }
    }

    /// Advance to the next match, wrapping around. No-op unless search
    /// is committed and has at least one match.
    pub fn modal_search_cycle_next(&mut self) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some((cur + 1) % search.matches.len());
    }

    /// Move to the previous match, wrapping around. No-op unless search
    /// is committed and has at least one match.
    pub fn modal_search_cycle_prev(&mut self) {
        let Some(modal) = self.detail_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some(if cur == 0 {
            search.matches.len() - 1
        } else {
            cur - 1
        });
    }
}

/// Mirror the cluster-owned `BulletinRing` into `BulletinsState`,
/// advancing the new-since-pause badge and keeping the selection
/// anchored on the same logical row. Called from the `ClusterChanged(Bulletins)`
/// arm in `src/app/mod.rs` after `ClusterStore` merges a fresh batch.
///
/// Semantics (preserved from the pre-Task-7 `apply_payload`):
/// - The canonical ring is `state.cluster.snapshot.bulletins.buf`;
///   `state.bulletins.ring` is a one-way mirror.
/// - `new_since_pause` counts only bulletins added since the last
///   mirror that match the current filters (paused mode).
/// - `auto_scroll` snaps `selected` to the newest group after mirroring.
/// - `last_fetched_at` is derived from the cluster meta's `Instant`
///   anchor (converted into wall-clock via the elapsed delta so the
///   renderer can reuse its existing `SystemTime::now() - fetched`
///   "last Ns ago" formula).
pub fn redraw_bulletins(state: &mut AppState) {
    let cluster_ring = &state.cluster.snapshot.bulletins;
    let before_ids: HashSet<i64> = state.bulletins.ring.iter().map(|b| b.id).collect();

    // Recompute the mirror from scratch — cluster-ring is the source
    // of truth. Copy-then-mutate is cheap: at the default ring_size of
    // 5000, one `BulletinSnapshot` clone is a few hundred bytes.
    let mut new_ring: VecDeque<BulletinSnapshot> = VecDeque::with_capacity(cluster_ring.buf.len());
    new_ring.extend(cluster_ring.buf.iter().cloned());

    // Count matching *new* rows for the +N badge (paused mode) BEFORE
    // swapping the mirror so filter predicates evaluate against the
    // current state's mute/filter set.
    let mut new_matching = 0u32;
    if !state.bulletins.auto_scroll {
        for b in new_ring.iter() {
            if !before_ids.contains(&b.id) && state.bulletins.row_matches(b) {
                new_matching = new_matching.saturating_add(1);
            }
        }
    }

    state.bulletins.ring = new_ring;

    // Derive a `SystemTime` anchor for the renderer. The cluster meta
    // records the fetch request start as a monotonic `Instant`; we map
    // it to wall-clock by subtracting the elapsed delta from `now`.
    // The `min(86400s)` clamp prevents a degenerate elapsed value
    // (e.g. from a test harness with a manipulated clock) producing an
    // underflow on the `SystemTime - Duration` subtraction.
    if let Some(meta) = cluster_ring.meta.as_ref() {
        state.bulletins.last_fetched_at =
            Some(SystemTime::now() - meta.fetched_at.elapsed().min(Duration::from_secs(86400)));
    }

    if state.bulletins.auto_scroll {
        let max = state.bulletins.grouped_view().len().saturating_sub(1);
        state.bulletins.selected = max;
        state.bulletins.new_since_pause = 0;
    } else {
        state.bulletins.new_since_pause =
            state.bulletins.new_since_pause.saturating_add(new_matching);
    }
}

#[cfg(test)]
mod tests;
