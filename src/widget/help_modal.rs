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
    AppAction, BrowserVerb, BulletinsVerb, EventsVerb, FocusAction, HistoryAction, TracerVerb, Verb,
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

/// The verbs section for the active tab, if any. Overview has no
/// view-local verbs so returns `None`.
pub fn tab_section(active_view: ViewId) -> Option<HelpSection> {
    match active_view {
        ViewId::Overview => None,
        ViewId::Bulletins => Some(section("Bulletins", BulletinsVerb::all())),
        ViewId::Browser => Some(section("Browser", BrowserVerb::all())),
        ViewId::Events => Some(section("Events", EventsVerb::all())),
        ViewId::Tracer => Some(section("Tracer", TracerVerb::all())),
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
pub fn render(frame: &mut Frame, area: Rect, active_view: ViewId) {
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
    if let Some(section) = tab_section(active_view) {
        push_section_lines(&mut right_lines, std::slice::from_ref(&section));
    } else {
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
    fn tab_section_adapts_to_view() {
        assert!(tab_section(ViewId::Overview).is_none());
        assert_eq!(tab_section(ViewId::Bulletins).unwrap().title, "Bulletins");
        assert_eq!(tab_section(ViewId::Browser).unwrap().title, "Browser");
        assert_eq!(tab_section(ViewId::Events).unwrap().title, "Events");
        assert_eq!(tab_section(ViewId::Tracer).unwrap().title, "Tracer");
    }

    #[test]
    fn tab_section_row_counts_match_verb_counts() {
        assert_eq!(
            tab_section(ViewId::Bulletins).unwrap().rows.len(),
            BulletinsVerb::all().len()
        );
        assert_eq!(
            tab_section(ViewId::Browser).unwrap().rows.len(),
            BrowserVerb::all().len()
        );
        assert_eq!(
            tab_section(ViewId::Events).unwrap().rows.len(),
            EventsVerb::all().len()
        );
        assert_eq!(
            tab_section(ViewId::Tracer).unwrap().rows.len(),
            TracerVerb::all().len()
        );
    }
}
