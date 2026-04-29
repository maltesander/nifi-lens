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

/// Tree-row prefix glyph for a Remote Process Group.
///
/// `▶` accent for `transmission_status == "Transmitting"`, `■` muted
/// otherwise. Anything other than `"Transmitting"` (including the
/// empty string before the first snapshot lands) renders as the muted
/// square — we deliberately don't try to interpret intermediate
/// states that NiFi does not document.
pub fn transmission_icon(transmission_status: &str) -> (char, Style) {
    if transmission_status == "Transmitting" {
        ('▶', crate::theme::accent())
    } else {
        ('■', crate::theme::muted())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;

    #[test]
    fn transmission_icon_transmitting_returns_accent_play_glyph() {
        let (ch, style) = transmission_icon("Transmitting");
        assert_eq!(ch, '▶');
        assert_eq!(style, crate::theme::accent());
    }

    #[test]
    fn transmission_icon_not_transmitting_returns_muted_square() {
        let (ch, style) = transmission_icon("Not Transmitting");
        assert_eq!(ch, '■');
        assert_eq!(style, crate::theme::muted());
    }

    #[test]
    fn transmission_icon_unknown_empty_string_returns_muted_square() {
        let (ch, style) = transmission_icon("");
        assert_eq!(ch, '■');
        assert_eq!(style, crate::theme::muted());
    }

    #[test]
    fn transmission_icon_unknown_nonempty_string_returns_muted_square() {
        let (ch, style) = transmission_icon("Validating");
        assert_eq!(ch, '■');
        assert_eq!(style, crate::theme::muted());
    }

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
