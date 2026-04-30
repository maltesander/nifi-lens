//! Overview tab: cluster health dashboard.
//!
//! - `state` holds pure reducers (testable without a terminal).
//! - `render` draws the snapshot into a ratatui frame.
//!
//! The per-view worker has been retired — Overview is now store-only.
//! Its projections (`root_pg`, `cs_counts`, `controller`, `nodes`,
//! `repositories_summary`, bulletin sparkline, noisy components,
//! unhealthy queues) are mirrored from `AppState.cluster.snapshot`
//! by the `redraw_*` reducers in [`state`], which the main loop invokes
//! on every `ClusterChanged` variant Overview cares about.

pub mod render;
pub mod state;

use ratatui::Frame;
use ratatui::layout::Rect;

pub use state::OverviewState;

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    render::render(frame, area, state);
}
