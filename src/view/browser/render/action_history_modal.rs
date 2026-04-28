//! Full-screen overlay for the Browser action-history modal.
//!
//! Layout: Title bar with component label + (showing N of M) progress.
//! Header row, then a scrollable rows list, then a hint strip.
//! Below MIN_WIDTH × MIN_HEIGHT degrades to a centered "terminal too
//! small" line, mirroring `version_control_modal`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::theme;
use crate::timestamp::{format_age_secs, parse_nifi_timestamp};
use crate::view::browser::state::action_history_modal::ActionHistoryModalState;
use crate::widget::panel::Panel;

const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 20;

pub fn render(frame: &mut Frame, area: Rect, modal: &ActionHistoryModalState) {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let line = Line::from(Span::styled("terminal too small", theme::muted()));
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }

    frame.render_widget(Clear, area);

    let outer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Action history", theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(modal.component_label.as_str(), theme::accent()),
        Span::raw(" "),
        progress_chip(modal),
        Span::raw(" "),
    ]);
    let outer = Panel::new(outer_title).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header row
            Constraint::Min(1),    // body
            Constraint::Length(1), // hint strip
        ])
        .split(inner);

    render_header(frame, rows[0]);
    render_body(frame, rows[1], modal);
    render_footer_status(frame, rows[2], modal);
}

fn progress_chip(modal: &ActionHistoryModalState) -> Span<'_> {
    if modal.error.is_some() {
        return Span::styled("(error)", theme::error());
    }
    let chip = match modal.total {
        Some(t) => format!("(showing {} of {})", modal.actions.len(), t),
        None if modal.loading => "(loading…)".to_string(),
        None => String::new(),
    };
    Span::styled(chip, theme::muted())
}

fn render_header(frame: &mut Frame, area: Rect) {
    let header = "  age      user             op              type             source";
    let line = Line::from(Span::styled(header, theme::muted()));
    frame.render_widget(Paragraph::new(line), area);
}

fn render_body(frame: &mut Frame, area: Rect, modal: &ActionHistoryModalState) {
    if let Some(err) = &modal.error {
        let msg = format!(" error: {err}");
        frame.render_widget(Paragraph::new(Span::styled(msg, theme::error())), area);
        return;
    }
    if modal.actions.is_empty() {
        let placeholder = if modal.loading {
            " loading…"
        } else {
            " no actions recorded for this component"
        };
        frame.render_widget(
            Paragraph::new(Span::styled(placeholder, theme::muted())),
            area,
        );
        return;
    }

    let match_line_idx: Option<usize> = modal
        .search
        .as_ref()
        .filter(|s| s.committed && !s.matches.is_empty())
        .and_then(|s| s.current.map(|c| s.matches[c].line_idx));

    let now = time::OffsetDateTime::now_utc();
    let mut lines: Vec<Line> = Vec::with_capacity(modal.actions.len() * 2);
    for (i, a) in modal.actions.iter().enumerate() {
        let inner = a.action.as_ref();
        let timestamp = a.timestamp.as_deref().unwrap_or("—");
        let age = render_age(timestamp, now);
        let user = inner
            .and_then(|x| x.user_identity.as_deref())
            .unwrap_or("—");
        let op = inner.and_then(|x| x.operation.as_deref()).unwrap_or("—");
        let stype = inner.and_then(|x| x.source_type.as_deref()).unwrap_or("—");
        let sname = inner.and_then(|x| x.source_name.as_deref()).unwrap_or("—");
        let selected = i == modal.selected;
        let is_current_match = Some(i) == match_line_idx;
        let row_style = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else if is_current_match {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  {age:<8} {user:<16} {op:<15} {stype:<16} {sname}"),
            row_style,
        )]));
        if modal.expanded_index == Some(i) {
            // v1: expansion shows the absolute timestamp on its own
            // line. ActionDetailsDto is empty in the OpenAPI types;
            // richer expansion lands when upstream exposes the JSON
            // details.
            lines.push(Line::from(vec![
                Span::raw("      "),
                Span::styled("at ", theme::muted()),
                Span::raw(timestamp.to_string()),
            ]));
        }
    }

    // Render with the modal's scroll offset.
    let scroll_offset = u16::try_from(modal.scroll.offset).unwrap_or(u16::MAX);
    frame.render_widget(Paragraph::new(lines).scroll((scroll_offset, 0)), area);
}

/// Bottom strip: when the user is typing a search query, show the
/// `/ {query}_` prompt instead of the hint list.
fn render_footer_status(frame: &mut Frame, area: Rect, modal: &ActionHistoryModalState) {
    if let Some(s) = modal.search.as_ref()
        && s.input_active
    {
        let line = Line::from(vec![
            Span::styled("/ ".to_string(), theme::accent()),
            Span::raw(s.query.clone()),
            Span::styled("_".to_string(), theme::search_cursor()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }
    render_footer_hint(frame, area);
}

/// Render an action's timestamp as a relative age (e.g. `12s`, `5m`,
/// `3h`) using the project's `format_age_secs` formatter. Falls back to
/// an em-dash when the timestamp is missing or unparseable. NiFi
/// timestamps come in two shapes (RFC-3339 / `MM/DD/YYYY HH:MM:SS UTC`);
/// `parse_nifi_timestamp` handles both.
fn render_age(timestamp: &str, now: time::OffsetDateTime) -> String {
    let Some(dt) = parse_nifi_timestamp(timestamp) else {
        return "\u{2014}".to_string();
    };
    let secs = (now - dt).whole_seconds().max(0) as u64;
    format_age_secs(secs)
}

fn render_footer_hint(frame: &mut Frame, area: Rect) {
    use crate::input::ActionHistoryModalVerb;
    use crate::input::Verb;

    let parts: Vec<String> = ActionHistoryModalVerb::all()
        .iter()
        .copied()
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
    use crate::test_support::{TEST_BACKEND_WIDTH, test_backend};
    use insta::assert_snapshot;
    use nifi_rust_client::dynamic::types::{ActionDto, ActionEntity};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn make_modal_state(actions: usize, loading: bool) -> ActionHistoryModalState {
        let mut s = ActionHistoryModalState::pending("proc-1".into(), "FetchKafka".into());
        for i in 0..actions {
            let mut inner = ActionDto::default();
            inner.id = Some(i as i32);
            inner.operation = Some("Configure".into());
            inner.source_id = Some("proc-1".into());
            inner.source_name = Some("FetchKafka".into());
            inner.source_type = Some("Processor".into());
            inner.user_identity = Some(format!("user{i}"));
            inner.timestamp = Some("2026-04-27T10:00:00Z".into());
            let mut a = ActionEntity::default();
            a.id = Some(i as i32);
            a.source_id = Some("proc-1".into());
            a.timestamp = Some("2026-04-27T10:00:00Z".into());
            a.action = Some(inner);
            s.actions.push(a);
        }
        s.total = Some(actions as u32);
        s.loading = loading;
        s
    }

    #[test]
    fn snapshot_loading() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let modal = ActionHistoryModalState::pending("proc-1".into(), "FetchKafka".into());
        // pending() leaves loading=true.
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    /// Insta filter that redacts the age column to a stable placeholder
    /// so snapshots are deterministic across runs. Action timestamps are
    /// fixed in test fixtures, but the rendered age depends on
    /// `OffsetDateTime::now_utc()` and would otherwise drift over time.
    /// The format `format_age_secs` produces is `<digits><s|m|h>` (e.g.
    /// `12s`, `3h`); the regex matches a digit run followed by exactly
    /// one of those unit characters, anchored after the leading row
    /// padding.
    fn age_filter() -> Vec<(&'static str, &'static str)> {
        vec![(r"  \d+[smh] ", "  <AGE> ")]
    }

    #[test]
    fn snapshot_loaded_5_actions() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let modal = make_modal_state(5, false);
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        insta::with_settings!({ filters => age_filter() }, {
            assert_snapshot!(format!("{}", term.backend()));
        });
    }

    #[test]
    fn snapshot_expanded_row() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut modal = make_modal_state(5, false);
        modal.expanded_index = Some(2);
        modal.selected = 2;
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        insta::with_settings!({ filters => age_filter() }, {
            assert_snapshot!(format!("{}", term.backend()));
        });
    }

    #[test]
    fn snapshot_below_min_size() {
        let backend = TestBackend::new(40, 10);
        let mut term = Terminal::new(backend).unwrap();
        let modal = make_modal_state(5, false);
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        // No age column rendered in the degraded path; no filter needed.
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_empty() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut modal = make_modal_state(0, false);
        modal.total = Some(0);
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        // Empty body — no rows mean no age column to redact.
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_search_input_active() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut modal = make_modal_state(5, false);
        modal.search = Some(crate::widget::search::SearchState {
            input_active: true,
            query: "user2".into(),
            ..Default::default()
        });
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        insta::with_settings!({ filters => age_filter() }, {
            assert_snapshot!(format!("{}", term.backend()));
        });
    }

    #[test]
    fn snapshot_search_committed_with_match() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut modal = make_modal_state(5, false);
        let body = modal.searchable_body();
        let matches = crate::widget::search::compute_matches(&body, "user3");
        let s = crate::widget::search::SearchState {
            query: "user3".into(),
            input_active: false,
            committed: true,
            current: if matches.is_empty() { None } else { Some(0) },
            matches,
        };
        modal.search = Some(s);
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        insta::with_settings!({ filters => age_filter() }, {
            assert_snapshot!(format!("{}", term.backend()));
        });
    }

    #[test]
    fn render_age_returns_em_dash_for_unparseable() {
        let now = time::OffsetDateTime::now_utc();
        assert_eq!(render_age("not a timestamp", now), "\u{2014}");
        assert_eq!(render_age("—", now), "\u{2014}");
    }

    #[test]
    fn render_age_handles_rfc3339_and_nifi_format() {
        let now = time::macros::datetime!(2026-04-28 11:00:00 UTC);
        // 5 minutes ago in RFC-3339.
        assert_eq!(render_age("2026-04-28T10:55:00Z", now), "5m");
        // Same instant in NiFi human format.
        assert_eq!(render_age("04/28/2026 10:55:00 UTC", now), "5m");
        // 30 seconds ago.
        assert_eq!(render_age("2026-04-28T10:59:30Z", now), "30s");
        // 2 hours ago.
        assert_eq!(render_age("2026-04-28T09:00:00Z", now), "2h");
        // Future timestamp clamps to 0s (no negative ages).
        assert_eq!(render_age("2026-04-28T12:00:00Z", now), "0s");
    }

    // Verify TEST_BACKEND_WIDTH is used as the standard width.
    const _: () = assert!(TEST_BACKEND_WIDTH == 100);
}
