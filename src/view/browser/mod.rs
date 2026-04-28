//! Browser tab: PG tree + per-node detail view.

pub mod render;
pub mod state;
pub mod worker;

use ratatui::Frame;
use ratatui::layout::Rect;

use crate::view::browser::state::{BrowserState, FlowIndex};

#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    flow_index: &Option<FlowIndex>,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    cluster: &crate::cluster::snapshot::ClusterSnapshot,
    age_warning: std::time::Duration,
    show_node_column: bool,
) {
    render::render(
        frame,
        area,
        state,
        flow_index,
        bulletins,
        cluster,
        age_warning,
        show_node_column,
    );
}
