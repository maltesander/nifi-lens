//! Color and style constants. No runtime theming in Phase 0.

use ratatui::style::{Color, Modifier, Style};

pub fn muted() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn accent() -> Style {
    Style::default()
        .fg(Color::Cyan)
        .add_modifier(Modifier::BOLD)
}

pub fn error() -> Style {
    Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)
}

pub fn warning() -> Style {
    Style::default().fg(Color::Yellow)
}

pub fn info() -> Style {
    Style::default().fg(Color::Blue)
}

pub fn cursor_row() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

pub fn success() -> Style {
    Style::default().fg(Color::Green)
}

pub fn disabled() -> Style {
    Style::default().fg(Color::DarkGray)
}

pub fn bold() -> Style {
    Style::default().add_modifier(Modifier::BOLD)
}

pub fn highlight() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
}

/// Dim color used for unfocused section borders and separator rules.
/// Matches the existing `muted()` hue so pre-Phase 7 borders read
/// identically; future tuning can split them apart.
pub fn border_dim() -> Style {
    muted()
}

/// Maps a percentage (0-100+) to a severity style. Use at render call
/// sites that currently inline `match pct { p if p >= 90.0 => ..., ... }`
/// blocks — Health heap, queue fill, etc.
pub fn severity_by_pct(pct: f32) -> Style {
    if pct >= 90.0 {
        error()
    } else if pct >= 75.0 {
        warning()
    } else {
        Style::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn severity_by_pct_low_is_default() {
        let s = severity_by_pct(50.0);
        assert_eq!(s, Style::default());
    }

    #[test]
    fn severity_by_pct_at_75_is_warning() {
        let s = severity_by_pct(75.0);
        assert_eq!(s, warning());
    }

    #[test]
    fn severity_by_pct_under_75_is_default() {
        let s = severity_by_pct(74.9);
        assert_eq!(s, Style::default());
    }

    #[test]
    fn severity_by_pct_at_90_is_error() {
        let s = severity_by_pct(90.0);
        assert_eq!(s, error());
    }

    #[test]
    fn severity_by_pct_under_90_is_warning() {
        let s = severity_by_pct(89.9);
        assert_eq!(s, warning());
    }

    #[test]
    fn severity_by_pct_over_100_is_error() {
        let s = severity_by_pct(150.0);
        assert_eq!(s, error());
    }
}
