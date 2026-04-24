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
}
