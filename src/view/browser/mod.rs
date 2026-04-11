//! Browser tab: PG tree + per-node detail view.
//!
//! This module grows through Phase 3 to include a pure state reducer
//! (`state`), per-kind ratatui renderers (`render/{pg,processor,
//! connection,controller_service}.rs`), and a polling + detail-fetch
//! worker task (`worker`). Until the renderer lands in Task 18, the
//! public `render` entry point delegates to the Phase 0 placeholder so
//! the directory move is a no-op from the user's view.

pub mod render;

use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Browser", "Phase 3");
}
