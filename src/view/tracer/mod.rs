//! Tracer tab: forensic flowfile investigation.

pub mod render;
pub mod state;
pub mod worker;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::TracerState;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &TracerState,
    cfg: &crate::timestamp::TimestampConfig,
) {
    render::render(frame, area, state, cfg);
}
