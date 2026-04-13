//! Footer row 1: left-aligned banner, right-aligned refresh age.
//!
//! Spec: UI reorg 2026-04-13, Layout C chrome. Severities drive the
//! banner color — INFO = info(), WARN = warning(), ERROR = error().
//! Refresh age is always rendered in muted style with a ⟳ glyph prefix.
//!
//! This widget mirrors `top_bar`'s split pattern: one public `render`
//! splits the row horizontally and delegates to two private helpers,
//! `render_banner` and `render_refresh_age`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, BannerSeverity};
use crate::theme;

/// Render footer row 1 into `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let refresh_text = format_refresh_age(state.last_refresh.elapsed().as_secs());
    // Reserve enough columns on the right for the refresh-age glyph +
    // text, plus one column of padding. Clamp to the area width so the
    // split is always valid even on a 10-column terminal.
    let refresh_cols = refresh_text
        .width()
        .saturating_add(1)
        .min(area.width as usize) as u16;

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Fill(1), Constraint::Length(refresh_cols)])
        .split(area);

    render_banner(frame, chunks[0], state);
    render_refresh_age(frame, chunks[1], &refresh_text);
}

fn render_banner(frame: &mut Frame, area: Rect, state: &AppState) {
    let line = match &state.status.banner {
        Some(banner) => {
            let style = match banner.severity {
                BannerSeverity::Error => theme::error(),
                BannerSeverity::Warning => theme::warning(),
                BannerSeverity::Info => theme::info(),
            };
            Line::from(Span::styled(banner.message.clone(), style))
        }
        None => Line::from(Span::styled(
            format!("nifi-lens {}", env!("CARGO_PKG_VERSION")),
            theme::muted(),
        )),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_refresh_age(frame: &mut Frame, area: Rect, refresh_text: &str) {
    let line = Line::from(Span::styled(refresh_text.to_string(), theme::muted()));
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), area);
}

/// Format a seconds-since-refresh value as `⟳ Ns ago` / `⟳ Nm ago` / `⟳ Nh ago`.
fn format_refresh_age(seconds: u64) -> String {
    if seconds < 60 {
        format!("\u{27f3} {seconds}s ago")
    } else if seconds < 3600 {
        format!("\u{27f3} {}m ago", seconds / 60)
    } else {
        format!("\u{27f3} {}h ago", seconds / 3600)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{Banner, BannerSeverity};
    use crate::test_support::fresh_state;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn format_refresh_age_seconds() {
        assert_eq!(format_refresh_age(3), "\u{27f3} 3s ago");
    }

    #[test]
    fn format_refresh_age_boundary_59s() {
        assert_eq!(format_refresh_age(59), "\u{27f3} 59s ago");
    }

    #[test]
    fn format_refresh_age_minutes() {
        assert_eq!(format_refresh_age(125), "\u{27f3} 2m ago");
    }

    #[test]
    fn format_refresh_age_boundary_60s() {
        assert_eq!(format_refresh_age(60), "\u{27f3} 1m ago");
    }

    #[test]
    fn format_refresh_age_hours() {
        assert_eq!(format_refresh_age(7_200), "\u{27f3} 2h ago");
    }

    #[test]
    fn renders_refresh_age_without_banner() {
        let state = fresh_state();
        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("\u{27f3}"), "refresh glyph should be present");
        assert!(out.contains("ago"), "refresh age suffix should be present");
    }

    #[test]
    fn renders_warning_banner_text() {
        let mut state = fresh_state();
        state.status.banner = Some(Banner {
            severity: BannerSeverity::Warning,
            message: "flowfile abc not traceable".to_string(),
            detail: None,
        });
        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("flowfile abc not traceable"));
    }

    #[test]
    fn renders_error_banner_text() {
        let mut state = fresh_state();
        state.status.banner = Some(Banner {
            severity: BannerSeverity::Error,
            message: "query failed".to_string(),
            detail: None,
        });
        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("query failed"));
    }

    #[test]
    fn render_works_on_very_narrow_terminal() {
        // Verify the Layout constraints don't panic when the refresh
        // text is wider than the total area. The render call must not
        // panic even if the right chunk has zero width.
        let state = fresh_state();
        let backend = TestBackend::new(5, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
    }
}
