//! Typed enums for NiFi processor + controller-service state strings
//! returned by the REST API. Centralized so case-insensitive parsing
//! and display styling live in one place.

use crate::theme;
use ratatui::style::{Modifier, Style};

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControllerServiceState {
    Enabled,
    Disabled,
    Enabling,
    Disabling,
    Invalid,
    Unknown,
}

impl ControllerServiceState {
    /// Parse a NiFi-wire CS state string (case-insensitive).
    /// Unrecognized values map to `Unknown`.
    pub fn from_wire(s: &str) -> Self {
        if s.eq_ignore_ascii_case("ENABLED") {
            Self::Enabled
        } else if s.eq_ignore_ascii_case("DISABLED") {
            Self::Disabled
        } else if s.eq_ignore_ascii_case("ENABLING") {
            Self::Enabling
        } else if s.eq_ignore_ascii_case("DISABLING") {
            Self::Disabling
        } else if s.eq_ignore_ascii_case("INVALID") {
            Self::Invalid
        } else {
            Self::Unknown
        }
    }

    /// Style used for CS state labels in the Browser tree + detail
    /// panes (non-bold variant). Mirrors the legacy mapping: enabled
    /// → success, disabled → disabled, enabling/disabling → info,
    /// anything else → muted.
    pub fn style(self) -> Style {
        match self {
            Self::Enabled => theme::success(),
            Self::Disabled => theme::disabled(),
            Self::Enabling | Self::Disabling => theme::info(),
            _ => theme::muted(),
        }
    }

    /// Bold-variant style used by the CS state badge in detail headers.
    /// Mirrors the legacy mapping: enabled → success+BOLD, disabled →
    /// muted, anything else → warning.
    pub fn badge_style(self) -> Style {
        match self {
            Self::Enabled => theme::success().add_modifier(Modifier::BOLD),
            Self::Disabled => theme::muted(),
            _ => theme::warning(),
        }
    }

    /// Style used for the CS state cell in the Referencing table on
    /// CS detail panes. Mirrors the legacy mapping: enabled → success,
    /// disabled → disabled, anything else → warning.
    pub fn referencing_style(self) -> Style {
        match self {
            Self::Enabled => theme::success(),
            Self::Disabled => theme::disabled(),
            _ => theme::warning(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PortStatus {
    Running,
    Stopped,
    Invalid,
    Disabled,
    Validating,
    Unknown,
}

impl PortStatus {
    /// Parse a NiFi-wire port run-status string (case-insensitive).
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

    /// Style used for port state labels in headers and identity panes.
    /// Mirrors the legacy `port::state_style` mapping byte-for-byte:
    /// running → success+BOLD, stopped → warning, disabled → muted,
    /// anything else (invalid / validating / unknown) → info.
    pub fn style(self) -> Style {
        match self {
            Self::Running => theme::success().add_modifier(Modifier::BOLD),
            Self::Stopped => theme::warning(),
            Self::Disabled => theme::muted(),
            Self::Invalid | Self::Validating | Self::Unknown => theme::info(),
        }
    }
}

/// `transmission_status` from `RemoteProcessGroupStatusDto`. NiFi
/// documents only `"Transmitting"` and `"Not Transmitting"`; anything
/// else (including the empty string before the first snapshot lands)
/// maps to `Unknown` and renders as the muted state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmissionStatus {
    Transmitting,
    NotTransmitting,
    Unknown,
}

impl TransmissionStatus {
    /// Parse a NiFi-wire transmission status (case-insensitive). Unknown
    /// and empty strings collapse to `Unknown`.
    pub fn from_wire(s: &str) -> Self {
        if s.eq_ignore_ascii_case("Transmitting") {
            Self::Transmitting
        } else if s.eq_ignore_ascii_case("Not Transmitting") {
            Self::NotTransmitting
        } else {
            Self::Unknown
        }
    }

    /// Glyph + style used by the RPG run-icon column. Transmitting uses
    /// the accent ▶; everything else (including unknown) uses the muted
    /// ■ — we deliberately don't try to interpret intermediate states
    /// that NiFi doesn't document.
    pub fn icon(self) -> (char, Style) {
        match self {
            Self::Transmitting => ('▶', theme::accent()),
            Self::NotTransmitting | Self::Unknown => ('■', theme::muted()),
        }
    }

    /// Style used for the transmission-state span in panel titles.
    pub fn style(self) -> Style {
        match self {
            Self::Transmitting => theme::accent(),
            Self::NotTransmitting | Self::Unknown => theme::muted(),
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

    #[test]
    fn controller_service_state_from_wire() {
        assert_eq!(
            ControllerServiceState::from_wire("ENABLED"),
            ControllerServiceState::Enabled
        );
        assert_eq!(
            ControllerServiceState::from_wire("enabled"),
            ControllerServiceState::Enabled
        );
        assert_eq!(
            ControllerServiceState::from_wire("DISABLED"),
            ControllerServiceState::Disabled
        );
        assert_eq!(
            ControllerServiceState::from_wire("Enabling"),
            ControllerServiceState::Enabling
        );
        assert_eq!(
            ControllerServiceState::from_wire("DISABLING"),
            ControllerServiceState::Disabling
        );
        assert_eq!(
            ControllerServiceState::from_wire("INVALID"),
            ControllerServiceState::Invalid
        );
        assert_eq!(
            ControllerServiceState::from_wire(""),
            ControllerServiceState::Unknown
        );
        assert_eq!(
            ControllerServiceState::from_wire("GARBAGE"),
            ControllerServiceState::Unknown
        );
    }

    #[test]
    fn controller_service_state_styles() {
        assert_eq!(
            ControllerServiceState::Enabled.style(),
            crate::theme::success()
        );
        assert_eq!(
            ControllerServiceState::Disabled.style(),
            crate::theme::disabled()
        );
        assert_eq!(
            ControllerServiceState::Enabling.style(),
            crate::theme::info()
        );
        assert_eq!(
            ControllerServiceState::Disabling.style(),
            crate::theme::info()
        );
        assert_eq!(
            ControllerServiceState::Invalid.style(),
            crate::theme::muted()
        );
        assert_eq!(
            ControllerServiceState::Unknown.style(),
            crate::theme::muted()
        );
    }

    #[test]
    fn controller_service_state_badge_styles() {
        assert_eq!(
            ControllerServiceState::Enabled.badge_style(),
            crate::theme::success().add_modifier(ratatui::style::Modifier::BOLD)
        );
        assert_eq!(
            ControllerServiceState::Disabled.badge_style(),
            crate::theme::muted()
        );
        // Any other variant falls through to warning.
        assert_eq!(
            ControllerServiceState::Invalid.badge_style(),
            crate::theme::warning()
        );
        assert_eq!(
            ControllerServiceState::Enabling.badge_style(),
            crate::theme::warning()
        );
        assert_eq!(
            ControllerServiceState::Unknown.badge_style(),
            crate::theme::warning()
        );
    }

    #[test]
    fn controller_service_state_referencing_styles() {
        assert_eq!(
            ControllerServiceState::Enabled.referencing_style(),
            crate::theme::success()
        );
        assert_eq!(
            ControllerServiceState::Disabled.referencing_style(),
            crate::theme::disabled()
        );
        // Everything else falls through to warning.
        assert_eq!(
            ControllerServiceState::Enabling.referencing_style(),
            crate::theme::warning()
        );
        assert_eq!(
            ControllerServiceState::Disabling.referencing_style(),
            crate::theme::warning()
        );
        assert_eq!(
            ControllerServiceState::Invalid.referencing_style(),
            crate::theme::warning()
        );
        assert_eq!(
            ControllerServiceState::Unknown.referencing_style(),
            crate::theme::warning()
        );
    }

    #[test]
    fn port_status_from_wire_case_insensitive() {
        assert_eq!(PortStatus::from_wire("RUNNING"), PortStatus::Running);
        assert_eq!(PortStatus::from_wire("running"), PortStatus::Running);
        assert_eq!(PortStatus::from_wire("Stopped"), PortStatus::Stopped);
        assert_eq!(PortStatus::from_wire("INVALID"), PortStatus::Invalid);
        assert_eq!(PortStatus::from_wire("DISABLED"), PortStatus::Disabled);
        assert_eq!(PortStatus::from_wire("VALIDATING"), PortStatus::Validating);
        assert_eq!(PortStatus::from_wire(""), PortStatus::Unknown);
        assert_eq!(PortStatus::from_wire("GARBAGE"), PortStatus::Unknown);
    }

    #[test]
    fn transmission_status_from_wire_case_insensitive() {
        assert_eq!(
            TransmissionStatus::from_wire("Transmitting"),
            TransmissionStatus::Transmitting
        );
        assert_eq!(
            TransmissionStatus::from_wire("transmitting"),
            TransmissionStatus::Transmitting
        );
        assert_eq!(
            TransmissionStatus::from_wire("Not Transmitting"),
            TransmissionStatus::NotTransmitting
        );
        assert_eq!(
            TransmissionStatus::from_wire("NOT TRANSMITTING"),
            TransmissionStatus::NotTransmitting
        );
        assert_eq!(
            TransmissionStatus::from_wire(""),
            TransmissionStatus::Unknown
        );
        assert_eq!(
            TransmissionStatus::from_wire("Validating"),
            TransmissionStatus::Unknown
        );
    }

    #[test]
    fn transmission_status_icon_and_style() {
        assert_eq!(TransmissionStatus::Transmitting.icon().0, '▶');
        assert_eq!(
            TransmissionStatus::Transmitting.icon().1,
            crate::theme::accent()
        );
        assert_eq!(TransmissionStatus::NotTransmitting.icon().0, '■');
        assert_eq!(
            TransmissionStatus::NotTransmitting.icon().1,
            crate::theme::muted()
        );
        assert_eq!(TransmissionStatus::Unknown.icon().0, '■');
        assert_eq!(
            TransmissionStatus::Transmitting.style(),
            crate::theme::accent()
        );
        assert_eq!(TransmissionStatus::Unknown.style(), crate::theme::muted());
    }

    #[test]
    fn port_status_styles() {
        // Running keeps the BOLD modifier from the legacy state_style.
        assert_eq!(
            PortStatus::Running.style(),
            crate::theme::success().add_modifier(Modifier::BOLD)
        );
        assert_eq!(PortStatus::Stopped.style(), crate::theme::warning());
        assert_eq!(PortStatus::Disabled.style(), crate::theme::muted());
        // Invalid / Validating / Unknown all collapse to the legacy `_` arm: info.
        assert_eq!(PortStatus::Invalid.style(), crate::theme::info());
        assert_eq!(PortStatus::Validating.style(), crate::theme::info());
        assert_eq!(PortStatus::Unknown.style(), crate::theme::info());
    }
}
