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
}
