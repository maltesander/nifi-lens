//! Sparkline helpers for the Browser detail identity-panel inline
//! strip. Pure rendering — no state, no allocation beyond the
//! returned `Line`.
//!
//! The values list is right-aligned (newest = rightmost) and
//! truncated from the LEFT if `width` is narrower than the values
//! count. Each value renders as one of the unicode block-element
//! glyphs scaled to the max value in the visible window.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

const GLYPHS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];

/// Render one sparkline row: `<label_pad><glyphs> peak <peak_label>`.
///
/// `width` is the total cell budget. The fn allocates a fixed
/// `label_width` for the leading label, leaves a single space gap,
/// and uses the rest for glyphs + trailing peak label. If the budget
/// is below the label + peak-label + a single glyph, the row renders
/// as `<label> —` (muted dash placeholder).
pub fn render_sparkline_row<'a>(
    label: &'a str,
    label_width: u16,
    values: &[u64],
    style: Style,
    peak_formatter: impl Fn(u64) -> String,
    width: u16,
) -> Line<'a> {
    let label_span = Span::styled(
        format!("{label:<width$} ", width = label_width as usize),
        style,
    );
    if width <= label_width + 4 {
        return Line::from(vec![label_span, Span::raw("—")]);
    }
    let peak = values.iter().copied().max().unwrap_or(0);
    let peak_label = format!(" peak {}", peak_formatter(peak));
    let glyph_budget = (width as usize).saturating_sub(label_width as usize + 1 + peak_label.len());
    let take = glyph_budget.min(values.len());
    let visible: &[u64] = &values[values.len() - take..];
    let glyphs: String = visible.iter().map(|v| glyph_for(*v, peak)).collect();
    Line::from(vec![
        label_span,
        Span::styled(glyphs, style),
        Span::styled(peak_label, style),
    ])
}

/// Pick the glyph for `value` scaled to `peak`. `peak == 0` returns
/// the lowest glyph (avoids div-by-zero); otherwise the index is
/// `floor(value / peak * 7)`.
fn glyph_for(value: u64, peak: u64) -> char {
    if peak == 0 {
        return GLYPHS[0];
    }
    let scaled = (value as f64 / peak as f64 * 7.0).floor() as usize;
    GLYPHS[scaled.min(7)]
}

/// Standard peak formatter for flowfile-count metrics. K/M
/// abbreviations beyond 1000 / 1_000_000.
pub fn count_formatter(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{}M", n / 1_000_000)
    } else if n >= 1_000 {
        format!("{}K", n / 1_000)
    } else {
        n.to_string()
    }
}

/// Peak formatter for nanosecond task-time. Shows `Nms` for 1ms+,
/// `Nus` for 1us+, `Nns` otherwise.
pub fn task_time_formatter(ns: u64) -> String {
    if ns >= 1_000_000 {
        format!("{}ms", ns / 1_000_000)
    } else if ns >= 1_000 {
        format!("{}us", ns / 1_000)
    } else {
        format!("{ns}ns")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Color;

    #[test]
    fn glyph_for_zero_returns_lowest() {
        assert_eq!(glyph_for(0, 100), GLYPHS[0]);
    }

    #[test]
    fn glyph_for_peak_returns_highest() {
        assert_eq!(glyph_for(100, 100), GLYPHS[7]);
    }

    #[test]
    fn glyph_for_zero_peak_returns_lowest_no_panic() {
        assert_eq!(glyph_for(0, 0), GLYPHS[0]);
        assert_eq!(glyph_for(50, 0), GLYPHS[0]);
    }

    #[test]
    fn glyph_for_midpoint_returns_middle() {
        // 50/100 = 0.5, floor(0.5 * 7) = 3 → GLYPHS[3] = ▄.
        assert_eq!(glyph_for(50, 100), GLYPHS[3]);
    }

    #[test]
    fn count_formatter_humanizes() {
        assert_eq!(count_formatter(0), "0");
        assert_eq!(count_formatter(999), "999");
        assert_eq!(count_formatter(1_500), "1K");
        assert_eq!(count_formatter(2_500_000), "2M");
    }

    #[test]
    fn task_time_formatter_humanizes() {
        assert_eq!(task_time_formatter(500), "500ns");
        assert_eq!(task_time_formatter(1_500), "1us");
        assert_eq!(task_time_formatter(2_500_000), "2ms");
    }

    #[test]
    fn render_sparkline_row_truncates_from_left_when_narrow() {
        let values = (0..50u64).collect::<Vec<_>>();
        let line = render_sparkline_row(
            "in",
            5,
            &values,
            Style::default().fg(Color::Green),
            count_formatter,
            30,
        );
        assert_eq!(line.spans.len(), 3);
    }

    #[test]
    fn render_sparkline_row_below_min_width_renders_em_dash() {
        let values = vec![1u64, 2, 3];
        let line = render_sparkline_row("in", 5, &values, Style::default(), count_formatter, 6);
        assert_eq!(line.spans.len(), 2);
        assert_eq!(line.spans[1].content, "—");
    }

    #[test]
    fn render_sparkline_row_empty_values_renders_label_and_peak_zero() {
        let line = render_sparkline_row("in", 5, &[], Style::default(), count_formatter, 40);
        // Three spans: label, empty glyphs, peak suffix (peak 0).
        assert_eq!(line.spans.len(), 3);
    }
}
