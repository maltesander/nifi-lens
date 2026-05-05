//! Shared layout constants for modal sizes and table column widths.
//!
//! Values here are referenced from more than one module. Module-local
//! layout values stay as private `const` in their module.

use ratatui::layout::{Constraint, Direction, Layout, Rect};

/// Error-detail modal: width as a percentage of terminal width, height in rows.
pub const ERROR_DETAIL_MODAL_WIDTH_PCT: u16 = 80;
pub const ERROR_DETAIL_MODAL_HEIGHT: u16 = 15;

/// Browser properties modal: absolute maximum width and height in cells.
/// Callers combine with `area.width.min(..)` / `area.height.min(..)`.
pub const BROWSER_DETAIL_MODAL_MAX_WIDTH: u16 = 90;
pub const BROWSER_DETAIL_MODAL_MAX_HEIGHT: u16 = 24;

/// Label column width in browser detail tables (processor properties,
/// controller-service properties).
pub const DETAIL_LABEL_COL_WIDTH: u16 = 30;

/// Maximum number of validation-error rows shown in a detail panel
/// before the panel stops growing.
pub const VALIDATION_ERROR_ROWS_MAX: usize = 5;

/// Two-column constraint used by detail tables (fixed-width label,
/// fill-remaining value).
pub fn detail_row_constraints() -> [Constraint; 2] {
    [
        Constraint::Length(DETAIL_LABEL_COL_WIDTH),
        Constraint::Fill(1),
    ]
}

/// Vertical split with a fixed-height header and footer. The body
/// (middle rect) takes the remaining rows.
///
/// The body uses `Constraint::Min(0)` as the sole flex slot; for
/// layouts with multiple flex slots or weighted `Fill(n)` shares,
/// inline the `Layout::default()` call instead.
pub fn split_header_body_footer(area: Rect, header: u16, footer: u16) -> [Rect; 3] {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header),
            Constraint::Min(0),
            Constraint::Length(footer),
        ])
        .split(area);
    [rows[0], rows[1], rows[2]]
}

/// Vertical split: fixed-shape first row, everything below is the
/// second row.
pub fn split_two_rows(area: Rect, first: Constraint) -> [Rect; 2] {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([first, Constraint::Min(0)])
        .split(area);
    [rows[0], rows[1]]
}

/// Horizontal split: fixed-shape first column, everything to the
/// right is the second column.
pub fn split_two_cols(area: Rect, first: Constraint) -> [Rect; 2] {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([first, Constraint::Min(0)])
        .split(area);
    [cols[0], cols[1]]
}

/// Center a rect inside `area`: width is `pct_x` percent of `area.width`,
/// height is fixed in rows. Used by modals that want a percentage-driven
/// horizontal footprint.
pub fn center_percent(area: Rect, pct_x: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vertical[1])[1]
}

/// Center a rect inside `area` with absolute width and height in cells.
/// Both dimensions are clamped to `area` so modals don't overflow on
/// narrow terminals.
pub fn center_absolute(area: Rect, width: u16, height: u16) -> Rect {
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
}

#[cfg(test)]
mod split_helper_tests {
    use super::*;
    use ratatui::layout::{Constraint, Rect};

    #[test]
    fn split_header_body_footer_divides_area() {
        let area = Rect::new(0, 0, 80, 30);
        let [h, b, f] = split_header_body_footer(area, 2, 1);
        assert_eq!(h.height, 2);
        assert_eq!(f.height, 1);
        assert_eq!(b.height, 27);
        assert_eq!(h.y + h.height, b.y);
        assert_eq!(b.y + b.height, f.y);
    }

    #[test]
    fn split_two_rows_with_fixed_first() {
        let area = Rect::new(0, 0, 80, 30);
        let [a, b] = split_two_rows(area, Constraint::Length(5));
        assert_eq!(a.height, 5);
        assert_eq!(b.height, 25);
        assert_eq!(a.y + a.height, b.y);
    }

    #[test]
    fn split_two_cols_with_fixed_first() {
        let area = Rect::new(0, 0, 80, 30);
        let [left, right] = split_two_cols(area, Constraint::Length(20));
        assert_eq!(left.width, 20);
        assert_eq!(right.width, 60);
        assert_eq!(left.x + left.width, right.x);
    }

    #[test]
    fn split_header_body_footer_handles_zero_header_or_footer() {
        let area = Rect::new(0, 0, 80, 30);
        let [h, b, f] = split_header_body_footer(area, 0, 0);
        assert_eq!(h.height, 0);
        assert_eq!(f.height, 0);
        assert_eq!(b.height, 30);
    }

    #[test]
    fn split_header_body_footer_handles_overflow() {
        // Header + footer exceed the total area height.
        // Ratatui clamps gracefully; body gets 0 rows.
        let area = Rect::new(0, 0, 80, 3);
        let [h, b, f] = split_header_body_footer(area, 5, 5);
        // Total height is preserved across the three rects.
        assert_eq!(h.height + b.height + f.height, area.height);
        // Body is zero-sized; headers are clamped to the available space.
        assert_eq!(b.height, 0);
    }

    #[test]
    fn center_percent_fits_inside_area() {
        let area = Rect::new(0, 0, 100, 30);
        let r = center_percent(area, 60, 10);
        assert_eq!(r.width, 60);
        assert_eq!(r.height, 10);
        assert!(r.x >= area.x && r.x + r.width <= area.x + area.width);
        assert!(r.y >= area.y && r.y + r.height <= area.y + area.height);
    }

    #[test]
    fn center_absolute_clamps_to_area() {
        let area = Rect::new(0, 0, 30, 10);
        let r = center_absolute(area, 100, 100);
        assert_eq!(r.width, 30);
        assert_eq!(r.height, 10);
    }

    #[test]
    fn center_absolute_centers_smaller_rect() {
        let area = Rect::new(0, 0, 80, 24);
        let r = center_absolute(area, 40, 12);
        assert_eq!(r.width, 40);
        assert_eq!(r.height, 12);
        assert_eq!(r.x, 20);
        assert_eq!(r.y, 6);
    }
}
