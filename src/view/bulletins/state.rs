//! Pure state for the Bulletins tab.
//!
//! Everything here is synchronous and no-I/O. The tokio worker in
//! `super::worker` is the only place that touches the network.

use std::collections::VecDeque;
use std::time::SystemTime;

use crate::app::navigation::ListNavigation;
use crate::client::BulletinSnapshot;
use crate::event::BulletinsPayload;

#[derive(Debug)]
pub struct BulletinsState {
    pub ring: VecDeque<BulletinSnapshot>,
    pub ring_capacity: usize,
    pub last_id: Option<i64>,
    pub last_fetched_at: Option<SystemTime>,
    pub filters: FilterState,
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
    pub group_consecutive: bool,
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

/// A row in the grouped display. Produced by
/// [`BulletinsState::grouped_view`] when `group_consecutive` is true,
/// or (implicitly, as a vec of `count = 1` singletons) when flat.
///
/// Ring indices are stable for the lifetime of the ring buffer — the
/// render layer dereferences them via `state.ring[latest_ring_idx]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GroupedRow {
    /// Ring index of the first bulletin in the run (oldest in the
    /// group). Used by `toggle_grouping` to find which group contains
    /// a given pre-toggle ring index.
    pub first_ring_idx: usize,
    /// Ring index of the most-recent bulletin in the run. Render uses
    /// this to fetch the displayed timestamp, source, and message.
    pub latest_ring_idx: usize,
    /// Number of bulletins folded into this group. `1` means no
    /// grouping occurred; render skips the `[×N]` prefix.
    pub count: usize,
}

impl BulletinsState {
    pub fn with_capacity(ring_capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(ring_capacity),
            ring_capacity,
            last_id: None,
            last_fetched_at: None,
            filters: FilterState::default(),
            text_input: None,
            pre_input_text: None,
            selected: 0,
            auto_scroll: true,
            new_since_pause: 0,
            group_consecutive: false,
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

    /// Fold consecutive bulletins sharing the same `source_id` in the
    /// filtered list into `GroupedRow`s. In flat mode (`group_consecutive`
    /// is `false`) every filtered row is its own singleton with `count = 1`.
    /// In grouped mode, consecutive same-source runs are folded.
    pub fn grouped_view(&self) -> Vec<GroupedRow> {
        let filtered = self.filtered_indices();
        if !self.group_consecutive {
            // Flat mode: each filtered row is its own "group" with count=1.
            return filtered
                .iter()
                .map(|&ring_idx| GroupedRow {
                    first_ring_idx: ring_idx,
                    latest_ring_idx: ring_idx,
                    count: 1,
                })
                .collect();
        }
        // Grouped mode: fold consecutive same-source runs.
        let mut out: Vec<GroupedRow> = Vec::with_capacity(filtered.len());
        for &ring_idx in &filtered {
            let source_id = &self.ring[ring_idx].source_id;
            match out.last_mut() {
                Some(group) if self.ring[group.latest_ring_idx].source_id == *source_id => {
                    group.latest_ring_idx = ring_idx;
                    group.count += 1;
                }
                _ => {
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
        self.filtered_indices().get(self.selected).copied()
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
        let vis_len = self.filtered_indices().len();
        if vis_len > 0 && self.selected == vis_len - 1 {
            self.auto_scroll = true;
            self.new_since_pause = 0;
        }
    }
    pub fn jump_to_oldest(&mut self) {
        ListNavigation::jump_home(self);
        self.auto_scroll = false;
    }
    pub fn jump_to_newest(&mut self) {
        ListNavigation::jump_end(self);
        self.auto_scroll = true;
        self.new_since_pause = 0;
    }
    pub fn toggle_pause(&mut self) {
        self.auto_scroll = !self.auto_scroll;
        if self.auto_scroll {
            let max = self.filtered_indices().len().saturating_sub(1);
            self.selected = max;
            self.new_since_pause = 0;
        }
    }

    /// Called whenever filters change. Snap `selected` to the nearest
    /// still-visible filtered index based on its *previous* ring position.
    /// Callers capture the ring index *before* mutating filters and pass
    /// it in as `prev_ring_index`.
    pub fn reconcile_selection(&mut self, prev_ring_index: Option<usize>) {
        let visible = self.filtered_indices();
        if visible.is_empty() {
            self.selected = 0;
            return;
        }
        if let Some(prev) = prev_ring_index {
            if let Some(pos) = visible.iter().position(|&i| i == prev) {
                self.selected = pos;
                return;
            }
            if let Some(pos) = visible.iter().rposition(|&i| i < prev) {
                self.selected = pos;
                return;
            }
            if let Some(pos) = visible.iter().position(|&i| i > prev) {
                self.selected = pos;
                return;
            }
        }
        self.selected = if self.auto_scroll {
            visible.len() - 1
        } else {
            0
        };
    }
}

impl ListNavigation for BulletinsState {
    fn list_len(&self) -> usize {
        self.filtered_indices().len()
    }

    fn selected(&self) -> Option<usize> {
        if self.filtered_indices().is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
    }
}

/// Fold one poll result into the state. Pure; no I/O.
pub fn apply_payload(state: &mut BulletinsState, payload: BulletinsPayload) {
    let cursor = state.last_id.unwrap_or(i64::MIN);
    let mut max_seen = cursor;
    let before_len = state.ring.len();

    for b in payload.bulletins {
        if b.id <= cursor {
            continue;
        }
        if b.id > max_seen {
            max_seen = b.id;
        }
        state.ring.push_back(b);
    }

    // Count matching new rows for the +N badge BEFORE drop-oldest so we
    // don't double-count rows that fall off the front.
    let mut new_matching = 0u32;
    if !state.auto_scroll {
        for b in state.ring.iter().skip(before_len) {
            if state.row_matches(b) {
                new_matching = new_matching.saturating_add(1);
            }
        }
    }

    while state.ring.len() > state.ring_capacity {
        state.ring.pop_front();
    }
    if max_seen > cursor {
        state.last_id = Some(max_seen);
    }
    state.last_fetched_at = Some(payload.fetched_at);

    if state.auto_scroll {
        let max = state.filtered_indices().len().saturating_sub(1);
        state.selected = max;
        state.new_since_pause = 0;
    } else {
        state.new_since_pause = state.new_since_pause.saturating_add(new_matching);
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

    fn payload(bulletins: Vec<BulletinSnapshot>) -> BulletinsPayload {
        BulletinsPayload {
            bulletins,
            fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
        }
    }

    #[test]
    fn apply_payload_seeds_empty_ring_with_initial_batch() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(
            &mut s,
            payload(vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]),
        );
        assert_eq!(s.ring.len(), 3);
        assert_eq!(s.ring[0].id, 1);
        assert_eq!(s.ring[2].id, 3);
        assert_eq!(s.last_id, Some(3));
        assert!(s.last_fetched_at.is_some());
    }

    #[test]
    fn apply_payload_dedups_on_id() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
        apply_payload(&mut s, payload(vec![b(2, "INFO"), b(3, "INFO")]));
        assert_eq!(s.ring.len(), 3);
        assert_eq!(s.last_id, Some(3));
    }

    #[test]
    fn apply_payload_drops_oldest_at_capacity() {
        let mut s = BulletinsState::with_capacity(4);
        apply_payload(
            &mut s,
            payload(vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]),
        );
        apply_payload(
            &mut s,
            payload(vec![b(4, "INFO"), b(5, "INFO"), b(6, "INFO")]),
        );
        assert_eq!(s.ring.len(), 4);
        assert_eq!(s.ring.front().unwrap().id, 3);
        assert_eq!(s.ring.back().unwrap().id, 6);
    }

    #[test]
    fn apply_payload_advances_last_id_monotonically() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(10, "INFO")]));
        assert_eq!(s.last_id, Some(10));
        // Stale batch (server reordered or wrapped): cursor stays at 10.
        apply_payload(&mut s, payload(vec![b(5, "INFO")]));
        assert_eq!(s.last_id, Some(10));
        // New bulletins above the cursor: advances.
        apply_payload(&mut s, payload(vec![b(11, "INFO"), b(15, "INFO")]));
        assert_eq!(s.last_id, Some(15));
    }

    #[test]
    fn apply_payload_empty_batch_is_noop_except_for_fetched_at() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![]));
        assert!(s.ring.is_empty());
        assert_eq!(s.last_id, None);
        assert!(s.last_fetched_at.is_some());
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
        apply_payload(&mut s, payload(rows));
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
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
        assert_eq!(s.selected, 1);
        apply_payload(&mut s, payload(vec![b(3, "INFO"), b(4, "INFO")]));
        assert_eq!(s.selected, 3);
    }

    #[test]
    fn auto_scroll_off_counts_new_since_pause() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
        s.auto_scroll = false;
        s.selected = 0;
        apply_payload(&mut s, payload(vec![b(3, "INFO"), b(4, "INFO")]));
        assert_eq!(s.new_since_pause, 2);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn auto_scroll_off_ignores_non_matching_for_badge() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(
            &mut s,
            payload(vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]),
        );
        s.auto_scroll = false;
        s.toggle_info();
        s.toggle_warning();
        // Only ERROR is visible now.
        apply_payload(
            &mut s,
            payload(vec![b_full(2, "INFO", "PROCESSOR", "B", "m")]),
        );
        assert_eq!(s.new_since_pause, 0);
    }

    #[test]
    fn g_and_end_resume_auto_scroll_and_clear_badge() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
        s.auto_scroll = false;
        s.new_since_pause = 7;
        s.selected = 0;
        s.jump_to_newest();
        assert!(s.auto_scroll);
        assert_eq!(s.new_since_pause, 0);
        assert_eq!(s.selected, 1);
    }

    #[test]
    fn p_toggles_auto_scroll_without_jumping() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
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
        apply_payload(
            &mut s,
            payload(vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]),
        );
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
        s.group_consecutive = true;
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
        apply_payload(
            &mut s,
            payload(vec![
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
            ]),
        );
        s.group_consecutive = true;
        let out = s.grouped_view();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].count, 3);
        assert_eq!(out[0].first_ring_idx, 0);
        assert_eq!(out[0].latest_ring_idx, 2);
    }

    #[test]
    fn grouped_view_interleaved_keeps_runs_separate() {
        // A, B, A pattern — three groups, each count = 1.
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
        apply_payload(
            &mut s,
            payload(vec![mk(1, "src-a"), mk(2, "src-b"), mk(3, "src-a")]),
        );
        s.group_consecutive = true;
        let out = s.grouped_view();
        assert_eq!(out.len(), 3);
        assert!(out.iter().all(|g| g.count == 1));
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
        apply_payload(
            &mut s,
            payload(vec![mk(1, "ERROR"), mk(2, "INFO"), mk(3, "ERROR")]),
        );
        s.group_consecutive = true;
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
}
