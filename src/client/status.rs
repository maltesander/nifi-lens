//! Typed enums for NiFi processor + controller-service state strings
//! returned by the REST API. Centralized so case-insensitive parsing
//! and display styling live in one place.

use crate::theme;
use ratatui::style::Style;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessorStatus {
    Running,
    Stopped,
    Invalid,
    Disabled,
    Validating,
    Unknown,
}

impl ProcessorStatus {
    /// Parse a NiFi-wire status string (case-insensitive).
    /// Unrecognized values map to `Unknown`.
    pub fn from_wire(s: &str) -> Self {
        if s.eq_ignore_ascii_case("RUNNING") {
            Self::Running
        } else if s.eq_ignore_ascii_case("STOPPED") {
            Self::Stopped
        } else if s.eq_ignore_ascii_case("INVALID") {
            Self::Invalid
        } else if s.eq_ignore_ascii_case("DISABLED") {
            Self::Disabled
        } else if s.eq_ignore_ascii_case("VALIDATING") {
            Self::Validating
        } else {
            Self::Unknown
        }
    }

    /// The ratatui style used for this status in tables and lists.
    pub fn style(self) -> Style {
        match self {
            Self::Running => theme::success(),
            Self::Stopped => theme::warning(),
            Self::Invalid => theme::error(),
            Self::Disabled => theme::disabled(),
            Self::Validating => theme::info(),
            Self::Unknown => Style::default(),
        }
    }

    /// Glyph + style used by the run-icon column.
    pub fn icon(self) -> (char, Style) {
        match self {
            Self::Running => ('\u{25CF}', theme::success()),
            Self::Stopped => ('\u{25CC}', theme::warning()),
            Self::Invalid => ('\u{26A0}', theme::error()),
            Self::Disabled => ('\u{2300}', theme::disabled()),
            Self::Validating => ('\u{25D0}', theme::info()),
            Self::Unknown => ('\u{25CF}', Style::default()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_wire_case_insensitive() {
        assert_eq!(
            ProcessorStatus::from_wire("RUNNING"),
            ProcessorStatus::Running
        );
        assert_eq!(
            ProcessorStatus::from_wire("running"),
            ProcessorStatus::Running
        );
        assert_eq!(
            ProcessorStatus::from_wire("Stopped"),
            ProcessorStatus::Stopped
        );
        assert_eq!(
            ProcessorStatus::from_wire("INVALID"),
            ProcessorStatus::Invalid
        );
        assert_eq!(
            ProcessorStatus::from_wire("DISABLED"),
            ProcessorStatus::Disabled
        );
        assert_eq!(
            ProcessorStatus::from_wire("VALIDATING"),
            ProcessorStatus::Validating
        );
        assert_eq!(ProcessorStatus::from_wire(""), ProcessorStatus::Unknown);
        assert_eq!(
            ProcessorStatus::from_wire("GARBAGE"),
            ProcessorStatus::Unknown
        );
    }

    #[test]
    fn icon_maps_expected_glyphs() {
        assert_eq!(ProcessorStatus::Running.icon().0, '\u{25CF}');
        assert_eq!(ProcessorStatus::Stopped.icon().0, '\u{25CC}');
        assert_eq!(ProcessorStatus::Invalid.icon().0, '\u{26A0}');
        assert_eq!(ProcessorStatus::Disabled.icon().0, '\u{2300}');
        assert_eq!(ProcessorStatus::Validating.icon().0, '\u{25D0}');
    }

    #[test]
    fn unknown_icon_matches_legacy_fallback() {
        // Legacy match fell through to ('\u{25CF}', Style::default()).
        let (glyph, style) = ProcessorStatus::Unknown.icon();
        assert_eq!(glyph, '\u{25CF}');
        assert_eq!(style, Style::default());
    }

    #[test]
    fn style_maps_match_theme() {
        assert_eq!(ProcessorStatus::Running.style(), crate::theme::success());
        assert_eq!(ProcessorStatus::Stopped.style(), crate::theme::warning());
        assert_eq!(ProcessorStatus::Invalid.style(), crate::theme::error());
        assert_eq!(ProcessorStatus::Disabled.style(), crate::theme::disabled());
        assert_eq!(ProcessorStatus::Validating.style(), crate::theme::info());
        assert_eq!(
            ProcessorStatus::Unknown.style(),
            ratatui::style::Style::default()
        );
    }
}
