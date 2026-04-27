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

/// Style a TLS-cert expiry display by days-until-expiry. Negative values
/// represent already-expired certs.
///
/// Thresholds match the Nodes-panel chip and the node-detail expiry row:
///   - expired (days < 0) or `<7d` → error + BOLD
///   - 7..30d → warning (yellow)
///   - else → muted
pub fn cert_expiry_style(days: i64) -> Style {
    if days < 7 {
        // covers both expired (negative) and <7d
        error().add_modifier(Modifier::BOLD)
    } else if days < 30 {
        warning()
    } else {
        muted()
    }
}

/// Foreground style for an inserted line (`+`) in the content-viewer
/// diff tab. Plain green so `+`/`-` prefixes carry the distinction for
/// colorblind fallback.
pub fn diff_add() -> Style {
    Style::default().fg(Color::Green)
}

/// Foreground style for a deleted line (`-`) in the content-viewer
/// diff tab.
pub fn diff_del() -> Style {
    Style::default().fg(Color::Red)
}

/// Foreground style for the `@@ input L{a} · output L{b} @@` hunk
/// header line in the content-viewer diff tab.
pub fn hunk_header() -> Style {
    Style::default()
        .fg(Color::Magenta)
        .add_modifier(Modifier::DIM)
}

/// Style for a search match (highlighted but not focused).
pub fn search_match() -> Style {
    Style::default().add_modifier(Modifier::UNDERLINED)
}

/// Style for the active/focused search match — the one cycled to with
/// `n` / `N`. Layered on top of [`search_match`] semantically; this
/// helper returns the combined style ready to apply directly.
pub fn search_match_active() -> Style {
    Style::default().add_modifier(Modifier::UNDERLINED | Modifier::REVERSED | Modifier::BOLD)
}

/// Style for the input-bar cursor block (the trailing `"_"` shown
/// while the user is typing into a search prompt).
pub fn search_cursor() -> Style {
    Style::default().add_modifier(Modifier::REVERSED)
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

    #[test]
    fn cert_expiry_style_buckets() {
        let red_bold = error().add_modifier(Modifier::BOLD);
        assert_eq!(cert_expiry_style(-1), red_bold); // expired
        assert_eq!(cert_expiry_style(0), red_bold); // expiring today
        assert_eq!(cert_expiry_style(6), red_bold); // <7d
        assert_eq!(cert_expiry_style(7), warning());
        assert_eq!(cert_expiry_style(29), warning());
        assert_eq!(cert_expiry_style(30), muted());
        assert_eq!(cert_expiry_style(365), muted());
    }

    #[test]
    fn search_match_underlines() {
        assert!(search_match().add_modifier.contains(Modifier::UNDERLINED));
    }

    #[test]
    fn search_match_active_combines_modifiers() {
        let s = search_match_active();
        assert!(s.add_modifier.contains(Modifier::UNDERLINED));
        assert!(s.add_modifier.contains(Modifier::REVERSED));
        assert!(s.add_modifier.contains(Modifier::BOLD));
    }

    #[test]
    fn search_cursor_reverses() {
        assert!(search_cursor().add_modifier.contains(Modifier::REVERSED));
        assert!(!search_cursor().add_modifier.contains(Modifier::UNDERLINED));
    }
}
