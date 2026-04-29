//! Shared scaffolding for full-screen modals. Owns the minimum
//! viewport-size gate (the "terminal too small" degradation) and the
//! footer hint strip driven by `Verb::all()`. Per-modal logic stays
//! in each view; only the boilerplate lives here.

use crate::input::Verb;
use crate::theme;
use ratatui::{
    Frame,
    layout::{Alignment, Rect},
    text::{Line, Span},
    widgets::{Clear, Paragraph},
};

/// Minimum viewport width below which a full-screen modal degrades to
/// the single muted line "terminal too small". Matches the convention
/// documented in AGENTS.md ("Modal conventions").
pub const MIN_WIDTH: u16 = 60;

/// Minimum viewport height below which a full-screen modal degrades.
pub const MIN_HEIGHT: u16 = 20;

/// Render the "terminal too small" degradation IF the area is below
/// the minimum. Returns `true` when the modal degraded — callers should
/// short-circuit their normal render in that case.
pub fn render_too_small(frame: &mut Frame, area: Rect) -> bool {
    if area.width >= MIN_WIDTH && area.height >= MIN_HEIGHT {
        return false;
    }
    frame.render_widget(Clear, area);
    let line = Line::from(Span::styled("terminal too small", theme::muted()));
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
    true
}

/// Render a footer hint strip from a slice of verbs. The caller passes
/// the verbs already filtered (typically `V::all().iter().copied().filter(|v|
/// v.show_in_hint_bar())`). Output format: `[chord] hint · [chord] hint`.
pub fn render_verb_hint_strip<V: Verb>(frame: &mut Frame, area: Rect, verbs: &[V]) {
    let parts: Vec<String> = verbs
        .iter()
        .filter(|v| v.show_in_hint_bar() && !v.hint().is_empty())
        .map(|v| format!("[{}] {}", v.chord().display(), v.hint()))
        .collect();
    let text = parts.join(" · ");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme::muted()))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_backend;
    use ratatui::Terminal;

    #[test]
    fn render_too_small_returns_false_above_threshold() {
        let mut term = Terminal::new(test_backend(MIN_HEIGHT)).unwrap();
        term.draw(|frame| {
            let degraded = render_too_small(frame, frame.area());
            assert!(!degraded);
        })
        .unwrap();
    }

    #[test]
    fn render_too_small_returns_true_below_threshold() {
        // Build a backend smaller than MIN_WIDTH × MIN_HEIGHT.
        use ratatui::backend::TestBackend;
        let mut term = Terminal::new(TestBackend::new(MIN_WIDTH - 1, MIN_HEIGHT)).unwrap();
        term.draw(|frame| {
            let degraded = render_too_small(frame, frame.area());
            assert!(degraded);
        })
        .unwrap();
    }
}
