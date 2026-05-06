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
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

use crate::app::state::{AppState, BannerSeverity};
use crate::theme;

/// Refresh-age threshold above which the glyph renders in `theme::warning()`.
/// At healthy default cadences the longest-period fetcher (`about`, 1 min) is
/// the main signal; if NO endpoint has reported in 60s, the cluster is likely
/// degrading.
const STALE_WARN_SECS: u64 = 60;

/// Refresh-age threshold above which the glyph renders in `theme::error()`.
/// 5 min matches the recommended-range maximum for every default-cadence
/// fetcher (see `config/polling.rs`); past that, the cluster is almost
/// certainly unreachable.
const STALE_ERROR_SECS: u64 = 5 * 60;

/// Render footer row 1 into `area`.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let age_secs = state.last_refresh.elapsed().as_secs();
    let refresh_text = format_refresh_age(age_secs);
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
    render_refresh_age(frame, chunks[1], &refresh_text, age_secs);
}

/// Pick the style for the refresh-age glyph based on how long since any
/// fetcher last reported success. `last_refresh` is bumped on every
/// successful `ClusterUpdate`, so a climbing counter signals systemic
/// failure across all 12 endpoints.
fn refresh_age_style(age_secs: u64) -> ratatui::style::Style {
    if age_secs >= STALE_ERROR_SECS {
        theme::error()
    } else if age_secs >= STALE_WARN_SECS {
        theme::warning()
    } else {
        theme::muted()
    }
}

fn render_banner(frame: &mut Frame, area: Rect, state: &AppState) {
    let line = match &state.status.banner {
        Some(banner) => {
            let style = match banner.severity {
                BannerSeverity::Error => theme::error(),
                BannerSeverity::Warning => theme::warning(),
                BannerSeverity::Info => theme::info(),
            };
            let msg = truncate_to_width(&banner.message, area.width as usize);
            Line::from(Span::styled(msg, style))
        }
        None => {
            // While the cluster store is still in its first poll cycle,
            // surface boot progress in the empty banner slot. Real
            // banners (warnings, errors, info) take priority — see the
            // `Some(banner)` arm above. Once `ready == total` the chip
            // disappears and the slot returns to empty.
            let (ready, total) = state.cluster.snapshot.ready_count();
            if ready < total {
                let chip = format!("init: {ready}/{total} endpoints ready");
                let chip = truncate_to_width(&chip, area.width as usize);
                Line::from(Span::styled(chip, theme::muted()))
            } else {
                Line::from(Span::raw(""))
            }
        }
    };
    frame.render_widget(Paragraph::new(line), area);
}

/// Truncates `s` to at most `max_width` terminal columns, appending `…` when
/// the text is shortened. Returns an owned `String` in all cases.
fn truncate_to_width(s: &str, max_width: usize) -> String {
    if s.width() <= max_width {
        return s.to_string();
    }
    if max_width == 0 {
        return String::new();
    }
    let mut out = String::new();
    let mut used = 0usize;
    for ch in s.chars() {
        let cw = ch.width().unwrap_or(0);
        if used + cw + 1 > max_width {
            break;
        }
        out.push(ch);
        used += cw;
    }
    out.push('\u{2026}'); // …
    out
}

fn render_refresh_age(frame: &mut Frame, area: Rect, refresh_text: &str, age_secs: u64) {
    let style = refresh_age_style(age_secs);
    let line = Line::from(Span::styled(refresh_text.to_string(), style));
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
        // The version used to live in the banner slot; it now lives in
        // the hint bar, so it must NOT appear in status_bar output.
        assert!(
            !out.contains("nifi-lens v"),
            "version must not appear in status_bar; it moved to hint_bar"
        );
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
    fn renders_init_chip_when_no_banner_and_endpoints_loading() {
        let state = fresh_state();
        // fresh_state has all endpoints in EndpointState::Loading by
        // default — ready_count() returns (0, 11).
        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains("init:"),
            "init chip should appear when endpoints are still loading"
        );
        let expected = format!("0/{}", crate::cluster::ClusterEndpoint::COUNT);
        assert!(
            out.contains(&expected),
            "expected `{expected}` endpoints ready on fresh state; got: {out:?}"
        );
    }

    #[test]
    fn real_banner_overrides_init_chip() {
        let mut state = fresh_state();
        // fresh_state is in Loading — would show the init chip — but a
        // real banner must take precedence.
        state.status.banner = Some(Banner {
            severity: BannerSeverity::Warning,
            message: "something went wrong".to_string(),
            detail: None,
        });
        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("something went wrong"));
        assert!(
            !out.contains("init:"),
            "init chip must not appear when a real banner is set"
        );
    }

    #[test]
    fn refresh_age_style_healthy_below_60s() {
        // Healthy: muted style — same as default 0s.
        let healthy = refresh_age_style(0);
        let almost_warn = refresh_age_style(STALE_WARN_SECS - 1);
        assert_eq!(healthy, theme::muted());
        assert_eq!(almost_warn, theme::muted());
    }

    #[test]
    fn refresh_age_style_warning_at_threshold() {
        let warn_at = refresh_age_style(STALE_WARN_SECS);
        let warn_mid = refresh_age_style(STALE_ERROR_SECS - 1);
        assert_eq!(warn_at, theme::warning());
        assert_eq!(warn_mid, theme::warning());
    }

    #[test]
    fn refresh_age_style_error_at_threshold() {
        let error_at = refresh_age_style(STALE_ERROR_SECS);
        let error_far = refresh_age_style(STALE_ERROR_SECS * 10);
        assert_eq!(error_at, theme::error());
        assert_eq!(error_far, theme::error());
    }

    #[test]
    fn render_uses_warning_style_when_data_is_stale() {
        use ratatui::style::Color;
        use std::time::{Duration, Instant};

        let mut state = fresh_state();
        // Simulate "no successful fetch for 90s" — past the warn threshold.
        state.last_refresh = Instant::now() - Duration::from_secs(STALE_WARN_SECS + 30);

        let backend = TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();

        // Find the cell containing the refresh glyph and verify its style
        // matches the warning theme — not muted.
        let buf = term.backend().buffer().clone();
        let warning_fg = match theme::warning().fg {
            Some(c) => c,
            None => Color::Reset,
        };
        let glyph_cell = (0..buf.area().width)
            .map(|x| buf[(x, 0)].clone())
            .find(|c| c.symbol() == "\u{27f3}")
            .expect("refresh glyph must be rendered");
        assert_eq!(
            glyph_cell.fg, warning_fg,
            "stale refresh-age glyph must render in warning style; got fg={:?}",
            glyph_cell.fg
        );
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
