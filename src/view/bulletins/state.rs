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
mod tests {
    use super::*;
    use crate::client::BulletinSnapshot;
    use std::time::{Duration, UNIX_EPOCH};

    const T0: u64 = 1_775_902_462; // 2026-04-11T10:14:22Z

    fn b(id: i64, level: &str) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: level.into(),
            message: format!("msg-{id}"),
            source_id: format!("src-{id}"),
            source_name: format!("Proc-{id}"),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        }
    }

    /// Test-only helper that mimics the pre-Task-7 `apply_payload` on
    /// a bare `BulletinsState`. Production code goes through
    /// `redraw_bulletins(&mut AppState)`; these reducer-shape tests
    /// don't need the full `AppState` stack.
    fn apply_payload_test(state: &mut BulletinsState, bulletins: Vec<BulletinSnapshot>) {
        let before_len = state.ring.len();
        let existing_ids: HashSet<i64> = state.ring.iter().map(|b| b.id).collect();
        for bulletin in bulletins {
            if existing_ids.contains(&bulletin.id) {
                continue;
            }
            state.ring.push_back(bulletin);
        }
        let mut new_matching = 0u32;
        if !state.auto_scroll {
            for bulletin in state.ring.iter().skip(before_len) {
                if state.row_matches(bulletin) {
                    new_matching = new_matching.saturating_add(1);
                }
            }
        }
        while state.ring.len() > state.ring_capacity {
            state.ring.pop_front();
        }
        state.last_fetched_at = Some(UNIX_EPOCH + Duration::from_secs(T0));
        if state.auto_scroll {
            let max = state.grouped_view().len().saturating_sub(1);
            state.selected = max;
            state.new_since_pause = 0;
        } else {
            state.new_since_pause = state.new_since_pause.saturating_add(new_matching);
        }
    }

    #[test]
    fn apply_payload_seeds_empty_ring_with_initial_batch() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]);
        assert_eq!(s.ring.len(), 3);
        assert_eq!(s.ring[0].id, 1);
        assert_eq!(s.ring[2].id, 3);
        assert!(s.last_fetched_at.is_some());
    }

    #[test]
    fn apply_payload_dedups_on_id() {
        // Cursor-based dedup is now the cluster ring's job; this shim
        // just filters out ids already in the mirror, matching render
        // expectations.
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        apply_payload_test(&mut s, vec![b(2, "INFO"), b(3, "INFO")]);
        assert_eq!(s.ring.len(), 3);
    }

    #[test]
    fn apply_payload_drops_oldest_at_capacity() {
        let mut s = BulletinsState::with_capacity(4);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]);
        apply_payload_test(&mut s, vec![b(4, "INFO"), b(5, "INFO"), b(6, "INFO")]);
        assert_eq!(s.ring.len(), 4);
        assert_eq!(s.ring.front().unwrap().id, 3);
        assert_eq!(s.ring.back().unwrap().id, 6);
    }

    #[test]
    fn apply_payload_empty_batch_is_noop_except_for_fetched_at() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![]);
        assert!(s.ring.is_empty());
        assert!(s.last_fetched_at.is_some());
    }

    #[test]
    fn redraw_bulletins_mirrors_cluster_ring_into_view_state() {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::Instant;
        let mut state = crate::test_support::fresh_state();
        // Seed the cluster ring with 3 bulletins + meta.
        state
            .cluster
            .snapshot
            .bulletins
            .merge(vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]);
        state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(5),
        });
        crate::view::bulletins::state::redraw_bulletins(&mut state);
        assert_eq!(state.bulletins.ring.len(), 3);
        assert_eq!(state.bulletins.ring[0].id, 1);
        assert_eq!(state.bulletins.ring[2].id, 3);
        assert!(state.bulletins.last_fetched_at.is_some());
    }

    #[test]
    fn redraw_bulletins_advances_new_since_pause_when_paused() {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::Instant;
        let mut state = crate::test_support::fresh_state();
        state.bulletins.auto_scroll = false;
        // First mirror: 2 bulletins.
        state
            .cluster
            .snapshot
            .bulletins
            .merge(vec![b(1, "INFO"), b(2, "INFO")]);
        state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(5),
        });
        crate::view::bulletins::state::redraw_bulletins(&mut state);
        let badge_after_first = state.bulletins.new_since_pause;

        // Second mirror: 2 more bulletins — the badge advances by the
        // newly matching rows only.
        state
            .cluster
            .snapshot
            .bulletins
            .merge(vec![b(3, "INFO"), b(4, "INFO")]);
        crate::view::bulletins::state::redraw_bulletins(&mut state);
        assert!(
            state.bulletins.new_since_pause > badge_after_first,
            "new_since_pause must grow as fresh matching rows arrive"
        );
    }

    #[test]
    fn redraw_bulletins_with_grouping_preserves_render_time_dedup() {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::Instant;
        let mut state = crate::test_support::fresh_state();
        // Three bulletins sharing source+message stem collapse to one
        // grouped row under the default `SourceAndMessage` mode.
        let shared = BulletinSnapshot {
            id: 0,
            level: "ERROR".into(),
            message: "Proc[id=p] same stem".into(),
            source_id: "src-same".into(),
            source_name: "Proc".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        };
        let build = |id: i64| BulletinSnapshot {
            id,
            ..shared.clone()
        };
        state
            .cluster
            .snapshot
            .bulletins
            .merge(vec![build(1), build(2), build(3)]);
        state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(5),
        });
        crate::view::bulletins::state::redraw_bulletins(&mut state);
        assert_eq!(state.bulletins.ring.len(), 3);
        let groups = state.bulletins.grouped_view();
        assert_eq!(
            groups.len(),
            1,
            "render-time dedup must fold repeating stems into one group"
        );
        assert_eq!(groups[0].count, 3);
    }

    fn b_full(
        id: i64,
        level: &str,
        source_type: &str,
        source_name: &str,
        message: &str,
    ) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: level.into(),
            message: message.into(),
            source_id: format!("src-{id}"),
            source_name: source_name.into(),
            source_type: source_type.into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        }
    }

    fn seed(capacity: usize, rows: Vec<BulletinSnapshot>) -> BulletinsState {
        let mut s = BulletinsState::with_capacity(capacity);
        apply_payload_test(&mut s, rows);
        s
    }

    #[test]
    fn severity_toggle_removes_matching_rows_from_filtered_view() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "INFO", "PROCESSOR", "A", "info msg"),
                b_full(2, "WARN", "PROCESSOR", "B", "warn msg"),
                b_full(3, "ERROR", "PROCESSOR", "C", "error msg"),
            ],
        );
        assert_eq!(s.filtered_indices().len(), 3);
        s.toggle_error();
        assert_eq!(s.filtered_indices().len(), 2);
        assert!(
            s.filtered_indices()
                .iter()
                .all(|&i| s.ring[i].level != "ERROR")
        );
    }

    #[test]
    fn unknown_severity_rides_with_info_chip() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "DEBUG", "PROCESSOR", "A", "unknown-level"),
                b_full(2, "INFO", "PROCESSOR", "B", "info"),
            ],
        );
        assert_eq!(s.filtered_indices().len(), 2);
        s.toggle_info();
        assert_eq!(
            s.filtered_indices().len(),
            0,
            "toggling off Info also hides Unknown-level rows"
        );
    }

    #[test]
    fn component_type_cycle_advances_through_five_states() {
        let mut s = BulletinsState::with_capacity(100);
        assert_eq!(s.filters.component_type, None);
        s.cycle_component_type();
        assert_eq!(s.filters.component_type, Some(ComponentType::Processor));
        s.cycle_component_type();
        assert_eq!(
            s.filters.component_type,
            Some(ComponentType::ControllerService)
        );
        s.cycle_component_type();
        assert_eq!(s.filters.component_type, Some(ComponentType::ReportingTask));
        s.cycle_component_type();
        assert_eq!(s.filters.component_type, Some(ComponentType::Other));
        s.cycle_component_type();
        assert_eq!(s.filters.component_type, None);
    }

    #[test]
    fn component_type_filter_maps_unknown_source_type_to_other() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "INFO", "PROCESSOR", "A", "m"),
                b_full(2, "INFO", "INPUT_PORT", "B", "m"),
                b_full(3, "INFO", "", "C", "m"),
            ],
        );
        s.filters.component_type = Some(ComponentType::Other);
        let filtered = s.filtered_indices();
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .any(|&i| s.ring[i].source_type == "INPUT_PORT")
        );
        assert!(filtered.iter().any(|&i| s.ring[i].source_type.is_empty()));
    }

    #[test]
    fn text_filter_substring_case_insensitive_matches_message() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "ERROR", "PROCESSOR", "A", "IOException thrown"),
                b_full(2, "ERROR", "PROCESSOR", "B", "other failure"),
            ],
        );
        s.filters.text = "ioex".into();
        let filtered = s.filtered_indices();
        assert_eq!(filtered.len(), 1);
        assert_eq!(s.ring[filtered[0]].id, 1);
    }

    #[test]
    fn text_filter_substring_matches_source_name() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "INFO", "PROCESSOR", "PutDatabase", "ok"),
                b_full(2, "INFO", "PROCESSOR", "PutKafka", "ok"),
            ],
        );
        s.filters.text = "kafka".into();
        let filtered = s.filtered_indices();
        assert_eq!(filtered.len(), 1);
        assert_eq!(s.ring[filtered[0]].id, 2);
    }

    #[test]
    fn clear_filters_resets_all_four_dimensions() {
        let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
        s.toggle_error();
        s.toggle_warning();
        s.cycle_component_type();
        s.filters.text = "xyz".into();
        s.clear_filters();
        assert!(s.filters.show_error);
        assert!(s.filters.show_warning);
        assert!(s.filters.show_info);
        assert_eq!(s.filters.component_type, None);
        assert_eq!(s.filters.text, "");
    }

    #[test]
    fn reconcile_selection_snaps_to_nearest_older_then_newer() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "INFO", "PROCESSOR", "A", "msg-A"),
                b_full(2, "INFO", "PROCESSOR", "B", "msg-B"),
                b_full(3, "ERROR", "PROCESSOR", "C", "msg-C"),
                b_full(4, "INFO", "PROCESSOR", "D", "msg-D"),
                b_full(5, "INFO", "PROCESSOR", "E", "msg-E"),
            ],
        );
        // Selection at filtered index 2 (the ERROR row at ring index 2).
        s.auto_scroll = false;
        s.selected = 2;
        s.toggle_info(); // Hide INFO. Visible ring: [2]. Filtered idx = 0.
        assert_eq!(s.filtered_indices(), vec![2]);
        assert_eq!(s.selected, 0);
        // Restore filters.
        s.clear_filters();
        assert_eq!(s.filtered_indices().len(), 5);
        // Select ring index 3 (the D row).
        s.selected = 3;
        s.auto_scroll = false;
        // Apply a text filter that matches only "B". `toggle_*` helpers
        // capture prev automatically; to simulate that for the direct
        // text assignment we call reconcile_selection with an explicit
        // prior ring index.
        let prev = s.selected_ring_index();
        s.filters.text = "B".into();
        s.reconcile_selection(prev);
        assert_eq!(s.filtered_indices(), vec![1]);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn reconcile_selection_handles_empty_filtered_list() {
        let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
        s.selected = 0;
        s.auto_scroll = true;
        s.filters.text = "nomatch".into();
        s.reconcile_selection(None);
        assert_eq!(s.filtered_indices().len(), 0);
        assert_eq!(s.selected, 0);
        assert!(s.auto_scroll, "auto_scroll unchanged when empty");
    }

    #[test]
    fn entering_text_input_mode_routes_keys_into_buffer() {
        let mut s = BulletinsState::with_capacity(100);
        s.enter_text_input_mode();
        assert!(s.text_input.is_some());
        s.push_text_input('f', None);
        s.push_text_input('o', None);
        s.push_text_input('o', None);
        assert_eq!(s.text_input.as_deref(), Some("foo"));
        s.pop_text_input(None);
        assert_eq!(s.text_input.as_deref(), Some("fo"));
    }

    #[test]
    fn enter_commits_text_input_and_updates_filter() {
        let mut s = BulletinsState::with_capacity(100);
        s.enter_text_input_mode();
        s.push_text_input('I', None);
        s.push_text_input('O', None);
        s.commit_text_input(None);
        assert!(s.text_input.is_none());
        assert_eq!(s.filters.text, "IO");
    }

    #[test]
    fn escape_cancels_text_input_without_committing() {
        let mut s = BulletinsState::with_capacity(100);
        s.filters.text = "keep".into();
        s.enter_text_input_mode();
        s.push_text_input('x', None);
        s.cancel_text_input(None);
        assert!(s.text_input.is_none());
        assert_eq!(s.filters.text, "keep");
    }

    #[test]
    fn auto_scroll_on_keeps_selection_at_bottom() {
        let mut s = BulletinsState::with_capacity(100);
        s.auto_scroll = true;
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        assert_eq!(s.selected, 1);
        apply_payload_test(&mut s, vec![b(3, "INFO"), b(4, "INFO")]);
        assert_eq!(s.selected, 3);
    }

    #[test]
    fn auto_scroll_off_counts_new_since_pause() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        s.auto_scroll = false;
        s.selected = 0;
        apply_payload_test(&mut s, vec![b(3, "INFO"), b(4, "INFO")]);
        assert_eq!(s.new_since_pause, 2);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn auto_scroll_off_ignores_non_matching_for_badge() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
        s.auto_scroll = false;
        s.toggle_info();
        s.toggle_warning();
        // Only ERROR is visible now.
        apply_payload_test(&mut s, vec![b_full(2, "INFO", "PROCESSOR", "B", "m")]);
        assert_eq!(s.new_since_pause, 0);
    }

    #[test]
    fn g_and_end_resume_auto_scroll_and_clear_badge() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        s.auto_scroll = false;
        s.new_since_pause = 7;
        s.selected = 0;
        s.goto_newest();
        assert!(s.auto_scroll);
        assert_eq!(s.new_since_pause, 0);
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn p_toggles_auto_scroll_without_goto() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        s.selected = 0;
        s.auto_scroll = true;
        s.toggle_pause();
        assert!(!s.auto_scroll);
        assert_eq!(s.selected, 0);
        s.toggle_pause();
        assert!(s.auto_scroll);
    }

    #[test]
    fn upward_navigation_pauses_auto_scroll() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]);
        assert_eq!(s.selected, 2);
        assert!(s.auto_scroll);
        s.move_selection_up();
        assert_eq!(s.selected, 1);
        assert!(!s.auto_scroll);
    }

    #[test]
    fn move_selection_down_on_empty_filtered_list_is_noop() {
        // Paused, `+N new` showing, empty filtered view — pressing down
        // must not silently resume auto-scroll or clear the badge.
        let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
        s.auto_scroll = false;
        s.new_since_pause = 5;
        s.filters.text = "nomatch".into();
        s.reconcile_selection(None);
        assert!(s.filtered_indices().is_empty());
        s.move_selection_down();
        assert!(!s.auto_scroll, "auto_scroll must stay paused");
        assert_eq!(s.new_since_pause, 5, "badge count must not be cleared");
    }

    #[test]
    fn grouped_view_returns_empty_when_ring_empty() {
        let s = BulletinsState::with_capacity(10);
        assert!(s.grouped_view().is_empty());
    }

    #[test]
    fn grouped_view_no_consecutive_duplicates() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "INFO", "PROCESSOR", "A", "m"),
                b_full(2, "INFO", "PROCESSOR", "B", "m"),
                b_full(3, "INFO", "PROCESSOR", "C", "m"),
            ],
        );
        s.group_mode = GroupMode::Source;
        // Each bulletin has a distinct source_id via `src-{id}` — no grouping.
        let out = s.grouped_view();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|g| g.count == 1));
        assert_eq!(out[0].first_ring_idx, 0);
        assert_eq!(out[0].latest_ring_idx, 0);
        assert_eq!(out[2].first_ring_idx, 2);
    }

    #[test]
    fn grouped_view_collapses_same_source_run() {
        // Build a seed with three bulletins sharing source_id "src-same".
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(
            &mut s,
            vec![
                BulletinSnapshot {
                    id: 1,
                    level: "ERROR".into(),
                    message: "first".into(),
                    source_id: "src-same".into(),
                    source_name: "P".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "root".into(),
                    timestamp_iso: "2026-04-11T10:14:22Z".into(),
                    timestamp_human: String::new(),
                },
                BulletinSnapshot {
                    id: 2,
                    level: "ERROR".into(),
                    message: "second".into(),
                    source_id: "src-same".into(),
                    source_name: "P".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "root".into(),
                    timestamp_iso: "2026-04-11T10:14:23Z".into(),
                    timestamp_human: String::new(),
                },
                BulletinSnapshot {
                    id: 3,
                    level: "ERROR".into(),
                    message: "third".into(),
                    source_id: "src-same".into(),
                    source_name: "P".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "root".into(),
                    timestamp_iso: "2026-04-11T10:14:24Z".into(),
                    timestamp_human: String::new(),
                },
            ],
        );
        s.group_mode = GroupMode::Source;
        let out = s.grouped_view();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 3);
        assert_eq!(out[0].first_ring_idx, 0);
        assert_eq!(out[0].latest_ring_idx, 2);
    }

    #[test]
    fn grouped_view_interleaved_folds_non_consecutive() {
        // A, B, A pattern — non-consecutive dedup folds into 2 groups:
        // src-a (ring_idx 0 and 2) and src-b (ring_idx 1).
        let mut s = BulletinsState::with_capacity(100);
        let mk = |id: i64, src: &str| BulletinSnapshot {
            id,
            level: "ERROR".into(),
            message: "m".into(),
            source_id: src.into(),
            source_name: src.into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        };
        apply_payload_test(&mut s, vec![mk(1, "src-a"), mk(2, "src-b"), mk(3, "src-a")]);
        s.group_mode = GroupMode::Source;
        let out = s.grouped_view();
        // Non-consecutive dedup: src-a (ring_idx 0+2) folds into one group.
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].count, 2);
        assert_eq!(out[0].first_ring_idx, 0);
        assert_eq!(out[0].latest_ring_idx, 2);
        assert_eq!(out[1].count, 1);
        assert_eq!(out[1].first_ring_idx, 1);
    }

    #[test]
    fn grouped_view_respects_filters() {
        // ERROR + INFO + ERROR all with same source_id. Toggling INFO off
        // should collapse the two ERRORs into a single group (they are
        // consecutive in the filtered list).
        let mut s = BulletinsState::with_capacity(100);
        let mk = |id: i64, level: &str| BulletinSnapshot {
            id,
            level: level.into(),
            message: format!("msg-{id}"),
            source_id: "src-same".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        };
        apply_payload_test(&mut s, vec![mk(1, "ERROR"), mk(2, "INFO"), mk(3, "ERROR")]);
        s.group_mode = GroupMode::Source;
        // All three share source_id; grouping folds them to one.
        assert_eq!(s.grouped_view().len(), 1);
        // Toggle INFO off — still one group because filtered list is
        // ring[0] and ring[2], both same source_id.
        s.toggle_info();
        let out = s.grouped_view();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 2);
        assert_eq!(out[0].first_ring_idx, 0);
        assert_eq!(out[0].latest_ring_idx, 2);
    }

    fn seed_grouping_fixture() -> BulletinsState {
        // Ring layout by ring_idx and source_id:
        //   0: src-a   (count group #0)
        //   1: src-a   (count group #0)
        //   2: src-b   (count group #1)
        //   3: src-c   (count group #2)
        //   4: src-c   (count group #2)
        //   5: src-c   (count group #2)
        let mut s = BulletinsState::with_capacity(100);
        let mk = |id: i64, src: &str| BulletinSnapshot {
            id,
            level: "ERROR".into(),
            message: format!("msg-{id}"),
            source_id: src.into(),
            source_name: src.into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: format!("2026-04-11T10:14:{:02}Z", id),
            timestamp_human: String::new(),
        };
        apply_payload_test(
            &mut s,
            vec![
                mk(1, "src-a"),
                mk(2, "src-a"),
                mk(3, "src-b"),
                mk(4, "src-c"),
                mk(5, "src-c"),
                mk(6, "src-c"),
            ],
        );
        s.auto_scroll = false;
        s
    }

    #[test]
    fn cycle_group_mode_on_preserves_selection_to_enclosing_group() {
        let mut s = seed_grouping_fixture();
        // Off mode: 6 visible rows. Select ring_idx 4 (second "src-c", msg-5).
        s.group_mode = GroupMode::Off;
        assert_eq!(s.grouped_view().len(), 6);
        s.selected = 4; // Points at ring_idx 4 in flat mode.
        assert_eq!(s.selected_ring_index(), Some(4));

        s.cycle_group_mode();

        // SourceAndMessage mode: fixture messages are msg-1..msg-6 (all distinct),
        // so each (source_id, message) pair is its own group → 6 groups.
        // Selection reconciles to the group whose latest_ring_idx == 4 (singleton),
        // which is at position 4.
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
        assert_eq!(s.grouped_view().len(), 6);
        assert_eq!(s.selected, 4);
        assert_eq!(s.selected_ring_index(), Some(4));
    }

    #[test]
    fn cycle_group_mode_off_preserves_selection_to_latest_bulletin() {
        let mut s = seed_grouping_fixture();
        s.group_mode = GroupMode::Source;
        // Grouped mode: 3 visible groups. Select group #2 (src-c run).
        s.selected = 2;
        assert_eq!(s.selected_ring_index(), Some(5));

        s.cycle_group_mode();

        // Off mode: 6 visible rows. Selection should land on the latest
        // bulletin of the previously-selected group — ring_idx 5, flat
        // position 5.
        assert_eq!(s.group_mode, GroupMode::Off);
        assert_eq!(s.selected, 5);
        assert_eq!(s.selected_ring_index(), Some(5));
    }

    #[test]
    fn cycle_group_mode_from_first_row_stays_on_first_row() {
        let mut s = seed_grouping_fixture();
        s.group_mode = GroupMode::Off;
        s.selected = 0; // src-a flat ring_idx 0
        s.cycle_group_mode();
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
        // Fixture messages are all distinct, so ring_idx 0 is its own group
        // at position 0; selection stays at 0.
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn cycle_group_mode_with_no_visible_rows_is_safe() {
        let mut s = BulletinsState::with_capacity(100);
        // Empty ring — nothing to preserve.
        s.cycle_group_mode();
        assert_eq!(s.group_mode, GroupMode::Source);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn strip_component_prefix_strips_standard_nifi_format() {
        let msg = "UpdateRecord[id=85cecfc6-019d-1000-ffff-ffffe8c7c778] field 'customer.id' missing in input record";
        assert_eq!(
            strip_component_prefix(msg),
            "field 'customer.id' missing in input record"
        );
    }

    #[test]
    fn strip_component_prefix_returns_original_when_prefix_absent() {
        let msg = "plain message with no brackets";
        assert_eq!(
            strip_component_prefix(msg),
            "plain message with no brackets"
        );
    }

    #[test]
    fn strip_component_prefix_handles_name_with_spaces() {
        let msg = "Route On Attribute[id=aaaaaaaa-1111-2222-3333-444444444444] routed to failure";
        assert_eq!(strip_component_prefix(msg), "routed to failure");
    }

    #[test]
    fn strip_component_prefix_returns_original_when_id_bracket_missing() {
        let msg = "Garbled[no-id-here] still garbled";
        assert_eq!(
            strip_component_prefix(msg),
            "Garbled[no-id-here] still garbled"
        );
    }

    #[test]
    fn strip_component_prefix_returns_original_when_no_trailing_space() {
        // Malformed: no space after the closing bracket.
        let msg = "Proc[id=aaaaaaaa-1111-2222-3333-444444444444]no-space";
        assert_eq!(
            strip_component_prefix(msg),
            "Proc[id=aaaaaaaa-1111-2222-3333-444444444444]no-space"
        );
    }

    #[test]
    fn strip_component_prefix_is_idempotent_on_already_clean_message() {
        let msg = "already clean";
        assert_eq!(strip_component_prefix(msg), "already clean");
    }

    #[test]
    fn normalize_dynamic_brackets_replaces_single_bracket_region() {
        assert_eq!(
            normalize_dynamic_brackets("Failed to process FlowFile[filename=abc.txt]"),
            "Failed to process FlowFile[\u{2026}]"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_replaces_multiple_bracket_regions() {
        let input = "a FlowFile[id=x] and StandardFlowFileRecord[uuid=y]";
        assert_eq!(
            normalize_dynamic_brackets(input),
            "a FlowFile[\u{2026}] and StandardFlowFileRecord[\u{2026}]"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_handles_nested_braces_inside_bracket() {
        let input = "StandardFlowFileRecord[uuid=abc, attributes={k=v, k2=v2}]";
        assert_eq!(
            normalize_dynamic_brackets(input),
            "StandardFlowFileRecord[\u{2026}]"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_returns_unchanged_when_no_brackets() {
        assert_eq!(
            normalize_dynamic_brackets("will route to failure"),
            "will route to failure"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_returns_unchanged_on_unclosed_bracket() {
        assert_eq!(
            normalize_dynamic_brackets("something [unclosed but no close"),
            "something [unclosed but no close"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_handles_empty_string() {
        assert_eq!(normalize_dynamic_brackets(""), "");
    }

    #[test]
    fn normalize_dynamic_brackets_handles_bracket_at_end() {
        assert_eq!(
            normalize_dynamic_brackets("prefix [suffix]"),
            "prefix [\u{2026}]"
        );
    }

    #[test]
    fn normalize_dynamic_brackets_preserves_text_between_brackets() {
        let input = "before FlowFile[a=1]; middle StandardFlowFileRecord[b=2]; after";
        assert_eq!(
            normalize_dynamic_brackets(input),
            "before FlowFile[\u{2026}]; middle StandardFlowFileRecord[\u{2026}]; after"
        );
    }

    #[test]
    fn grouped_view_collapses_across_flowfile_attrs() {
        // Reproduces the user-reported bug: two bulletins from the same
        // source with the same message shape but different embedded
        // flowfile attributes should collapse into a single grouped row.
        use crate::client::BulletinSnapshot;
        let mut s = BulletinsState::with_capacity(100);
        s.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "UpdateRecord[id=pid] Failed to process FlowFile[filename=a.txt]; \
                      will route to failure"
                .into(),
            source_id: "pid".into(),
            source_name: "UpdateRecord".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-14T10:00:00Z".into(),
            timestamp_human: String::new(),
        });
        s.ring.push_back(BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: "UpdateRecord[id=pid] Failed to process FlowFile[filename=b.txt]; \
                      will route to failure"
                .into(),
            source_id: "pid".into(),
            source_name: "UpdateRecord".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-14T10:00:01Z".into(),
            timestamp_human: String::new(),
        });
        let rows = s.grouped_view();
        assert_eq!(
            rows.len(),
            1,
            "two same-shape bulletins must collapse into one row"
        );
        assert_eq!(rows[0].count, 2, "count must reflect both occurrences");
    }

    #[test]
    fn source_and_message_mode_dedups_identical_stems_across_ring() {
        let mut s = BulletinsState::with_capacity(100);
        // Three bulletins from src-1 with an identical stripped stem
        // interleaved with one bulletin from src-2.
        let prefix_a = "ProcA[id=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa] ";
        let prefix_b = "ProcB[id=bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb] ";
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: format!("{prefix_a}same stem"),
                source_id: "src-1".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: format!("{prefix_b}other stem"),
                source_id: "src-2".into(),
                source_name: "ProcB".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:23Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "ERROR".into(),
                message: format!("{prefix_a}same stem"),
                source_id: "src-1".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:24Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 4,
                level: "ERROR".into(),
                message: format!("{prefix_a}same stem"),
                source_id: "src-1".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:25Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
        let rows = s.grouped_view();
        // Two unique groups: src-1/"same stem" ×3, src-2/"other stem" ×1.
        assert_eq!(rows.len(), 2);
        // Stable ordering: groups appear in order of first-seen ring index.
        assert_eq!(rows[0].count, 3, "src-1 group should fold 3 bulletins");
        assert_eq!(rows[0].first_ring_idx, 0);
        assert_eq!(rows[0].latest_ring_idx, 3);
        assert_eq!(rows[1].count, 1);
        assert_eq!(rows[1].first_ring_idx, 1);
        assert_eq!(rows[1].latest_ring_idx, 1);
    }

    #[test]
    fn source_mode_folds_all_messages_from_one_source() {
        let mut s = BulletinsState::with_capacity(100);
        s.group_mode = GroupMode::Source;
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "ProcA[id=a] msg one".into(),
                source_id: "src-1".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "WARN".into(),
                message: "ProcA[id=a] msg two".into(),
                source_id: "src-1".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g1".into(),
                timestamp_iso: "2026-04-11T10:14:23Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        let rows = s.grouped_view();
        assert_eq!(rows.len(), 1, "Source mode collapses different stems");
        assert_eq!(rows[0].count, 2);
    }

    #[test]
    fn off_mode_emits_one_row_per_bulletin() {
        let mut s = BulletinsState::with_capacity(100);
        s.group_mode = GroupMode::Off;
        let rows = vec![
            b(1, "INFO"),
            b(2, "INFO"), // note: `b()` test helper gives different source_ids
        ];
        apply_payload_test(&mut s, rows);
        let rows = s.grouped_view();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|g| g.count == 1));
    }

    #[test]
    fn dedup_is_non_consecutive() {
        // Regression guard vs the old consecutive-only grouping.
        let mut s = BulletinsState::with_capacity(100);
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "P[id=a] boom".into(),
                source_id: "src-1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "Q[id=b] boom".into(),
                source_id: "src-2".into(),
                source_name: "Q".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:14:23Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "ERROR".into(),
                message: "P[id=a] boom".into(),
                source_id: "src-1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:14:24Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        let rows = s.grouped_view();
        // Old code would have produced 3 groups (P, Q, P). New dedup
        // collapses P across the interruption → 2 groups.
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].count, 2, "P rows fold across the Q interruption");
        assert_eq!(rows[1].count, 1);
    }

    #[test]
    fn group_mode_default_is_source_and_message() {
        let s = BulletinsState::with_capacity(100);
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    }

    #[test]
    fn cycle_group_mode_walks_source_and_message_then_source_then_off() {
        let mut s = BulletinsState::with_capacity(100);
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
        s.cycle_group_mode();
        assert_eq!(s.group_mode, GroupMode::Source);
        s.cycle_group_mode();
        assert_eq!(s.group_mode, GroupMode::Off);
        s.cycle_group_mode();
        assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    }

    #[test]
    fn mute_selected_source_hides_matching_rows() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
                b_full(2, "ERROR", "PROCESSOR", "B", "ProcB[id=b] crash"),
                b_full(3, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
            ],
        );
        // Use Off mode to see all rows individually.
        s.group_mode = GroupMode::Off;
        // Select the first row (src-1) and mute it.
        s.selected = 0;
        s.mute_selected_source();
        let rows = s.grouped_view();
        // src-1 is filtered out, leaving src-2 and src-3 (2 rows in Off mode).
        assert_eq!(rows.len(), 2);
        assert_eq!(s.ring[rows[0].latest_ring_idx].source_id, "src-2");
        assert_eq!(s.ring[rows[1].latest_ring_idx].source_id, "src-3");
        // Selection must have snapped forward to the surviving row.
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn mute_selected_source_is_a_toggle_on_repress() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
                b_full(2, "ERROR", "PROCESSOR", "B", "ProcB[id=b] crash"),
            ],
        );
        s.selected = 0;
        s.mute_selected_source();
        assert_eq!(s.grouped_view().len(), 1);
        // Navigate back onto src-1 is impossible while muted. Unmute by
        // API — the handler-side toggle path is covered in Task 6.
        s.mutes.remove("src-1");
        assert_eq!(s.grouped_view().len(), 2);
    }

    #[test]
    fn mute_noop_when_no_selection() {
        let mut s = BulletinsState::with_capacity(100);
        // Empty ring → no selection.
        s.mute_selected_source();
        assert!(s.mutes.is_empty());
    }

    #[test]
    fn severity_counts_returns_raw_ring_totals_ignoring_other_filters() {
        let mut s = seed(
            100,
            vec![
                b_full(1, "ERROR", "PROCESSOR", "A", "a"),
                b_full(2, "ERROR", "PROCESSOR", "A", "a"),
                b_full(3, "WARN", "PROCESSOR", "B", "b"),
                b_full(4, "INFO", "PROCESSOR", "C", "c"),
            ],
        );
        // Apply a text filter — should NOT affect severity counts.
        s.filters.text = "zzz".into();
        let counts = s.severity_counts();
        assert_eq!(counts.error, 2);
        assert_eq!(counts.warning, 1);
        assert_eq!(counts.info, 1);
    }

    #[test]
    fn group_details_returns_first_and_last_seen_for_dedup_group() {
        let mut s = BulletinsState::with_capacity(100);
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "P[id=a] same stem".into(),
                source_id: "src-1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:00:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "P[id=a] same stem".into(),
                source_id: "src-1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:05:00Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        s.selected = 0;
        let d = s.group_details().expect("group exists");
        assert_eq!(d.count, 2);
        assert_eq!(d.first_seen_iso, "2026-04-11T10:00:00Z");
        assert_eq!(d.last_seen_iso, "2026-04-11T10:05:00Z");
        assert_eq!(d.source_id, "src-1");
        assert_eq!(d.group_id, "g");
        assert_eq!(d.stripped_message, "same stem");
        assert_eq!(d.raw_message, "P[id=a] same stem");
    }

    #[test]
    fn recent_for_source_id_returns_newest_first_up_to_limit() {
        let mut s = BulletinsState::with_capacity(100);
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "INFO".into(),
                message: "a".into(),
                source_id: "p1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:00:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "b".into(),
                source_id: "p2".into(),
                source_name: "Q".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:01:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "WARN".into(),
                message: "c".into(),
                source_id: "p1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:02:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 4,
                level: "WARN".into(),
                message: "d".into(),
                source_id: "p1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:03:00Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        let hits = recent_for_source_id(&s.ring, "p1", 2);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, 4, "newest first");
        assert_eq!(hits[1].id, 3);
    }

    #[test]
    fn recent_for_source_id_limit_zero_returns_empty() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO")]);
        let hits = recent_for_source_id(&s.ring, "src-1", 0);
        assert!(hits.is_empty());
    }

    #[test]
    fn recent_for_source_id_no_match_returns_empty() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
        let hits = recent_for_source_id(&s.ring, "nonexistent", 10);
        assert!(hits.is_empty());
    }

    #[test]
    fn recent_for_group_id_filters_by_group_id() {
        let mut s = BulletinsState::with_capacity(100);
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "INFO".into(),
                message: "a".into(),
                source_id: "p1".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "noisy".into(),
                timestamp_iso: "2026-04-11T10:00:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "b".into(),
                source_id: "p2".into(),
                source_name: "Q".into(),
                source_type: "PROCESSOR".into(),
                group_id: "healthy".into(),
                timestamp_iso: "2026-04-11T10:01:00Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "WARN".into(),
                message: "c".into(),
                source_id: "p3".into(),
                source_name: "R".into(),
                source_type: "PROCESSOR".into(),
                group_id: "noisy".into(),
                timestamp_iso: "2026-04-11T10:02:00Z".into(),
                timestamp_human: String::new(),
            },
        ];
        apply_payload_test(&mut s, rows);
        let hits = recent_for_group_id(&s.ring, "noisy", 10);
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].id, 3, "newest first");
        assert_eq!(hits[1].id, 1);
    }
}
