//! Overview tab: cluster health dashboard.
//!
//! - `state` holds a pure reducer (testable without a terminal).
//! - `render` (Task 8) draws the snapshot into a ratatui frame.
//! - `worker` (Task 9) spawns the polling task that feeds the reducer.

pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{OverviewState, apply_payload};

pub fn render(frame: &mut Frame, area: Rect) {
    // Placeholder until Task 8 wires the real renderer.
    super::render_placeholder(frame, area, "Overview", "Phase 1");
}
