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
        .title(" Switch Context (Ctrl+K) ")
        .borders(Borders::ALL);

    let lines: Vec<Line> = state
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let cursor = if i == state.cursor { "▸ " } else { "  " };
            let version = if e.connecting {
                "Connecting…".to_string()
            } else if let Some(v) = &e.version {
                format!("(NiFi {v})")
            } else {
                String::new()
            };
            let content = format!("{cursor}{} {} {}", e.name, e.url, version);
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
