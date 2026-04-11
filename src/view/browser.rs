use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Browser", "Phase 3");
}
