//! Shared layout constants for modal sizes and table column widths.
//!
//! Values here are referenced from more than one module. Module-local
//! layout values stay as private `const` in their module.

use ratatui::layout::Constraint;

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
