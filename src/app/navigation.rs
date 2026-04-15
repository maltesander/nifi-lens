//! Shared list-navigation trait with clamp and wrap defaults.

/// Common navigation math for any view that has a selectable list.
///
/// Implementors provide `list_len`, `selected`, and `set_selected`.
/// All movement methods have default implementations that handle
/// empty lists gracefully (no panics) and support both clamping
/// (default) and wrapping behaviour via `wraps`.
pub(crate) trait ListNavigation {
    /// Number of items in the list.
    fn list_len(&self) -> usize;

    /// Currently selected index, or `None` if the list is empty.
    fn selected(&self) -> Option<usize>;

    /// Set the selected index. Implementations may clamp or ignore
    /// out-of-range values.
    fn set_selected(&mut self, index: Option<usize>);

    /// Whether navigation wraps around at both ends.
    /// Default is `false` (clamp).
    fn wraps(&self) -> bool {
        false
    }

    /// Move selection one item up.
    fn move_up(&mut self) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let Some(current) = self.selected() else {
            self.set_selected(Some(0));
            return;
        };
        if current == 0 {
            if self.wraps() {
                self.set_selected(Some(len - 1));
            }
            // clamping: already at 0, do nothing
        } else {
            self.set_selected(Some(current - 1));
        }
    }

    /// Move selection one item down.
    fn move_down(&mut self) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let Some(current) = self.selected() else {
            self.set_selected(Some(0));
            return;
        };
        if current >= len - 1 {
            if self.wraps() {
                self.set_selected(Some(0));
            }
            // clamping: already at end, do nothing
        } else {
            self.set_selected(Some(current + 1));
        }
    }

    /// Move selection one page up. Always clamps (no wrapping).
    fn page_up(&mut self, page_size: usize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let Some(current) = self.selected() else {
            self.set_selected(Some(0));
            return;
        };
        self.set_selected(Some(current.saturating_sub(page_size)));
    }

    /// Move selection one page down. Always clamps (no wrapping).
    fn page_down(&mut self, page_size: usize) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        let Some(current) = self.selected() else {
            self.set_selected(Some(0));
            return;
        };
        let last = len - 1;
        self.set_selected(Some(current.saturating_add(page_size).min(last)));
    }

    /// Jump to the first item.
    fn goto_first(&mut self) {
        if self.list_len() == 0 {
            return;
        }
        self.set_selected(Some(0));
    }

    /// Jump to the last item.
    fn goto_last(&mut self) {
        let len = self.list_len();
        if len == 0 {
            return;
        }
        self.set_selected(Some(len - 1));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Minimal test harness implementing `ListNavigation`.
    struct TestList {
        items: Vec<&'static str>,
        selected: Option<usize>,
        wraps: bool,
    }

    impl TestList {
        fn new(count: usize, wraps: bool) -> Self {
            let items: Vec<&str> = (0..count).map(|_| "x").collect();
            let selected = if count > 0 { Some(0) } else { None };
            Self {
                items,
                selected,
                wraps,
            }
        }
    }

    impl ListNavigation for TestList {
        fn list_len(&self) -> usize {
            self.items.len()
        }

        fn selected(&self) -> Option<usize> {
            self.selected
        }

        fn set_selected(&mut self, index: Option<usize>) {
            self.selected = index;
        }

        fn wraps(&self) -> bool {
            self.wraps
        }
    }

    // ── Empty list safety ──────────────────────────────────────────

    #[test]
    fn empty_list_move_up_is_noop() {
        let mut list = TestList::new(0, false);
        list.move_up();
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn empty_list_move_down_is_noop() {
        let mut list = TestList::new(0, false);
        list.move_down();
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn empty_list_page_up_is_noop() {
        let mut list = TestList::new(0, false);
        list.page_up(5);
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn empty_list_page_down_is_noop() {
        let mut list = TestList::new(0, false);
        list.page_down(5);
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn empty_list_goto_first_is_noop() {
        let mut list = TestList::new(0, false);
        list.goto_first();
        assert_eq!(list.selected(), None);
    }

    #[test]
    fn empty_list_goto_last_is_noop() {
        let mut list = TestList::new(0, false);
        list.goto_last();
        assert_eq!(list.selected(), None);
    }

    // ── Clamping (default) ─────────────────────────────────────────

    #[test]
    fn clamp_move_down_at_end() {
        let mut list = TestList::new(3, false);
        list.set_selected(Some(2));
        list.move_down();
        assert_eq!(list.selected(), Some(2));
    }

    #[test]
    fn clamp_move_up_at_zero() {
        let mut list = TestList::new(3, false);
        list.set_selected(Some(0));
        list.move_up();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn clamp_move_down_normal() {
        let mut list = TestList::new(5, false);
        list.set_selected(Some(1));
        list.move_down();
        assert_eq!(list.selected(), Some(2));
    }

    #[test]
    fn clamp_move_up_normal() {
        let mut list = TestList::new(5, false);
        list.set_selected(Some(3));
        list.move_up();
        assert_eq!(list.selected(), Some(2));
    }

    // ── Wrapping ───────────────────────────────────────────────────

    #[test]
    fn wrap_move_down_at_end() {
        let mut list = TestList::new(3, true);
        list.set_selected(Some(2));
        list.move_down();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn wrap_move_up_at_zero() {
        let mut list = TestList::new(3, true);
        list.set_selected(Some(0));
        list.move_up();
        assert_eq!(list.selected(), Some(2));
    }

    // ── Page up / down (always clamp) ──────────────────────────────

    #[test]
    fn page_down_clamps_at_end() {
        let mut list = TestList::new(10, false);
        list.set_selected(Some(7));
        list.page_down(5);
        assert_eq!(list.selected(), Some(9));
    }

    #[test]
    fn page_up_clamps_at_zero() {
        let mut list = TestList::new(10, false);
        list.set_selected(Some(2));
        list.page_up(5);
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn page_down_normal() {
        let mut list = TestList::new(20, false);
        list.set_selected(Some(3));
        list.page_down(5);
        assert_eq!(list.selected(), Some(8));
    }

    #[test]
    fn page_up_normal() {
        let mut list = TestList::new(20, false);
        list.set_selected(Some(10));
        list.page_up(5);
        assert_eq!(list.selected(), Some(5));
    }

    #[test]
    fn page_up_down_ignore_wraps_flag() {
        let mut list = TestList::new(5, true);
        list.set_selected(Some(1));
        list.page_up(10);
        assert_eq!(list.selected(), Some(0), "page_up should clamp, not wrap");

        list.set_selected(Some(3));
        list.page_down(10);
        assert_eq!(list.selected(), Some(4), "page_down should clamp, not wrap");
    }

    // ── Jump home / end ────────────────────────────────────────────

    #[test]
    fn goto_first() {
        let mut list = TestList::new(10, false);
        list.set_selected(Some(7));
        list.goto_first();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn goto_last() {
        let mut list = TestList::new(10, false);
        list.set_selected(Some(2));
        list.goto_last();
        assert_eq!(list.selected(), Some(9));
    }

    // ── Single-item list ───────────────────────────────────────────

    #[test]
    fn single_item_clamp_stays() {
        let mut list = TestList::new(1, false);
        list.move_up();
        assert_eq!(list.selected(), Some(0));
        list.move_down();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn single_item_wrap_stays() {
        let mut list = TestList::new(1, true);
        list.move_up();
        assert_eq!(list.selected(), Some(0));
        list.move_down();
        assert_eq!(list.selected(), Some(0));
    }

    // ── None-selected initialisation ───────────────────────────────

    #[test]
    fn none_selected_move_down_selects_first() {
        let mut list = TestList::new(5, false);
        list.set_selected(None);
        list.move_down();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn none_selected_move_up_selects_first() {
        let mut list = TestList::new(5, false);
        list.set_selected(None);
        list.move_up();
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn none_selected_page_down_selects_first() {
        let mut list = TestList::new(5, false);
        list.set_selected(None);
        list.page_down(3);
        assert_eq!(list.selected(), Some(0));
    }

    #[test]
    fn none_selected_page_up_selects_first() {
        let mut list = TestList::new(5, false);
        list.set_selected(None);
        list.page_up(3);
        assert_eq!(list.selected(), Some(0));
    }
}
