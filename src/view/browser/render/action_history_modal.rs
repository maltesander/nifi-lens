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
    render_hints(frame, rows[2]);
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
    let header = "  time              user             op              type             source";
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

    let mut lines: Vec<Line> = Vec::with_capacity(modal.actions.len() * 2);
    for (i, a) in modal.actions.iter().enumerate() {
        let inner = a.action.as_ref();
        let timestamp = a.timestamp.as_deref().unwrap_or("—");
        let user = inner
            .and_then(|x| x.user_identity.as_deref())
            .unwrap_or("—");
        let op = inner.and_then(|x| x.operation.as_deref()).unwrap_or("—");
        let stype = inner.and_then(|x| x.source_type.as_deref()).unwrap_or("—");
        let sname = inner.and_then(|x| x.source_name.as_deref()).unwrap_or("—");
        let selected = i == modal.selected;
        let row_style = if selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![Span::styled(
            format!("  {timestamp:<18} {user:<16} {op:<15} {stype:<16} {sname}"),
            row_style,
        )]));
        if modal.expanded_index == Some(i) {
            // v1: expansion shows full timestamp on its own line.
            // ActionDetailsDto is empty in the OpenAPI types; richer
            // expansion lands when upstream exposes the JSON details.
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

fn render_hints(frame: &mut Frame, area: Rect) {
    let hints = " [/ find] [n/N next] [Enter expand] [c copy] [r refresh] [Esc close]";
    frame.render_widget(Paragraph::new(Span::styled(hints, theme::muted())), area);
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

    #[test]
    fn snapshot_loaded_5_actions() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let modal = make_modal_state(5, false);
        term.draw(|f| {
            render(f, f.area(), &modal);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
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
        assert_snapshot!(format!("{}", term.backend()));
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
        assert_snapshot!(format!("{}", term.backend()));
    }

    // Verify TEST_BACKEND_WIDTH is used as the standard width.
    const _: () = assert!(TEST_BACKEND_WIDTH == 100);
}
