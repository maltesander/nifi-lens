//! Health tab: cluster-wide operational health dashboard.

use ratatui::Frame;
use ratatui::layout::Rect;

pub mod render;
pub mod state;
pub mod worker;
pub use state::HealthState;

pub fn render(frame: &mut Frame, area: Rect, state: &HealthState) {
    render::render(frame, area, state);
}
