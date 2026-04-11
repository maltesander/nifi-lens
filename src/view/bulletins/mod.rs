//! Bulletins tab: cluster-wide bulletin tail.

pub mod render;
pub mod state;
pub mod worker;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{BulletinsState, apply_payload};

pub fn render(frame: &mut Frame, area: Rect, state: &BulletinsState) {
    render::render(frame, area, state);
}
