//! Mapping from NiFi `run_status` strings to a (glyph, style) pair.
//!
//! Used by the Browser tree row renderer and the fuzzy find modal's
//! State column. Centralised here so theme tuning touches one file.

use ratatui::style::Style;

/// Maps NiFi's `run_status` string to a (glyph, style) pair for a
/// Processor row. Unknown values fall back to the default ● glyph
/// unstyled.
pub fn processor_run_icon(run_status: &str) -> (char, Style) {
    crate::client::status::ProcessorStatus::from_wire(run_status).icon()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn processor_icon_running_is_green_filled_circle() {
        let (glyph, style) = processor_run_icon("RUNNING");
        assert_eq!(glyph, '\u{25CF}');
        assert_eq!(style, theme::success());
    }

    #[test]
    fn processor_icon_stopped_is_yellow_dotted_circle() {
        let (glyph, style) = processor_run_icon("STOPPED");
        assert_eq!(glyph, '\u{25CC}');
        assert_eq!(style, theme::warning());
    }

    #[test]
    fn processor_icon_invalid_is_red_warning() {
        let (glyph, style) = processor_run_icon("INVALID");
        assert_eq!(glyph, '\u{26A0}');
        assert_eq!(style, theme::error());
    }

    #[test]
    fn processor_icon_disabled_is_gray_empty() {
        let (glyph, style) = processor_run_icon("DISABLED");
        assert_eq!(glyph, '\u{2300}');
        assert_eq!(style, theme::disabled());
    }

    #[test]
    fn processor_icon_validating_is_blue_half() {
        let (glyph, style) = processor_run_icon("VALIDATING");
        assert_eq!(glyph, '\u{25D0}');
        assert_eq!(style, theme::info());
    }

    #[test]
    fn processor_icon_unknown_falls_back_to_default() {
        let (glyph, style) = processor_run_icon("");
        assert_eq!(glyph, '\u{25CF}');
        assert_eq!(style, Style::default());

        let (glyph2, style2) = processor_run_icon("garbage");
        assert_eq!(glyph2, '\u{25CF}');
        assert_eq!(style2, Style::default());
    }
}
