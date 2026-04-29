//! Per-RPG selection state for the Browser detail pane. Holds the
//! detail-fetch result (or its error) plus the currently-selected
//! arena id so the renderer can short-circuit a `state.details` lookup.
//! The sparkline lives on `BrowserState::sparkline` (shared with
//! processor / PG / connection); this module is identity-only.

use crate::client::browser::RemoteProcessGroupDetail;

#[derive(Debug, Clone, Default)]
pub struct RemoteProcessGroupSelection {
    /// Currently-selected RPG arena id; `None` when no RPG is selected.
    pub selected_id: Option<String>,
    /// Latest successful detail fetch for `selected_id`. Cleared when
    /// the selection changes.
    pub detail: Option<RemoteProcessGroupDetail>,
    /// Last detail-fetch error message for `selected_id`, surfaced as
    /// the muted chip in the Identity-header subtitle. Cleared when
    /// `detail` lands.
    pub last_error: Option<String>,
}

impl RemoteProcessGroupSelection {
    /// Switch to a new RPG selection. When the id changes (or there was
    /// no prior selection), the cached detail and error are dropped so
    /// the renderer falls back to a `loading…` chip until the
    /// detail-fetch worker emits.
    pub fn select(&mut self, id: String) {
        if self.selected_id.as_deref() != Some(&id) {
            self.selected_id = Some(id);
            self.detail = None;
            self.last_error = None;
        }
    }

    /// Tear down the per-RPG selection state. Called on selection
    /// changes that move away from a `NodeKind::RemoteProcessGroup`
    /// row so stale detail / error data does not leak into the next
    /// render frame.
    pub fn clear(&mut self) {
        self.selected_id = None;
        self.detail = None;
        self.last_error = None;
    }
}

#[cfg(test)]
mod tests {
    use super::RemoteProcessGroupSelection;

    #[test]
    fn select_then_clear_round_trips() {
        let mut s = RemoteProcessGroupSelection::default();
        s.select("rpg-1".into());
        assert_eq!(s.selected_id.as_deref(), Some("rpg-1"));
        s.clear();
        assert!(s.selected_id.is_none());
        assert!(s.detail.is_none());
        assert!(s.last_error.is_none());
    }

    #[test]
    fn select_same_id_preserves_detail_and_error() {
        let mut s = RemoteProcessGroupSelection::default();
        s.select("rpg-1".into());
        s.last_error = Some("boom".into());
        s.select("rpg-1".into());
        assert_eq!(s.last_error.as_deref(), Some("boom"));
    }

    #[test]
    fn select_different_id_drops_detail_and_error() {
        let mut s = RemoteProcessGroupSelection::default();
        s.select("rpg-1".into());
        s.last_error = Some("boom".into());
        s.select("rpg-2".into());
        assert_eq!(s.selected_id.as_deref(), Some("rpg-2"));
        assert!(s.last_error.is_none());
    }
}
