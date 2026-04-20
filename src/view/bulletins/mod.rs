//! Bulletins tab: cluster-wide bulletin tail.

pub mod modal;
pub mod render;
pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::{BulletinsState, redraw_bulletins};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &mut BulletinsState,
    browser: &crate::view::browser::state::BrowserState,
    cfg: &crate::timestamp::TimestampConfig,
) {
    render::render(frame, area, state, browser, cfg);
    if state.detail_modal.is_some() {
        modal::render(frame, area, state);
    }
}
