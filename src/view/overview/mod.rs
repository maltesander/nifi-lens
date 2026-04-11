//! Overview tab: cluster health dashboard.

pub mod render;
pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{OverviewState, apply_payload};

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    render::render(frame, area, state);
}
