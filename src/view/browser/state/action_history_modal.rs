//! State for the Browser tab's action-history modal.
//!
//! Mirrors `version_control_modal` and `parameter_context_modal` in
//! shape: per-open struct on `BrowserState.action_history_modal`,
//! populated by the reducer from worker-emitted `ActionHistoryPage`
//! payloads. Uses the shared `widget::scroll::VerticalScrollState`
//! and `widget::search::SearchState` primitives.

use std::sync::Arc;

use nifi_rust_client::dynamic::types::ActionEntity;
use tokio::sync::Notify;

use crate::widget::scroll::VerticalScrollState;
use crate::widget::search::SearchState;

#[derive(Debug, Clone)]
pub struct ActionHistoryModalState {
    /// Component the modal was opened on. Stays fixed for the modal
    /// session; reducer guards stale `ActionHistoryPage` emits with
    /// this value.
    pub source_id: String,
    /// Pre-resolved component name shown in the modal title.
    pub component_label: String,
    /// Already-fetched actions, in fetch order (newest first per NiFi
    /// default sort). Deduplicated by `ActionEntity` `id` field.
    pub actions: Vec<ActionEntity>,
    /// Total reported by the paginator. `None` until the first page
    /// arrives.
    pub total: Option<u32>,
    /// True while a page fetch is in flight. The render module shows
    /// a `loading…` chip when set.
    pub loading: bool,
    /// Optional failure message; presence stops auto-loading.
    pub error: Option<String>,
    /// Index into `actions` of the row whose details are inline-
    /// expanded. `None` when no row is expanded.
    pub expanded_index: Option<usize>,
    /// Vertical scroll position. The renderer drives the
    /// `apply_dimensions` call on each frame to clamp.
    pub scroll: VerticalScrollState,
    /// Optional search overlay; same lifecycle as the bulletins
    /// detail modal (`None` = inactive, `Some` with `input_active` =
    /// typing, `Some` with `committed` = n/N cycles matches).
    pub search: Option<SearchState>,
    /// Worker wakes on this to fetch the next page. Reducer fires
    /// `notify_one()` when scroll position is near the loaded tail
    /// AND `actions.len() < total`.
    pub fetch_signal: Arc<Notify>,
    /// Currently selected row (cursor index into `actions`). The renderer
    /// highlights this row; ToggleExpand/Copy use it as the target.
    pub selected: usize,
}

impl ActionHistoryModalState {
    pub fn pending(source_id: String, component_label: String) -> Self {
        Self {
            source_id,
            component_label,
            actions: Vec::new(),
            total: None,
            loading: true,
            error: None,
            expanded_index: None,
            scroll: VerticalScrollState::default(),
            search: None,
            fetch_signal: Arc::new(Notify::new()),
            selected: 0,
        }
    }

    /// Append a fetched page, dedup-by-id, and clear `loading`.
    /// Stale pages whose `source_id` doesn't match are ignored
    /// (caller must check before invoking, but defensive equality
    /// here avoids drift).
    pub fn apply_page(&mut self, source_id: &str, actions: Vec<ActionEntity>, total: Option<u32>) {
        if source_id != self.source_id {
            return;
        }
        let mut seen: std::collections::HashSet<i32> =
            self.actions.iter().filter_map(|a| a.id).collect();
        for a in actions {
            if let Some(id) = a.id
                && !seen.insert(id)
            {
                continue;
            }
            self.actions.push(a);
        }
        if total.is_some() {
            self.total = total;
        }
        self.loading = false;
    }

    /// Reset to a fresh-load state for `r` refresh. Caller is
    /// responsible for re-spawning the worker.
    pub fn reset_for_refresh(&mut self) {
        self.actions.clear();
        self.total = None;
        self.loading = true;
        self.error = None;
        self.expanded_index = None;
        self.scroll = VerticalScrollState::default();
        self.selected = 0;
        // Keep `search` and `source_id` / `component_label` intact.
    }

    /// Move selection up by 1, clamping to 0.
    pub fn move_selection_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
    }

    /// Move selection down by 1, clamping to `len - 1`.
    pub fn move_selection_down(&mut self) {
        if self.selected + 1 < self.actions.len() {
            self.selected += 1;
        }
    }

    /// Selected row index, or 0 if no rows.
    pub fn selected_row(&self) -> usize {
        self.selected
    }

    /// Toggle inline expansion of the row currently selected by
    /// `scroll.selected_index()`. Pass the current selected row
    /// from the reducer.
    pub fn toggle_expanded(&mut self, selected: usize) {
        self.expanded_index = match self.expanded_index {
            Some(i) if i == selected => None,
            _ => Some(selected),
        };
    }

    /// Whether scrolling within `n` rows of the loaded tail should
    /// signal the worker to fetch the next page.
    pub fn should_signal_next_page(&self, viewport_bottom: usize, threshold: usize) -> bool {
        let Some(total) = self.total else {
            return false; // first page hasn't landed yet
        };
        if (self.actions.len() as u32) >= total {
            return false;
        }
        if self.error.is_some() || self.loading {
            return false;
        }
        viewport_bottom + threshold >= self.actions.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nifi_rust_client::dynamic::types::ActionEntity;

    fn entity(id: i32) -> ActionEntity {
        let mut e = ActionEntity::default();
        e.id = Some(id);
        e
    }

    #[test]
    fn pending_initial_state_is_loading() {
        let s = ActionHistoryModalState::pending("proc-1".into(), "FetchKafka".into());
        assert!(s.actions.is_empty());
        assert_eq!(s.total, None);
        assert!(s.loading);
        assert!(s.error.is_none());
        assert!(s.expanded_index.is_none());
    }

    #[test]
    fn apply_page_appends_and_clears_loading() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("proc-1", vec![entity(1), entity(2)], Some(5));
        assert_eq!(s.actions.len(), 2);
        assert_eq!(s.total, Some(5));
        assert!(!s.loading);
    }

    #[test]
    fn apply_page_dedups_by_id() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("proc-1", vec![entity(1), entity(2)], Some(3));
        s.apply_page("proc-1", vec![entity(2), entity(3)], Some(3));
        assert_eq!(
            s.actions.iter().map(|a| a.id.unwrap()).collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
    }

    #[test]
    fn apply_page_drops_stale_source_id() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("other", vec![entity(99)], Some(1));
        assert!(s.actions.is_empty());
        assert!(s.loading, "stale page must not clear loading");
    }

    #[test]
    fn reset_for_refresh_restores_loading() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("proc-1", vec![entity(1)], Some(1));
        s.reset_for_refresh();
        assert!(s.actions.is_empty());
        assert_eq!(s.total, None);
        assert!(s.loading);
    }

    #[test]
    fn toggle_expanded_collapses_when_same_row() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.toggle_expanded(2);
        assert_eq!(s.expanded_index, Some(2));
        s.toggle_expanded(2);
        assert_eq!(s.expanded_index, None);
        s.toggle_expanded(2);
        s.toggle_expanded(5);
        assert_eq!(s.expanded_index, Some(5));
    }

    #[test]
    fn should_signal_next_page_within_threshold() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("proc-1", (0..10).map(entity).collect(), Some(50));
        // Viewport bottom at row 5, threshold 10 → 5 + 10 >= 10 → true.
        assert!(s.should_signal_next_page(5, 10));
        // Bottom at 0, threshold 5 → 0 + 5 < 10 → false.
        assert!(!s.should_signal_next_page(0, 5));
    }

    #[test]
    fn should_signal_next_page_false_when_exhausted() {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "X".into());
        s.apply_page("proc-1", (0..5).map(entity).collect(), Some(5));
        assert!(!s.should_signal_next_page(4, 100));
    }

    #[test]
    fn move_selection_clamps_to_bounds() {
        let mut s = ActionHistoryModalState::pending("p".into(), "X".into());
        // No actions — both moves are no-ops.
        s.move_selection_down();
        assert_eq!(s.selected, 0);
        s.move_selection_up();
        assert_eq!(s.selected, 0);
        // Add 3 actions, navigate.
        s.apply_page("p", (0..3).map(entity).collect(), Some(3));
        s.move_selection_down();
        s.move_selection_down();
        assert_eq!(s.selected, 2);
        s.move_selection_down(); // clamps at len-1
        assert_eq!(s.selected, 2);
        s.move_selection_up();
        assert_eq!(s.selected, 1);
    }
}
