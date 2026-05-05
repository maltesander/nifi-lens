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

/// Canonical "still loading" placeholder shown by every modal whose
/// data is fetched asynchronously. Single source of truth so the text
/// (and its trailing horizontal-ellipsis) stays consistent.
pub const LOADING_LABEL: &str = "loading…";

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

/// Three-state load gate shared by per-component modals (Access,
/// Identity) whose render bodies branch on a `Loading / Failed / Loaded`
/// status enum. Each modal owns its own status type; the conversion to
/// `LoadGate` happens at the call site (or via `LoadStatus::as_load_gate`).
pub enum LoadGate<'a> {
    Loading,
    Failed(&'a str),
    Loaded,
}

/// Generic owned-payload load status used by modals whose loaded data
/// lives inside the `Loaded` variant (Parameter context, Version
/// control diff). Modals where data lives in sibling fields keep their
/// own bespoke status enum and convert to `LoadGate` directly.
#[derive(Debug, Clone, Default)]
pub enum LoadStatus<T> {
    #[default]
    Loading,
    Failed(String),
    Loaded(T),
}

impl<T> LoadStatus<T> {
    /// Borrow the loaded payload, if any.
    pub fn loaded(&self) -> Option<&T> {
        match self {
            Self::Loaded(t) => Some(t),
            _ => None,
        }
    }

    /// Project to a `LoadGate` for use with `render_load_gate`.
    pub fn as_load_gate(&self) -> LoadGate<'_> {
        match self {
            Self::Loading => LoadGate::Loading,
            Self::Failed(err) => LoadGate::Failed(err),
            Self::Loaded(_) => LoadGate::Loaded,
        }
    }
}

/// Render the modal load-status placeholder for `Loading` / `Failed`,
/// or do nothing for `Loaded`. Returns `true` when a placeholder was
/// drawn — callers should short-circuit their normal render.
pub fn render_load_gate(frame: &mut Frame, area: Rect, gate: LoadGate<'_>) -> bool {
    match gate {
        LoadGate::Loading => {
            frame.render_widget(
                Paragraph::new(Span::styled(LOADING_LABEL, theme::muted())),
                area,
            );
            true
        }
        LoadGate::Failed(err) => {
            frame.render_widget(
                Paragraph::new(Span::styled(format!("failed: {err}"), theme::error())),
                area,
            );
            true
        }
        LoadGate::Loaded => false,
    }
}

/// Render a footer hint strip from a slice of verbs. Filters out verbs
/// where `show_in_hint_bar()` returns false or `hint()` is empty, then
/// formats the survivors as `[chord] hint · [chord] hint` rendered with
/// `theme::muted()`. Caller can pass `V::all()` directly — filtering is
/// internal so call sites stay trivial.
pub fn render_verb_hint_strip<V: Verb>(frame: &mut Frame, area: Rect, verbs: &[V]) {
    render_verb_hint_strip_with(frame, area, verbs, |_| true);
}

/// Variant of [`render_verb_hint_strip`] that accepts a per-verb
/// `enabled` predicate so modals can hide chords whose preconditions
/// are not met (e.g. `n`/`N` only after a search is committed). The
/// filter composes with the standard `show_in_hint_bar` / `hint()`
/// gates — a verb must pass all three to render.
pub fn render_verb_hint_strip_with<V: Verb>(
    frame: &mut Frame,
    area: Rect,
    verbs: &[V],
    enabled: impl Fn(V) -> bool,
) {
    let parts: Vec<String> = verbs
        .iter()
        .copied()
        .filter(|v| v.show_in_hint_bar() && !v.hint().is_empty() && enabled(*v))
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
    use crate::input::Chord;
    use crate::test_support::test_backend;
    use crossterm::event::KeyCode;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

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
    fn load_gate_loaded_returns_false_and_renders_nothing() {
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let mut drew = true;
        term.draw(|frame| {
            drew = render_load_gate(frame, frame.area(), LoadGate::Loaded);
        })
        .unwrap();
        assert!(!drew);
        let out = format!("{}", term.backend());
        assert!(!out.contains("loading"), "out was:\n{out}");
        assert!(!out.contains("failed"), "out was:\n{out}");
    }

    #[test]
    fn load_gate_loading_renders_placeholder_and_returns_true() {
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let mut drew = false;
        term.draw(|frame| {
            drew = render_load_gate(frame, frame.area(), LoadGate::Loading);
        })
        .unwrap();
        assert!(drew);
        let out = format!("{}", term.backend());
        assert!(out.contains("loading"), "out was:\n{out}");
    }

    #[test]
    fn load_status_default_is_loading() {
        let s: LoadStatus<i32> = LoadStatus::default();
        assert!(matches!(s, LoadStatus::Loading));
        assert!(s.loaded().is_none());
    }

    #[test]
    fn load_status_loaded_exposes_payload_and_gates_loaded() {
        let s: LoadStatus<i32> = LoadStatus::Loaded(42);
        assert_eq!(s.loaded(), Some(&42));
        assert!(matches!(s.as_load_gate(), LoadGate::Loaded));
    }

    #[test]
    fn load_status_failed_carries_message_to_gate() {
        let s: LoadStatus<i32> = LoadStatus::Failed("boom".into());
        assert!(s.loaded().is_none());
        match s.as_load_gate() {
            LoadGate::Failed(err) => assert_eq!(err, "boom"),
            _ => panic!("expected Failed"),
        }
    }

    #[test]
    fn load_gate_failed_renders_error_and_returns_true() {
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        let mut drew = false;
        term.draw(|frame| {
            drew = render_load_gate(frame, frame.area(), LoadGate::Failed("boom"));
        })
        .unwrap();
        assert!(drew);
        let out = format!("{}", term.backend());
        assert!(out.contains("failed"), "out was:\n{out}");
        assert!(out.contains("boom"), "out was:\n{out}");
    }

    #[test]
    fn render_too_small_returns_true_below_threshold() {
        // Build a backend smaller than MIN_WIDTH × MIN_HEIGHT.
        let mut term = Terminal::new(TestBackend::new(MIN_WIDTH - 1, MIN_HEIGHT)).unwrap();
        term.draw(|frame| {
            let degraded = render_too_small(frame, frame.area());
            assert!(degraded);
        })
        .unwrap();
    }

    /// Mock `Verb` impl so the hint-strip tests don't depend on real
    /// per-view enums (and so we can flip `show_in_hint_bar` / `hint`
    /// independently).
    #[derive(Clone, Copy)]
    struct MockVerb {
        chord_char: char,
        hint_text: &'static str,
        show: bool,
    }

    impl Verb for MockVerb {
        fn chord(self) -> Chord {
            Chord::simple(KeyCode::Char(self.chord_char))
        }
        fn label(self) -> &'static str {
            "mock"
        }
        fn hint(self) -> &'static str {
            self.hint_text
        }
        fn show_in_hint_bar(self) -> bool {
            self.show
        }
        fn all() -> &'static [Self] {
            &[]
        }
    }

    #[test]
    fn hint_strip_formats_visible_verbs_with_separator() {
        let verbs = vec![
            MockVerb {
                chord_char: 'a',
                hint_text: "alpha",
                show: true,
            },
            MockVerb {
                chord_char: 'b',
                hint_text: "bravo",
                show: true,
            },
        ];
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        term.draw(|frame| {
            render_verb_hint_strip(frame, frame.area(), &verbs);
        })
        .unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("[a] alpha"), "out was:\n{out}");
        assert!(out.contains("[b] bravo"), "out was:\n{out}");
        assert!(out.contains(" \u{b7} "), "out was:\n{out}");
    }

    #[test]
    fn hint_strip_filters_out_show_in_hint_bar_false() {
        let verbs = vec![
            MockVerb {
                chord_char: 'a',
                hint_text: "alpha",
                show: true,
            },
            MockVerb {
                chord_char: 'b',
                hint_text: "bravo",
                show: false,
            },
        ];
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        term.draw(|frame| {
            render_verb_hint_strip(frame, frame.area(), &verbs);
        })
        .unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("alpha"), "out was:\n{out}");
        assert!(!out.contains("bravo"), "out was:\n{out}");
    }

    #[test]
    fn hint_strip_filters_out_empty_hint() {
        let verbs = vec![
            MockVerb {
                chord_char: 'a',
                hint_text: "alpha",
                show: true,
            },
            MockVerb {
                chord_char: 'b',
                hint_text: "",
                show: true,
            },
        ];
        let mut term = Terminal::new(TestBackend::new(40, 1)).unwrap();
        term.draw(|frame| {
            render_verb_hint_strip(frame, frame.area(), &verbs);
        })
        .unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("alpha"), "out was:\n{out}");
        assert!(!out.contains("[b]"), "out was:\n{out}");
    }
}
