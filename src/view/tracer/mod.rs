//! Tracer tab: forensic flowfile investigation.

pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::TracerState;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Tracer", "Phase 4");
}
