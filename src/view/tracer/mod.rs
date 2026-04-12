//! Tracer tab: forensic flowfile investigation.
//!
//! This module grows through Phase 4 to include a pure state reducer
//! (`state`), a per-mode ratatui renderer (`render`), and five one-shot
//! workers (`worker`) for latest-events / lineage submit / lineage poll
//! / event detail / content fetches. Until the renderer lands in Task 14,
//! the public `render` entry point delegates to the Phase 0 placeholder so
//! the directory move is a no-op from the user's view.

use ratatui::Frame;
use ratatui::layout::Rect;

pub fn render(frame: &mut Frame, area: Rect) {
    super::render_placeholder(frame, area, "Tracer", "Phase 4");
}
