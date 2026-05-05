//! Help modal — two-column layout.
//!
//! Left column: general keybindings (Navigation, History, Tabs, App,
//! Cross-tab). Same for every view. Rows are paired where possible so
//! the column stays compact (e.g. `↑/↓` is one row, not two).
//!
//! Right column: verbs for the currently active tab. Generated from
//! the tab's `Verb::all()` so it stays in sync with the dispatcher.
//!
//! Chord strings always come from `Verb::chord().display()` — the
//! general section still derives its keys from the enum layer, it
//! just collapses pairs into one row for brevity.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Wrap};

use crate::app::state::ViewId;
use crate::input::{
    AppAction, BrowserVerb, BulletinsVerb, ContentModalVerb, EventsVerb, FocusAction,
    HistoryAction, TracerVerb, Verb,
};
use crate::theme;

/// One logical section of the help modal. Each section has a title
/// plus a list of `(chord, label)` rows.
pub struct HelpSection {
    pub title: &'static str,
    pub rows: Vec<(String, &'static str)>,
}

/// Mutable state for the Help modal — currently just the vertical
/// scroll offset shared by both columns. Carried inside `Modal::Help`
/// so the dispatcher can mutate it on `↑/↓/PgUp/PgDn/Home/End`.
#[derive(Debug, Default, Clone, Copy)]
pub struct HelpState {
    pub scroll: u16,
}

impl HelpState {
    pub fn new() -> Self {
        Self { scroll: 0 }
    }

    /// Move scroll by `delta` rows. Negative values scroll up. The
    /// caller passes a `max_scroll` ceiling computed from the most
    /// recently rendered area; `scroll` saturates at zero.
    pub fn scroll_by(&mut self, delta: i32, max_scroll: u16) {
        let cur = self.scroll as i32;
        let next = (cur + delta).clamp(0, max_scroll as i32);
        self.scroll = next as u16;
    }

    pub fn scroll_top(&mut self) {
        self.scroll = 0;
    }

    pub fn scroll_bottom(&mut self, max_scroll: u16) {
        self.scroll = max_scroll;
    }
}

/// General (tab-independent) sections. Paired into compact rows.
pub fn general_sections() -> Vec<HelpSection> {
    vec![
        HelpSection {
            title: "Navigation",
            rows: vec![
                (pair(FocusAction::Up, FocusAction::Down), "move up / down"),
                (
                    pair(FocusAction::Left, FocusAction::Right),
                    "peer left / right",
                ),
                (
                    pair(FocusAction::PageUp, FocusAction::PageDown),
                    "page up / down",
                ),
                (
                    pair(FocusAction::First, FocusAction::Last),
                    "goto first / last",
                ),
                (FocusAction::Descend.chord().display(), "drill / activate"),
                (FocusAction::Ascend.chord().display(), "leave pane / cancel"),
                (
                    pair(FocusAction::NextPane, FocusAction::PrevPane),
                    "focus next / prev pane",
                ),
            ],
        },
        HelpSection {
            title: "History",
            rows: vec![
                (HistoryAction::Back.chord().display(), "back"),
                (HistoryAction::Forward.chord().display(), "forward"),
            ],
        },
        HelpSection {
            title: "Tabs",
            rows: vec![("F1..F5".to_string(), "goto tab 1..5")],
        },
        HelpSection {
            title: "App",
            rows: vec![
                (
                    format!("{} / Ctrl+c", AppAction::Quit.chord().display()),
                    "quit",
                ),
                (AppAction::Help.chord().display(), "this help"),
                (
                    AppAction::ContextSwitcher.chord().display(),
                    "switch context",
                ),
                (AppAction::FuzzyFind.chord().display(), "fuzzy find"),
            ],
        },
        HelpSection {
            title: "Cross-tab",
            rows: vec![(AppAction::Goto.chord().display(), "goto related tab")],
        },
    ]
}

/// The verbs sections for the active tab.
///
/// Returns an empty `Vec` for Overview (no view-local verbs). For
/// the Tracer tab, when `content_modal_open` is `true`, an additional
/// "Content viewer" section is appended after the base "Tracer" section
/// so both sets are visible while the modal is up.
pub fn tab_sections(active_view: ViewId, content_modal_open: bool) -> Vec<HelpSection> {
    match active_view {
        ViewId::Overview => vec![],
        ViewId::Bulletins => vec![section("Bulletins", BulletinsVerb::all())],
        ViewId::Browser => vec![section("Browser", BrowserVerb::all())],
        ViewId::Events => vec![section("Events", EventsVerb::all())],
        ViewId::Tracer => {
            let mut out = vec![section("Tracer", TracerVerb::all())];
            if content_modal_open {
                out.push(section("Content viewer", ContentModalVerb::all()));
            }
            out
        }
    }
}

fn section<V: Verb>(title: &'static str, variants: &[V]) -> HelpSection {
    let rows = variants
        .iter()
        .map(|v| (v.chord().display(), v.label()))
        .collect();
    HelpSection { title, rows }
}

/// Pair two verbs' chords into `"a / b"`, e.g. `"↑ / ↓"`.
fn pair<A: Verb, B: Verb>(a: A, b: B) -> String {
    format!("{} / {}", a.chord().display(), b.chord().display())
}

/// Render a single column of sections into `lines`.
fn push_section_lines(lines: &mut Vec<Line<'static>>, sections: &[HelpSection]) {
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            lines.push(Line::from(""));
        }
        lines.push(Line::from(Span::styled(
            s.title.to_string(),
            theme::accent().add_modifier(Modifier::BOLD),
        )));
        for (chord, label) in &s.rows {
            lines.push(Line::from(vec![
                Span::styled(format!("  {chord:16}"), theme::accent()),
                Span::styled(" ".to_string(), Style::default()),
                Span::styled((*label).to_string(), theme::muted()),
            ]));
        }
    }
}

/// Maximum modal width — wider than this wastes horizontal space on
/// chord/label columns that don't need it.
const MAX_WIDTH: u16 = 80;
/// Minimum interior height — below this the help modal is unhelpful;
/// the parent caller can choose to skip rendering, but we still degrade
/// gracefully via `render_too_small`.
const MIN_INNER_HEIGHT: u16 = 6;

/// Total content rows needed to render `sections` end-to-end at the
/// 40-col column width (matches the Layout::Percentage(50) split when
/// modal width is 80). Includes blank-line separators between sections.
fn count_section_rows(sections: &[HelpSection]) -> u16 {
    let mut rows: u16 = 0;
    for (i, s) in sections.iter().enumerate() {
        if i > 0 {
            rows = rows.saturating_add(1); // blank separator
        }
        rows = rows.saturating_add(1); // title row
        rows = rows.saturating_add(s.rows.len() as u16);
    }
    rows
}

/// Render the help modal into `area` for the given active view.
///
/// The modal grows to fit content up to `MAX_WIDTH` × `area.height-4`,
/// then scrolls. Two columns split 50/50; the scroll offset on `state`
/// is shared between both columns. The bottom row of the modal carries
/// a hint strip ("↑↓ scroll · Esc close") only when content overflows.
///
/// `content_modal_open` controls whether the "Content viewer" section
/// is appended when the Tracer tab is active.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    active_view: ViewId,
    content_modal_open: bool,
    state: &mut HelpState,
) {
    // Build content first so we can size the modal to fit.
    let general = general_sections();
    let tab_secs = tab_sections(active_view, content_modal_open);

    let mut left_lines: Vec<Line<'static>> = Vec::new();
    push_section_lines(&mut left_lines, &general);

    let mut right_lines: Vec<Line<'static>> = Vec::new();
    if tab_secs.is_empty() {
        right_lines.push(Line::from(Span::styled(
            "Overview".to_string(),
            theme::accent().add_modifier(Modifier::BOLD),
        )));
        right_lines.push(Line::from(Span::styled(
            "  (no view-local keybindings)".to_string(),
            theme::muted(),
        )));
    } else {
        push_section_lines(&mut right_lines, &tab_secs);
    }

    let content_rows = count_section_rows(&general).max(if tab_secs.is_empty() {
        2
    } else {
        count_section_rows(&tab_secs)
    });

    // Modal sizing: cap by terminal area, leaving a 2-row gutter top/bottom.
    let max_modal_h = area.height.saturating_sub(4).max(MIN_INNER_HEIGHT + 2);
    let max_modal_w = area.width.saturating_sub(4).min(MAX_WIDTH);
    // +3 = top border + bottom border + footer hint row when overflowing.
    let needed_modal_h = content_rows.saturating_add(3).min(max_modal_h);
    let modal_h = needed_modal_h.max(MIN_INNER_HEIGHT + 2).min(max_modal_h);
    let modal = crate::layout::center_absolute(area, max_modal_w, modal_h);
    frame.render_widget(Clear, modal);

    let block = crate::widget::panel::Panel::new(" Help ")
        .border_style(theme::accent())
        .into_block();
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    // Carve out a 1-row footer for the scroll hint when the content
    // doesn't fit. Otherwise the body uses the full inner area.
    let inner_height = inner.height;
    let max_scroll = content_rows.saturating_sub(inner_height.saturating_sub(1));
    let overflowing = max_scroll > 0;
    state.scroll = state.scroll.min(max_scroll);

    let body = if overflowing {
        let parts = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Fill(1), Constraint::Length(1)])
            .split(inner);
        let footer_text = format!(
            " ↑↓ scroll · PgUp/PgDn page · {}/{} ",
            state.scroll.saturating_add(1),
            max_scroll.saturating_add(1),
        );
        let footer = Paragraph::new(Line::from(Span::styled(footer_text, theme::muted())));
        frame.render_widget(footer, parts[1]);
        parts[0]
    } else {
        inner
    };

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(body);

    let scroll_pair = (state.scroll, 0);
    frame.render_widget(
        Paragraph::new(left_lines)
            .scroll(scroll_pair)
            .wrap(Wrap { trim: false }),
        cols[0],
    );
    frame.render_widget(
        Paragraph::new(right_lines)
            .scroll(scroll_pair)
            .wrap(Wrap { trim: false }),
        cols[1],
    );
}

/// Coarse ceiling for the dispatcher to clamp scroll against without
/// needing the rendered area. Every frame, `render` saturates `scroll`
/// against the *true* max for the current terminal size, so the user
/// never sees a value above that — but the in-state value can briefly
/// exceed it between key press and next render. This ceiling keeps the
/// in-state value bounded so Up/PgUp aren't slow to climb back.
pub fn estimate_max_scroll(general: &[HelpSection], tab_secs: &[HelpSection]) -> u16 {
    let general_rows = count_section_rows(general);
    let tab_rows = if tab_secs.is_empty() {
        2
    } else {
        count_section_rows(tab_secs)
    };
    general_rows.max(tab_rows)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_sections_are_stable_across_views() {
        // General column doesn't depend on active view.
        let a = general_sections();
        let b = general_sections();
        assert_eq!(a.len(), b.len());
        for (sa, sb) in a.iter().zip(b.iter()) {
            assert_eq!(sa.title, sb.title);
            assert_eq!(sa.rows.len(), sb.rows.len());
        }
    }

    #[test]
    fn general_sections_have_five_titles() {
        let s = general_sections();
        let titles: Vec<_> = s.iter().map(|s| s.title).collect();
        assert_eq!(
            titles,
            vec!["Navigation", "History", "Tabs", "App", "Cross-tab"]
        );
    }

    #[test]
    fn navigation_section_pairs_up_arrows() {
        let s = &general_sections()[0];
        assert_eq!(s.title, "Navigation");
        // Up/Down paired, Left/Right paired, PgUp/PgDn paired,
        // First/Last paired, Descend, Ascend, NextPane/PrevPane paired = 7 rows.
        assert_eq!(s.rows.len(), 7);
        assert!(s.rows[0].0.contains('/'));
        assert!(s.rows[1].0.contains('/'));
    }

    #[test]
    fn tab_sections_adapts_to_view() {
        assert!(tab_sections(ViewId::Overview, false).is_empty());
        assert_eq!(tab_sections(ViewId::Bulletins, false)[0].title, "Bulletins");
        assert_eq!(tab_sections(ViewId::Browser, false)[0].title, "Browser");
        assert_eq!(tab_sections(ViewId::Events, false)[0].title, "Events");
        assert_eq!(tab_sections(ViewId::Tracer, false)[0].title, "Tracer");
    }

    #[test]
    fn content_modal_open_adds_content_viewer_section_for_tracer() {
        let secs = tab_sections(ViewId::Tracer, true);
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, "Tracer");
        assert_eq!(secs[1].title, "Content viewer");
        assert_eq!(secs[1].rows.len(), ContentModalVerb::all().len());
    }

    #[test]
    fn content_modal_open_does_not_add_section_for_other_views() {
        // content_modal_open=true is ignored when not on Tracer tab.
        assert!(tab_sections(ViewId::Overview, true).is_empty());
        assert_eq!(tab_sections(ViewId::Bulletins, true).len(), 1);
        assert_eq!(tab_sections(ViewId::Browser, true).len(), 1);
        assert_eq!(tab_sections(ViewId::Events, true).len(), 1);
    }

    #[test]
    fn help_state_scroll_by_clamps_to_bounds() {
        let mut hs = HelpState::new();
        hs.scroll_by(-1, 5);
        assert_eq!(hs.scroll, 0, "scrolling up from 0 must saturate at 0");
        hs.scroll_by(3, 5);
        assert_eq!(hs.scroll, 3);
        hs.scroll_by(10, 5);
        assert_eq!(hs.scroll, 5, "scrolling down past max must saturate at max");
        hs.scroll_top();
        assert_eq!(hs.scroll, 0);
        hs.scroll_bottom(5);
        assert_eq!(hs.scroll, 5);
    }

    #[test]
    fn count_section_rows_matches_layout() {
        let sections = vec![HelpSection {
            title: "X",
            rows: vec![("a".into(), "1"), ("b".into(), "2"), ("c".into(), "3")],
        }];
        // 1 title + 3 row entries.
        assert_eq!(count_section_rows(&sections), 4);

        let two = vec![
            HelpSection {
                title: "X",
                rows: vec![("a".into(), "1")],
            },
            HelpSection {
                title: "Y",
                rows: vec![("b".into(), "2")],
            },
        ];
        // 1 title + 1 row + 1 separator + 1 title + 1 row.
        assert_eq!(count_section_rows(&two), 5);
    }

    #[test]
    fn render_writes_back_clamped_scroll_when_scroll_exceeds_max() {
        // Tiny terminal forces the modal to overflow even on Overview.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 12);
        let mut term = Terminal::new(backend).unwrap();
        let mut hs = HelpState { scroll: 999 };
        term.draw(|f| render(f, f.area(), ViewId::Browser, false, &mut hs))
            .unwrap();
        // After render, scroll is clamped to whatever max fits the
        // 12-row terminal — definitely below 999.
        assert!(
            hs.scroll < 999,
            "render must clamp scroll, got {}",
            hs.scroll
        );
    }

    #[test]
    fn render_keeps_scroll_zero_when_content_fits() {
        // A tall terminal accommodates Overview content with no scroll.
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(80, 60);
        let mut term = Terminal::new(backend).unwrap();
        let mut hs = HelpState { scroll: 0 };
        term.draw(|f| render(f, f.area(), ViewId::Overview, false, &mut hs))
            .unwrap();
        assert_eq!(hs.scroll, 0);
    }

    #[test]
    fn help_modal_still_lists_severity_filters_for_bulletins() {
        use crate::input::{BulletinsVerb, Verb};
        let sec = section("Bulletins", BulletinsVerb::all());
        let keys: Vec<String> = sec.rows.iter().map(|(k, _)| k.clone()).collect();
        assert!(keys.iter().any(|k| k == "1"));
        assert!(keys.iter().any(|k| k == "2"));
        assert!(keys.iter().any(|k| k == "3"));
    }

    #[test]
    fn tab_section_row_counts_match_verb_counts() {
        assert_eq!(
            tab_sections(ViewId::Bulletins, false)[0].rows.len(),
            BulletinsVerb::all().len()
        );
        assert_eq!(
            tab_sections(ViewId::Browser, false)[0].rows.len(),
            BrowserVerb::all().len()
        );
        assert_eq!(
            tab_sections(ViewId::Events, false)[0].rows.len(),
            EventsVerb::all().len()
        );
        assert_eq!(
            tab_sections(ViewId::Tracer, false)[0].rows.len(),
            TracerVerb::all().len()
        );
    }
}
