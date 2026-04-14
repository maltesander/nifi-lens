use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::Clear;

use crate::app::state::ContextSwitcherState;
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &ContextSwitcherState) {
    use ratatui::widgets::{Cell, Row, Table, TableState};

    let height = (state.entries.len() as u16 + 5).clamp(7, 20);
    let modal = center(area, 70, height);
    frame.render_widget(Clear, modal);

    let block = crate::widget::panel::Panel::new(" Switch Context (K) ").into_block();
    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let header = Row::new(vec![
        Cell::from(Span::styled("Name", theme::muted())),
        Cell::from(Span::styled("URL", theme::muted())),
        Cell::from(Span::styled("Version", theme::muted())),
        Cell::from(Span::styled("Active", theme::muted())),
    ]);

    let rows: Vec<Row> = state
        .entries
        .iter()
        .map(|e| {
            let name_style = if e.is_active {
                theme::accent()
            } else {
                Style::default()
            };
            let version_text = if e.connecting {
                "Connecting…".to_string()
            } else if let Some(v) = &e.version {
                v.to_string()
            } else {
                String::new()
            };
            let active_cell = if e.is_active {
                Cell::from(Span::styled("(*)", theme::accent()))
            } else {
                Cell::from("")
            };
            Row::new(vec![
                Cell::from(Span::styled(e.name.clone(), name_style)),
                Cell::from(e.url.clone()),
                Cell::from(version_text),
                active_cell,
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(20),
        Constraint::Min(0),
        Constraint::Length(12),
        Constraint::Length(8),
    ];
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut ts = TableState::default();
    ts.select(Some(state.cursor));
    frame.render_stateful_widget(table, inner, &mut ts);
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
    fn context_table_renders_column_headers() {
        let state = sample_state("dev-nifi-2-9-0", 0);
        let out = render_with(&state);
        assert!(out.contains("Name"), "expected Name header in:\n{out}");
        assert!(out.contains("URL"), "expected URL header in:\n{out}");
        assert!(
            out.contains("Version"),
            "expected Version header in:\n{out}"
        );
        assert!(out.contains("Active"), "expected Active header in:\n{out}");
    }

    #[test]
    fn marker_is_independent_of_cursor_position() {
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::style::Modifier;

        // Active is entry 1 (dev-nifi-2-9-0), cursor is on entry 0.
        let state = sample_state("dev-nifi-2-9-0", 0);

        let backend = TestBackend::new(120, 12);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &state)).unwrap();
        let buffer = term.backend().buffer();

        let area = buffer.area;
        let width = area.width as usize;
        let content = buffer.content();
        let mut row_texts: Vec<(u16, String, bool)> = Vec::new();
        for y in area.top()..area.bottom() {
            let row_start = (y as usize) * width;
            let row_slice = &content[row_start..row_start + width];
            let mut text = String::new();
            let mut reversed = false;
            for cell in row_slice {
                text.push_str(cell.symbol());
                if cell.style().add_modifier.contains(Modifier::REVERSED) {
                    reversed = true;
                }
            }
            row_texts.push((y, text, reversed));
        }

        let cursor_row = row_texts
            .iter()
            .find(|(_, _, rev)| *rev)
            .expect("cursor row not highlighted");
        assert!(
            cursor_row.1.contains("dev-nifi-2-6-0"),
            "cursor should be on first entry, got row: {}",
            cursor_row.1
        );
        assert!(
            !cursor_row.1.contains("(*)"),
            "cursor row is not the active row, should not carry (*) marker"
        );

        let active_row = row_texts
            .iter()
            .find(|(_, text, _)| text.contains("(*)"))
            .expect("active marker missing");
        assert!(
            active_row.1.contains("dev-nifi-2-9-0"),
            "active marker should be on dev-nifi-2-9-0, got row: {}",
            active_row.1
        );
    }
}
