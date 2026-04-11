use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

const HELP_TEXT: &str = "\
Global Keys:
  Tab / Shift+Tab   Cycle tabs
  F1..F4            Jump to tab
  Ctrl+K            Switch context
  Ctrl+F            Fuzzy find (not yet implemented)
  ?                 Toggle this help
  q / Ctrl+Q        Quit
  Esc               Close modal

(Phase 0 — per-view help comes in later phases.)";

pub fn render(frame: &mut Frame, area: Rect) {
    let modal = center(area, 60, 14);
    frame.render_widget(Clear, modal);
    let block = Block::default().title(" Help ").borders(Borders::ALL);
    let p = Paragraph::new(HELP_TEXT)
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
