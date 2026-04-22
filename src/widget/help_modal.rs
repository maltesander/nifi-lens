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
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

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

/// Render the help modal into `area` for the given active view.
///
/// Modal is centered, `80` cols × `27` rows, two columns split
/// 50/50. Left is general, right is tab-specific. The extra row
/// beyond the raw section count accommodates the `focus next / prev
/// pane` label wrapping at the 40-col column width.
///
/// `content_modal_open` controls whether the "Content viewer" section
/// is appended when the Tracer tab is active.
pub fn render(frame: &mut Frame, area: Rect, active_view: ViewId, content_modal_open: bool) {
    let modal = center(area, 80, 27);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(theme::accent());
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(inner);

    // Left: general.
    let mut left_lines: Vec<Line<'static>> = Vec::new();
    push_section_lines(&mut left_lines, &general_sections());
    frame.render_widget(
        Paragraph::new(left_lines).wrap(Wrap { trim: false }),
        cols[0],
    );

    // Right: tab-specific.
    let mut right_lines: Vec<Line<'static>> = Vec::new();
    let tab_secs = tab_sections(active_view, content_modal_open);
    if tab_secs.is_empty() {
        // Overview — no view-local verbs. Render a short note so the
        // column isn't empty.
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
    frame.render_widget(
        Paragraph::new(right_lines).wrap(Wrap { trim: false }),
        cols[1],
    );
}

fn center(area: Rect, width: u16, height: u16) -> Rect {
    // Clamp to area so we don't overflow on narrow terminals.
    let width = width.min(area.width);
    let height = height.min(area.height);
    let x = area.x + (area.width.saturating_sub(width)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width,
        height,
    }
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
