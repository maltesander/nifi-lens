use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::state::ContextSwitcherState;
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &ContextSwitcherState) {
    let height = (state.entries.len() as u16 + 4).clamp(6, 20);
    let modal = center(area, 70, height);
    frame.render_widget(Clear, modal);
    let block = Block::default()
        .title(" Switch Context (K) ")
        .borders(Borders::ALL);

    let lines: Vec<Line> = state
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let cursor = if i == state.cursor { "▸ " } else { "  " };
            let active_marker = if e.is_active { " (*)" } else { "" };
            let version = if e.connecting {
                "Connecting…".to_string()
            } else if let Some(v) = &e.version {
                format!("(NiFi {v})")
            } else {
                String::new()
            };
            let content = format!("{cursor}{} {} {}{active_marker}", e.name, e.url, version);
            let mut span = Span::raw(content);
            if e.is_active {
                span = span.style(theme::accent());
            }
            if i == state.cursor {
                span = span.style(theme::cursor_row());
            }
            Line::from(span)
        })
        .collect();

    let p = Paragraph::new(lines)
        .alignment(Alignment::Left)
        .block(block);
    frame.render_widget(p, modal);
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
    use crate::app::state::ContextEntry;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use semver::Version;

    fn render_with(state: &ContextSwitcherState) -> String {
        let backend = TestBackend::new(120, 10);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), state)).unwrap();
        format!("{}", term.backend())
    }

    fn sample_state(active: &str, cursor: usize) -> ContextSwitcherState {
        let names = ["dev-nifi-2-6-0", "dev-nifi-2-9-0", "staging"];
        let entries = names
            .iter()
            .map(|n| ContextEntry {
                name: (*n).to_string(),
                url: format!("http://localhost/{n}"),
                is_active: *n == active,
                version: if *n == active {
                    Some(Version::new(2, 9, 0))
                } else {
                    None
                },
                connecting: false,
            })
            .collect::<Vec<_>>();
        ContextSwitcherState { entries, cursor }
    }

    #[test]
    fn active_entry_shows_star_marker() {
        let state = sample_state("dev-nifi-2-9-0", 0);
        let out = render_with(&state);
        // Only the active entry gets the marker.
        assert!(
            out.contains("dev-nifi-2-9-0"),
            "expected active name in output"
        );
        assert!(
            out.contains("(*)"),
            "expected (*) marker on the active entry, got:\n{out}"
        );
        // A quick sanity check: the marker appears exactly once.
        assert_eq!(
            out.matches("(*)").count(),
            1,
            "expected marker on exactly one entry, got:\n{out}"
        );
    }

    #[test]
    fn marker_is_independent_of_cursor_position() {
        // Active is entry 1 (dev-nifi-2-9-0), cursor is on entry 0.
        let state = sample_state("dev-nifi-2-9-0", 0);
        let out = render_with(&state);
        // Cursor chevron is on the first row, marker is on the active row.
        let lines: Vec<&str> = out.lines().collect();
        let cursor_line = lines
            .iter()
            .find(|l| l.contains("▸"))
            .expect("cursor chevron missing");
        assert!(
            cursor_line.contains("dev-nifi-2-6-0"),
            "cursor should be on first entry"
        );
        assert!(
            !cursor_line.contains("(*)"),
            "cursor row is not the active row, should not carry marker"
        );
        let active_line = lines
            .iter()
            .find(|l| l.contains("(*)"))
            .expect("active marker missing");
        assert!(
            active_line.contains("dev-nifi-2-9-0"),
            "active marker should be on dev-nifi-2-9-0"
        );
    }
}
