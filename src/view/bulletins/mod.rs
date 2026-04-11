//! Bulletins tab: cluster-wide bulletin tail.
//!
//! This module grows through Phase 2 to include a pure state reducer
//! (`state`), a ratatui renderer (`render`), and a polling worker task
//! (`worker`). Until the renderer lands in Task 12, the public `render`
//! entry point delegates to the Phase 0 placeholder so the directory
//! move is a no-op from the user's view.

pub mod state;
pub mod worker;

pub use state::{BulletinsState, apply_payload};

use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Bulletins", "Phase 2");
}
