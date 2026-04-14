//! Help modal — generated from Verb::all() for every action enum.
//!
//! Lives in widget/ because it's a pure render helper. All content
//! comes from the Verb trait impls — adding a new keybinding
//! automatically shows up here without any change to this file.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};

use crate::app::state::ViewId;
use crate::input::{
    AppAction, BrowserVerb, BulletinsVerb, EventsVerb, FocusAction, GoTarget, HistoryAction,
    TabAction, TracerVerb, Verb,
};
use crate::theme;

/// One logical section of the help modal. Each section has a title
/// plus a list of `(chord, label)` rows.
pub struct HelpSection {
    pub title: &'static str,
    pub rows: Vec<(String, &'static str)>,
}

/// Build the full list of help sections for the given active view.
pub fn build_help_sections(active_view: ViewId) -> Vec<HelpSection> {
    let mut out = vec![
        section("Navigation", FocusAction::all()),
        section("History", HistoryAction::all()),
        section("Tabs", TabAction::all()),
        section("App", AppAction::all()),
        section("Cross-tab", GoTarget::all()),
    ];

    match active_view {
        ViewId::Overview => {}
        ViewId::Bulletins => out.push(section("Bulletins", BulletinsVerb::all())),
        ViewId::Browser => out.push(section("Browser", BrowserVerb::all())),
        ViewId::Events => out.push(section("Events", EventsVerb::all())),
        ViewId::Tracer => out.push(section("Tracer", TracerVerb::all())),
    }

    out
}

fn section<V: Verb>(title: &'static str, variants: &[V]) -> HelpSection {
    let rows = variants
        .iter()
        .map(|v| (v.chord().display(), v.label()))
        .collect();
    HelpSection { title, rows }
}

/// Render the help modal into the given area for the given active
/// view. Each section is rendered as a header row plus one row per
/// chord.
pub fn render(frame: &mut Frame, area: Rect, active_view: ViewId) {
    let sections = build_help_sections(active_view);
    let mut lines: Vec<Line<'static>> = Vec::new();

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
                Span::styled(format!("  {chord:12}"), theme::accent()),
                Span::styled(" ".to_string(), Style::default()),
                Span::styled(label.to_string(), theme::muted()),
            ]));
        }
    }

    let modal = center(area, 70, 34);
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Help ")
        .border_style(theme::accent());
    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false });
    frame.render_widget(para, modal);
}

fn center(area: Rect, pct_x: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_modal_lists_every_verb_once() {
        // For the Tracer view, count the total rows across all sections
        // and assert it matches the sum of variant counts.
        let sections = build_help_sections(ViewId::Tracer);
        let expected = FocusAction::all().len()
            + HistoryAction::all().len()
            + TabAction::all().len()
            + AppAction::all().len()
            + GoTarget::all().len()
            + TracerVerb::all().len();
        let total: usize = sections.iter().map(|s| s.rows.len()).sum();
        assert_eq!(total, expected);
    }

    #[test]
    fn help_modal_adapts_to_active_view() {
        let bulletins = build_help_sections(ViewId::Bulletins);
        let browser = build_help_sections(ViewId::Browser);
        // Both should have 6 sections (Navigation, History, Tabs, App, Cross-tab, <view>).
        assert_eq!(bulletins.len(), 6);
        assert_eq!(browser.len(), 6);
        // The last section title should match the active view.
        assert_eq!(bulletins.last().unwrap().title, "Bulletins");
        assert_eq!(browser.last().unwrap().title, "Browser");
    }

    #[test]
    fn help_modal_overview_has_no_view_specific_section() {
        let overview = build_help_sections(ViewId::Overview);
        // Overview has no view-local verbs, so only the 5 universal sections.
        assert_eq!(overview.len(), 5);
    }
}
