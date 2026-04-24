//! Ratatui renderer for the Overview tab.
//!
//! Top-level `render` orchestrator + the shared style helpers used by
//! multiple sibling submodules. The four responsibility-focused panels
//! live in their own files:
//!
//! - `components.rs` — Components table (PGs / processors / CSes).
//! - `nodes.rs` — Nodes list + repositories aggregate row + cert chip.
//! - `bulletins_noisy.rs` — bulletins sparkline + noisy components table.
//! - `queues.rs` — unhealthy queues table.
//!
//! The node detail modal lives in `node_detail.rs` and is re-exported
//! here so callers at `crate::view::overview::render::render_node_detail_modal`
//! continue to work unchanged.
//!
//! Layout 3:
//!
//! ```text
//! ┌─ Components ──────────────────────────────────────────────────────────────┐
//! │  Process groups         5    all in sync   INPUTS     2  OUTPUTS    1    │ ← components panel (5 rows)
//! │  Processors            46    RUNNING   42  STOPPED    3  INVALID    0    │
//! │  Controller services   12    ENABLED   12  DISABLED   0  INVALID    0    │
//! ├─ Nodes (N connected) ─────────────────────────────────────────────────────┤ ← nodes panel (variable, capped)
//! │   node-name   heap  N%   gc Nms/5m   load N.N                            │
//! │   repositories  content  N%   flowfile  N%   provenance  N%              │
//! ├─ Bulletins / min ──────────────┬─ Noisy components ─────────────────────┤ ← bulletins+noisy panel (6 rows)
//! │  sparkline                     │  cnt  source              worst          │
//! ├─ Unhealthy queues ─────────────────────────────────────────────────────────┤ ← unhealthy queues panel (fills rest)
//! │ fill  queue             src → dst                      ffiles             │
//! └────────────────────────────────────────────────────────────────────────────┘
//! ```

mod bulletins_noisy;
mod components;
pub mod node_detail;
mod nodes;
mod queues;

pub use node_detail::render_node_detail_modal;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;

use super::state::{OverviewFocus, OverviewState};
use crate::theme;
use crate::widget::panel::Panel;

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    // Vertical panels. Nodes panel height adapts to the number of nodes
    // (1 row per node + 1 repos row) and the available terminal height,
    // capped at ~1/3 of the area so the bulletin/queue panels always keep
    // their share. +2 for the top/bottom border.
    let nodes_height = nodes_zone_height(state, area.height);
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),            // components panel (3 rows + 2 border)
            Constraint::Length(nodes_height), // nodes panel
            Constraint::Length(7),            // bulletins/noisy panel (5 content + 2 border)
            Constraint::Fill(1),              // unhealthy queues panel
        ])
        .split(area);

    // Components panel
    let components_block = Panel::new(" Components ").into_block();
    let components_inner = components_block.inner(zones[0]);
    frame.render_widget(components_block, zones[0]);
    components::render_components_table(frame, components_inner, state);

    // Nodes panel — title shows node count when populated.
    // When any row has cluster membership data (any_cluster = true) the
    // title switches to the "C/T connected" format.
    let total = state.nodes.nodes.len();
    let nodes_title = if total == 0 {
        " Nodes ".to_string()
    } else if state.nodes.nodes.iter().any(|n| n.cluster.is_some()) {
        let connected = state
            .nodes
            .nodes
            .iter()
            .filter(|r| {
                r.cluster
                    .as_ref()
                    .map(|c| c.status == crate::client::health::ClusterNodeStatus::Connected)
                    .unwrap_or(true)
            })
            .count();
        format!(" Nodes ({connected}/{total} connected) ")
    } else {
        format!(" Nodes ({total} connected) ")
    };
    let nodes_block = Panel::new(nodes_title)
        .focused(state.focus == OverviewFocus::Nodes)
        .into_block();
    let nodes_inner = nodes_block.inner(zones[1]);
    frame.render_widget(nodes_block, zones[1]);
    nodes::render_nodes_zone(frame, nodes_inner, state);

    // Bulletins + Noisy horizontal split, each in its own panel
    let bn_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(zones[2]);
    bulletins_noisy::render_bulletins_and_noisy(frame, bn_chunks[0], bn_chunks[1], state);

    // Unhealthy queues panel
    let queues_focused = state.focus == OverviewFocus::Queues;
    let queues_block = Panel::new(" Unhealthy queues ")
        .focused(queues_focused)
        .into_block();
    let queues_inner = queues_block.inner(zones[3]);
    frame.render_widget(queues_block, zones[3]);
    queues::render_unhealthy_queues(
        frame,
        queues_inner,
        &state.unhealthy,
        queues_focused,
        state.queues_selected,
    );
}

/// Compute how many rows the nodes panel needs. One row per visible node +
/// one row for the repositories aggregate + 2 rows for the panel border.
/// Grows with terminal height but capped at ~1/3 of the available area so
/// the bulletin/queue panels always keep their share; `render_nodes_zone`
/// scrolls when the node count exceeds what fits.
fn nodes_zone_height(state: &OverviewState, area_height: u16) -> u16 {
    // N node rows + 1 repositories row + 2 border rows. `max(1)` keeps a
    // slot for the loading message when the snapshot is empty.
    let visible_nodes = (state.nodes.nodes.len() as u16).max(1);
    let desired = visible_nodes + 1 + 2;
    let cap = (area_height / 3).max(4);
    desired.min(cap).max(4)
}

/// Shared fill-style helper. Used by both `nodes.rs` (repositories
/// footer) and `queues.rs` (queue fill column).
pub(super) fn fill_style(percent: u32) -> Style {
    match percent {
        0..=49 => theme::success(),
        50..=79 => theme::warning(),
        _ => theme::error(),
    }
}

/// Convert a `client::health::Severity` (Green/Yellow/Red) into a theme style.
///
/// Shared helper: used by `nodes.rs` for heap cells and by
/// `node_detail.rs` for the detail modal's resource gauges.
pub(super) fn health_severity_style(s: crate::client::health::Severity) -> Style {
    use crate::client::health::Severity as H;
    match s {
        H::Green => theme::success(),
        H::Yellow => theme::warning(),
        H::Red => theme::error(),
    }
}

#[cfg(test)]
mod tests;
