//! Bulletins tab: cluster-wide bulletin tail.

pub mod render;
pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{BulletinsState, redraw_bulletins};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &BulletinsState,
    browser: &crate::view::browser::state::BrowserState,
    cfg: &crate::timestamp::TimestampConfig,
) {
    render::render(frame, area, state, browser, cfg);
}
