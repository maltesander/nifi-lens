//! Per-tab view modules. Phase 0 ships placeholders.

pub mod browser;
pub mod bulletins;
pub mod events;
pub mod overview;
pub mod tracer;

use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::theme;

/// Render a centered `"{title} — coming in {phase}"` message.
pub fn render_placeholder(frame: &mut Frame, area: Rect, title: &str, phase: &str) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(title.to_string());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let text = format!("{title} — coming in {phase}");
    let p = Paragraph::new(text)
        .style(theme::muted())
        .alignment(Alignment::Center);
    // Center the paragraph vertically by positioning it at the vertical middle.
    let mid = inner.height.saturating_sub(1) / 2;
    let centered = Rect {
        x: inner.x,
        y: inner.y + mid,
        width: inner.width,
        height: 1,
    };
    frame.render_widget(p, centered);
}
