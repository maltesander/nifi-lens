//! Overview tab: cluster health dashboard.
//!
//! This module will grow through Phase 1 to include a pure state reducer
//! (`state`), a ratatui renderer (`render`), and a polling worker task
//! (`worker`). For the duration of Task 1 it preserves the Phase 0
//! placeholder so the directory move is a no-op from the user's view.

use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Overview", "Phase 1");
}
