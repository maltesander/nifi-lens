//! Reusable scroll-offset state for full-screen modals. Designed to
//! be composed into modal state structs — the modal owns its content
//! and renders the offset; the widget owns only the math.

/// Vertical-only scroll state used by the Bulletins detail modal.
/// The modal owns the content; this struct owns only the offset math.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct VerticalScrollState {
    pub offset: usize,
    /// Last known viewport height in wrapped-line rows. Set by the
    /// render pass each frame so scroll math knows the body height.
    pub last_viewport_rows: usize,
}

impl VerticalScrollState {
    /// Scroll by `delta` rows. Negative values go up; positive go
    /// down. Clamped to `[0, content_rows - last_viewport_rows]`.
    pub fn scroll_by(&mut self, delta: i32, content_rows: usize) {
        let max = content_rows.saturating_sub(self.last_viewport_rows);
        let new = if delta < 0 {
            self.offset.saturating_sub(delta.unsigned_abs() as usize)
        } else {
            self.offset.saturating_add(delta as usize)
        };
        self.offset = new.min(max);
    }

    /// Page up by one viewport height. Clamps to 0.
    pub fn page_up(&mut self) {
        self.offset = self.offset.saturating_sub(self.last_viewport_rows);
    }

    /// Page down by one viewport height. Clamps to the maximum
    /// scrollable offset for the given content size.
    pub fn page_down(&mut self, content_rows: usize) {
        let max = content_rows.saturating_sub(self.last_viewport_rows);
        self.offset = self.offset.saturating_add(self.last_viewport_rows).min(max);
    }

    /// Jump to the top (offset 0).
    pub fn jump_top(&mut self) {
        self.offset = 0;
    }

    /// Jump to the bottom — the last row is visible at the bottom
    /// edge of the viewport.
    pub fn jump_bottom(&mut self, content_rows: usize) {
        self.offset = content_rows.saturating_sub(self.last_viewport_rows);
    }

    /// Adjust `offset` so that `target_row` (a 0-based row index into
    /// the content) is within the current viewport. If it's above
    /// `offset`, scroll up to put it at the top edge. If it's below
    /// `offset + last_viewport_rows`, scroll down so the target sits
    /// at the bottom edge of the viewport. Otherwise, no change.
    ///
    /// Used by modals that auto-scroll to a search match.
    pub fn scroll_to_visible(&mut self, target_row: usize) {
        if self.last_viewport_rows == 0 {
            return;
        }
        if target_row < self.offset {
            self.offset = target_row;
        } else if target_row >= self.offset + self.last_viewport_rows {
            self.offset = target_row + 1 - self.last_viewport_rows;
        }
    }

    /// Clamp `offset` so it never exceeds `content_rows - last_viewport_rows`.
    /// Called by render after the content's wrapped height is known —
    /// without this, fast paging or window resize can leave the offset
    /// dangling past the end of the content.
    pub fn clamp_to_content(&mut self, content_rows: usize) {
        let max = content_rows.saturating_sub(self.last_viewport_rows);
        if self.offset > max {
            self.offset = max;
        }
    }
}

/// Vertical + horizontal scroll state used by the Tracer content
/// modal. Composes `VerticalScrollState` for the up/down axis and
/// adds horizontal offset math alongside.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BidirectionalScrollState {
    pub vertical: VerticalScrollState,
    pub horizontal_offset: usize,
    /// Last known viewport body width in columns. Set by the render
    /// pass each frame.
    pub last_viewport_body_cols: usize,
}

impl BidirectionalScrollState {
    /// Scroll one column left. Clamps to 0.
    pub fn scroll_left(&mut self) {
        self.horizontal_offset = self.horizontal_offset.saturating_sub(1);
    }

    /// Scroll one column right. Clamps to
    /// `content_cols - last_viewport_body_cols`.
    pub fn scroll_right(&mut self, content_cols: usize) {
        let max = content_cols.saturating_sub(self.last_viewport_body_cols);
        self.horizontal_offset = self.horizontal_offset.saturating_add(1).min(max);
    }

    /// Page left by one viewport width.
    pub fn page_left(&mut self) {
        self.horizontal_offset = self
            .horizontal_offset
            .saturating_sub(self.last_viewport_body_cols);
    }

    /// Page right by one viewport width.
    pub fn page_right(&mut self, content_cols: usize) {
        let max = content_cols.saturating_sub(self.last_viewport_body_cols);
        self.horizontal_offset = self
            .horizontal_offset
            .saturating_add(self.last_viewport_body_cols)
            .min(max);
    }

    /// Reset both axes to 0. Does not change the last-viewport
    /// dimensions (those are set by the next render).
    pub fn reset(&mut self) {
        self.vertical.offset = 0;
        self.horizontal_offset = 0;
    }
}

/// Vertical scroll state coupled to a row cursor. The cursor and the
/// viewport offset are kept in sync — `move_up`/`move_down`/`page_*`/
/// `jump_*` adjust both, calling `scroll_to_visible` so the cursor
/// stays inside the rendered window.
///
/// Action history modal uses this to drive its row selection. Use it
/// for any modal that presents a scrollable list with a row cursor.
///
/// Composes `VerticalScrollState` via `Deref` so callers that previously
/// reached for `state.scroll.offset` / `state.scroll.last_viewport_rows`
/// continue to work without change.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CursoredScrollState {
    pub scroll: VerticalScrollState,
    /// Currently selected row (0-based). Callers clamp to `0..len` via
    /// `move_down(len)` / `jump_bottom(len)`.
    pub selected: usize,
}

impl std::ops::Deref for CursoredScrollState {
    type Target = VerticalScrollState;
    fn deref(&self) -> &Self::Target {
        &self.scroll
    }
}

impl std::ops::DerefMut for CursoredScrollState {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.scroll
    }
}

impl CursoredScrollState {
    /// Move the cursor up by 1 (clamped to 0) and ensure it remains
    /// visible.
    pub fn move_up(&mut self) {
        self.selected = self.selected.saturating_sub(1);
        self.scroll.scroll_to_visible(self.selected);
    }

    /// Move the cursor down by 1 (clamped to `len - 1`) and ensure it
    /// remains visible. No-op if `len == 0`.
    pub fn move_down(&mut self, len: usize) {
        if self.selected + 1 < len {
            self.selected += 1;
            self.scroll.scroll_to_visible(self.selected);
        }
    }

    /// Move the cursor up by one viewport (using
    /// `last_viewport_rows`) and ensure visibility.
    pub fn page_up(&mut self) {
        let bump = self.scroll.last_viewport_rows.max(1);
        self.selected = self.selected.saturating_sub(bump);
        self.scroll.page_up();
        self.scroll.scroll_to_visible(self.selected);
    }

    /// Move the cursor down by one viewport (clamped to `len - 1`)
    /// and ensure visibility. No-op if `len == 0`.
    pub fn page_down(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        let bump = self.scroll.last_viewport_rows.max(1);
        self.selected = (self.selected + bump).min(len.saturating_sub(1));
        self.scroll.page_down(len);
        self.scroll.scroll_to_visible(self.selected);
    }

    /// Jump cursor and viewport to the top.
    pub fn jump_top(&mut self) {
        self.selected = 0;
        self.scroll.jump_top();
    }

    /// Jump cursor to `len - 1` and scroll the viewport to show the
    /// bottom. No-op if `len == 0`.
    pub fn jump_bottom(&mut self, len: usize) {
        if len == 0 {
            return;
        }
        self.selected = len - 1;
        self.scroll.jump_bottom(len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_scroll_by_negative_clamps_to_zero() {
        let mut v = VerticalScrollState {
            offset: 0,
            last_viewport_rows: 10,
        };
        v.scroll_by(-5, 100);
        assert_eq!(v.offset, 0);
    }

    #[test]
    fn vertical_scroll_by_positive_clamps_to_max() {
        let mut v = VerticalScrollState {
            offset: 90,
            last_viewport_rows: 10,
        };
        v.scroll_by(50, 100);
        // max = content_rows - viewport = 100 - 10 = 90
        assert_eq!(v.offset, 90);
    }

    #[test]
    fn vertical_page_up_and_down_use_viewport() {
        let mut v = VerticalScrollState {
            offset: 50,
            last_viewport_rows: 10,
        };
        v.page_up();
        assert_eq!(v.offset, 40);
        v.page_down(100);
        assert_eq!(v.offset, 50);
    }

    #[test]
    fn vertical_jump_top_and_bottom() {
        let mut v = VerticalScrollState {
            offset: 42,
            last_viewport_rows: 10,
        };
        v.jump_top();
        assert_eq!(v.offset, 0);
        v.jump_bottom(100);
        assert_eq!(v.offset, 90); // 100 - 10
    }

    #[test]
    fn vertical_page_up_empty_saturates_at_zero() {
        let mut v = VerticalScrollState {
            offset: 5,
            last_viewport_rows: 10,
        };
        v.page_up();
        assert_eq!(v.offset, 0);
    }

    #[test]
    fn bidirectional_horizontal_scroll_bounds() {
        let mut b = BidirectionalScrollState {
            last_viewport_body_cols: 20,
            ..Default::default()
        };
        // scroll right should advance by 1 and clamp at max
        for _ in 0..50 {
            b.scroll_right(30);
        }
        assert_eq!(b.horizontal_offset, 10); // 30 - 20
        // scroll left to 0
        for _ in 0..50 {
            b.scroll_left();
        }
        assert_eq!(b.horizontal_offset, 0);
    }

    #[test]
    fn bidirectional_page_left_right_use_viewport_width() {
        let mut b = BidirectionalScrollState {
            last_viewport_body_cols: 20,
            ..Default::default()
        };
        b.page_right(100);
        assert_eq!(b.horizontal_offset, 20);
        b.page_right(100);
        assert_eq!(b.horizontal_offset, 40);
        b.page_left();
        assert_eq!(b.horizontal_offset, 20);
    }

    #[test]
    fn vertical_page_down_with_content_smaller_than_viewport() {
        let mut v = VerticalScrollState {
            offset: 0,
            last_viewport_rows: 10,
        };
        v.page_down(5); // content fits; no scroll possible
        assert_eq!(v.offset, 0);
    }

    #[test]
    fn vertical_page_up_and_down_noop_with_zero_viewport() {
        let mut v = VerticalScrollState {
            offset: 5,
            last_viewport_rows: 0,
        };
        v.page_up();
        assert_eq!(v.offset, 5, "page_up with viewport=0 must not move");
        v.page_down(100);
        assert_eq!(v.offset, 5, "page_down with viewport=0 must not move");
    }

    #[test]
    fn vertical_jump_bottom_with_content_smaller_than_viewport() {
        let mut v = VerticalScrollState {
            offset: 0,
            last_viewport_rows: 10,
        };
        v.jump_bottom(5); // content fits; jump_bottom should pin at 0
        assert_eq!(v.offset, 0);
    }

    #[test]
    fn bidirectional_page_right_with_content_smaller_than_viewport() {
        let mut b = BidirectionalScrollState {
            last_viewport_body_cols: 20,
            ..Default::default()
        };
        b.page_right(10); // content fits
        assert_eq!(b.horizontal_offset, 0);
    }

    #[test]
    fn vertical_jump_bottom_with_usize_max_sentinel_preserves_render_clamp_pattern() {
        // Bulletins passes usize::MAX as content_rows so the render pass
        // is the single source of truth for the real clamp. The widget
        // must not panic on this sentinel.
        let mut v = VerticalScrollState {
            offset: 0,
            last_viewport_rows: 30,
        };
        v.jump_bottom(usize::MAX);
        assert_eq!(v.offset, usize::MAX - 30);
    }

    #[test]
    fn scroll_to_visible_above_viewport() {
        let mut v = VerticalScrollState {
            offset: 20,
            last_viewport_rows: 10,
        };
        v.scroll_to_visible(5);
        assert_eq!(v.offset, 5);
    }

    #[test]
    fn scroll_to_visible_below_viewport() {
        let mut v = VerticalScrollState {
            offset: 0,
            last_viewport_rows: 10,
        };
        v.scroll_to_visible(25);
        // target=25 must be at the bottom edge → offset = 25 + 1 - 10 = 16
        assert_eq!(v.offset, 16);
    }

    #[test]
    fn scroll_to_visible_inside_viewport_no_change() {
        let mut v = VerticalScrollState {
            offset: 10,
            last_viewport_rows: 10,
        };
        v.scroll_to_visible(15);
        assert_eq!(v.offset, 10);
    }

    #[test]
    fn scroll_to_visible_zero_viewport_is_noop() {
        let mut v = VerticalScrollState {
            offset: 7,
            last_viewport_rows: 0,
        };
        v.scroll_to_visible(0);
        assert_eq!(v.offset, 7);
    }

    #[test]
    fn clamp_to_content_within_bounds_noop() {
        let mut v = VerticalScrollState {
            offset: 5,
            last_viewport_rows: 10,
        };
        v.clamp_to_content(100);
        assert_eq!(v.offset, 5);
    }

    #[test]
    fn clamp_to_content_past_end_clamped() {
        let mut v = VerticalScrollState {
            offset: 200,
            last_viewport_rows: 10,
        };
        v.clamp_to_content(100);
        // max = 100 - 10 = 90
        assert_eq!(v.offset, 90);
    }

    #[test]
    fn clamp_to_content_smaller_than_viewport_clamps_to_zero() {
        let mut v = VerticalScrollState {
            offset: 5,
            last_viewport_rows: 10,
        };
        v.clamp_to_content(3);
        assert_eq!(v.offset, 0);
    }

    #[test]
    fn bidirectional_reset_clears_both_axes() {
        let mut b = BidirectionalScrollState {
            vertical: VerticalScrollState {
                offset: 50,
                last_viewport_rows: 0,
            },
            horizontal_offset: 20,
            last_viewport_body_cols: 0,
        };
        b.reset();
        assert_eq!(b.vertical.offset, 0);
        assert_eq!(b.horizontal_offset, 0);
    }

    #[test]
    fn cursored_move_up_clamps_to_zero() {
        let mut c = CursoredScrollState::default();
        c.scroll.last_viewport_rows = 5;
        c.selected = 3;
        c.move_up();
        assert_eq!(c.selected, 2);
        c.selected = 0;
        c.move_up();
        assert_eq!(c.selected, 0);
    }

    #[test]
    fn cursored_move_down_clamps_to_len_minus_one() {
        let mut c = CursoredScrollState::default();
        c.scroll.last_viewport_rows = 5;
        c.move_down(3);
        assert_eq!(c.selected, 1);
        c.move_down(3);
        assert_eq!(c.selected, 2);
        c.move_down(3);
        assert_eq!(c.selected, 2, "clamped at len-1");
    }

    #[test]
    fn cursored_move_down_noop_on_empty() {
        let mut c = CursoredScrollState::default();
        c.move_down(0);
        assert_eq!(c.selected, 0);
    }

    #[test]
    fn cursored_move_down_auto_scrolls_into_view() {
        let mut c = CursoredScrollState {
            scroll: VerticalScrollState {
                offset: 0,
                last_viewport_rows: 3,
            },
            selected: 0,
        };
        // Cursor at row 5 with viewport size 3 → offset must scroll so 5 is visible.
        for _ in 0..5 {
            c.move_down(20);
        }
        assert_eq!(c.selected, 5);
        // After scroll_to_visible(5) with last_viewport_rows=3 → offset = 5 + 1 - 3 = 3
        assert_eq!(c.scroll.offset, 3);
    }

    #[test]
    fn cursored_jump_bottom_with_empty_is_noop() {
        let mut c = CursoredScrollState::default();
        c.jump_bottom(0);
        assert_eq!(c.selected, 0);
    }

    #[test]
    fn cursored_deref_exposes_inner_offset() {
        let mut c = CursoredScrollState::default();
        c.scroll.last_viewport_rows = 10;
        c.scroll_by(5, 100); // via DerefMut
        assert_eq!(c.offset, 5); // via Deref
    }
}
