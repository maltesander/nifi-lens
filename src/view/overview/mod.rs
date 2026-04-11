//! Overview tab: cluster health dashboard.
//!
//! - `state` holds a pure reducer (testable without a terminal).
//! - `render` draws the snapshot into a ratatui frame.
//! - `worker` spawns the polling task that feeds the reducer.

pub mod render;
pub mod state;
pub mod worker;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{OverviewState, apply_payload};

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    render::render(frame, area, state);
}
