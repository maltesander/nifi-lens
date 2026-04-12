//! Save-to-file modal for Tracer's content pane.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

/// State for the save-to-file modal, holding the event id, content side,
/// and the user-editable file path.
#[derive(Debug, Clone)]
pub struct SaveEventContentState {
    pub event_id: i64,
    pub side: crate::client::ContentSide,
    pub path: String,
}

impl SaveEventContentState {
    pub fn new(event_id: i64, side: crate::client::ContentSide) -> Self {
        let side_str = side.as_str();
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        Self {
            event_id,
            side,
            path: format!("{home}/nifilens-content-{event_id}-{side_str}.bin"),
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.path.push(ch);
    }

    pub fn backspace(&mut self) {
        self.path.pop();
    }

    pub fn clear(&mut self) {
        self.path.clear();
    }
}

/// Renders the save modal centered on screen.
pub fn render(frame: &mut Frame, area: Rect, state: &SaveEventContentState) {
    let modal = center(area, 70, 7);
    frame.render_widget(Clear, modal);

    let block = Block::default()
        .title(" Save content ")
        .borders(Borders::ALL)
        .border_style(Style::default().fg(Color::Cyan));

    let inner = block.inner(modal);
    frame.render_widget(block, modal);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // label
            Constraint::Length(1), // path input
            Constraint::Length(1), // spacer
            Constraint::Length(1), // footer
        ])
        .split(inner);

    let label = Paragraph::new(Line::from(vec![Span::styled(
        "Path: ",
        Style::default().fg(Color::Gray),
    )]));
    frame.render_widget(label, chunks[0]);

    // Render the editable path with a cursor indicator.
    let path_line = Line::from(vec![
        Span::raw(&state.path),
        Span::styled("█", Style::default().fg(Color::Cyan)),
    ]);
    let path_para = Paragraph::new(path_line);
    frame.render_widget(path_para, chunks[1]);

    let footer = Paragraph::new(Line::from(vec![
        Span::styled("Enter", Style::default().fg(Color::Green)),
        Span::raw(" save  "),
        Span::styled("Esc", Style::default().fg(Color::Yellow)),
        Span::raw(" cancel"),
    ]));
    frame.render_widget(footer, chunks[3]);
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
    use crate::client::ContentSide;

    #[test]
    fn new_sets_default_path() {
        let state = SaveEventContentState::new(42, ContentSide::Input);
        assert_eq!(state.event_id, 42);
        assert!(state.path.contains("42"));
        assert!(state.path.contains("input"));
        assert!(state.path.ends_with(".bin"));
    }

    #[test]
    fn push_backspace_clear() {
        let mut state = SaveEventContentState::new(1, ContentSide::Output);
        let original_len = state.path.len();
        state.push_char('x');
        assert_eq!(state.path.len(), original_len + 1);
        state.backspace();
        assert_eq!(state.path.len(), original_len);
        state.clear();
        assert!(state.path.is_empty());
    }
}
