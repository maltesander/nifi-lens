//! Ratatui renderer for the Overview tab.
//!
//! The node detail modal lives in `node_detail.rs` and is re-exported here
//! so callers at `crate::view::overview::render::render_node_detail_modal`
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

pub mod node_detail;
pub use node_detail::render_node_detail_modal;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Cell, Paragraph, Row, Table};

use super::state::{
    BulletinBucket, NoisyComponent, OverviewFocus, OverviewState, SPARKLINE_MINUTES, Severity,
    UnhealthyQueue,
};
use crate::app::navigation::compute_scroll_window;
use crate::client::{ControllerServiceCounts, ControllerStatusSnapshot, ProcessorStateCounts};
use crate::theme;
use crate::widget::gauge::fill_bar;
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
    render_components_table(frame, components_inner, state);

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
    render_nodes_zone(frame, nodes_inner, state);

    // Bulletins + Noisy horizontal split, each in its own panel
    let bn_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(zones[2]);
    render_bulletins_and_noisy(frame, bn_chunks[0], bn_chunks[1], state);

    // Unhealthy queues panel
    let queues_focused = state.focus == OverviewFocus::Queues;
    let queues_block = Panel::new(" Unhealthy queues ")
        .focused(queues_focused)
        .into_block();
    let queues_inner = queues_block.inner(zones[3]);
    frame.render_widget(queues_block, zones[3]);
    render_unhealthy_queues(
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

/// Three-row Components table — process groups, processors, controller
/// services. Display-only; not focusable. Aligned columns: 2-pad +
/// 20-label + 4-count + 4-gap + repeating 12-slot (8 label + 4 value).
///
/// All projections are sourced from `OverviewState` fields mirrored
/// from `AppState.cluster.snapshot` by the `redraw_*` reducers. The
/// renderer shows "loading…" until both `root_pg_status` and
/// `controller_status` have landed in the cluster snapshot. The CS row
/// degrades to "cs list unavailable" when `state.cs_counts` is `None`.
fn render_components_table(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let (Some(controller), Some(root_pg)) = (state.controller.as_ref(), state.root_pg.as_ref())
    else {
        let line = Line::from(Span::styled("loading…", theme::muted()));
        frame.render_widget(Paragraph::new(line), area);
        return;
    };
    let lines = vec![
        pg_row(controller, root_pg),
        processors_row(&root_pg.processors),
        controller_services_row(state.cs_counts.as_ref()),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn pg_row(
    controller: &ControllerStatusSnapshot,
    root_pg: &crate::client::RootPgStatusSnapshot,
) -> Line<'static> {
    let mut spans = label_and_count("Process groups", root_pg.process_group_count);
    let stale = controller.stale;
    let modified = controller.locally_modified;
    let sync_err = controller.sync_failure;
    if stale + modified + sync_err == 0 {
        spans.extend(slot_text("all in sync", theme::muted()));
    } else {
        spans.extend(slot("STALE", stale, theme::warning()));
        spans.extend(slot_gap());
        spans.extend(slot("MODIFIED", modified, theme::warning()));
        spans.extend(slot_gap());
        spans.extend(slot("SYNC-ERR", sync_err, theme::error()));
    }
    spans.extend(slot_gap());
    spans.extend(slot("INPUTS", root_pg.input_port_count, theme::muted()));
    spans.extend(slot_gap());
    spans.extend(slot("OUTPUTS", root_pg.output_port_count, theme::muted()));
    Line::from(spans)
}

fn processors_row(p: &ProcessorStateCounts) -> Line<'static> {
    let mut spans = label_and_count("Processors", p.total());
    spans.extend(slot("RUNNING", p.running, theme::success()));
    spans.extend(slot_gap());
    spans.extend(slot("STOPPED", p.stopped, theme::warning()));
    spans.extend(slot_gap());
    spans.extend(slot("INVALID", p.invalid, theme::error()));
    spans.extend(slot_gap());
    spans.extend(slot("DISABLED", p.disabled, theme::muted()));
    Line::from(spans)
}

fn controller_services_row(counts: Option<&ControllerServiceCounts>) -> Line<'static> {
    match counts {
        Some(c) => {
            let mut spans = label_and_count("Controller services", c.total());
            spans.extend(slot("ENABLED", c.enabled, theme::success()));
            spans.extend(slot_gap());
            spans.extend(slot("DISABLED", c.disabled, theme::muted()));
            spans.extend(slot_gap());
            spans.extend(slot("INVALID", c.invalid, theme::error()));
            Line::from(spans)
        }
        None => Line::from(vec![
            Span::raw("  "),
            Span::raw(format!("{:<20}", "Controller services")),
            Span::styled(format!("{:>4}", "?"), theme::muted()),
            Span::raw("    "),
            Span::styled("cs list unavailable", theme::error()),
        ]),
    }
}

/// Returns `[pad, label-padded-to-20, count-right-aligned-in-4, 4-space-gap]`
/// — the fixed prefix every row shares.
fn label_and_count(label: &str, count: u32) -> Vec<Span<'static>> {
    vec![
        Span::raw("  "),
        Span::raw(format!("{:<20}", label)),
        Span::styled(format!("{:>4}", count), theme::accent()),
        Span::raw("    "),
    ]
}

/// One status slot (12 chars total): label left-aligned in 8, value
/// right-aligned in 4. Returns 2 spans (label, value) — caller adds the gap.
fn slot(label: &'static str, value: u32, value_style: Style) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{:<8}", label), theme::muted()),
        Span::styled(format!("{:>4}", value), value_style),
    ]
}

/// One status slot occupied by a single text chip (e.g. "all in sync"),
/// padded to 12 chars total to stay aligned with the numeric slots.
fn slot_text(text: &'static str, style: Style) -> Vec<Span<'static>> {
    vec![Span::styled(format!("{:<12}", text), style)]
}

/// Two-space gap between consecutive slots.
fn slot_gap() -> Vec<Span<'static>> {
    vec![Span::raw("  ")]
}

fn render_nodes_zone(frame: &mut Frame, area: Rect, state: &OverviewState) {
    render_nodes_zone_at(
        frame,
        area,
        state,
        state.focus == OverviewFocus::Nodes,
        time::OffsetDateTime::now_utc(),
    );
}

pub(crate) fn render_nodes_zone_at(
    frame: &mut Frame,
    area: Rect,
    state: &OverviewState,
    focused: bool,
    now: time::OffsetDateTime,
) {
    let total = state.nodes.nodes.len();

    if total == 0 {
        frame.render_widget(
            Paragraph::new(Span::styled(
                "  loading system diagnostics…",
                theme::muted(),
            )),
            area,
        );
        return;
    }

    let any_cluster = state.nodes.nodes.iter().any(|n| n.cluster.is_some());
    let selected = state.nodes.selected;
    // Reserve 1 row at the bottom for the repositories aggregate.
    let visible_node_rows = area.height.saturating_sub(1) as usize;
    let window = compute_scroll_window(selected, state.nodes.nodes.len(), visible_node_rows);

    let mut rows: Vec<Row> = state
        .nodes
        .nodes
        .iter()
        .skip(window.offset)
        .take(visible_node_rows)
        .enumerate()
        .map(|(idx, node)| {
            let is_focused_sel = focused && idx == window.selected_local;
            let is_dead = node
                .cluster
                .as_ref()
                .map(|c| c.status.is_dead())
                .unwrap_or(false);
            let row_style = if is_focused_sel {
                theme::cursor_row()
            } else if is_dead {
                theme::muted()
            } else {
                Style::default()
            };
            node_to_row(node, any_cluster, is_dead, now).style(row_style)
        })
        .collect();

    // Cluster-aggregate repositories row. When badges are rendered,
    // prepend an empty cell so the columns line up with node rows.
    let repos = &state.repositories_summary;
    let mut footer_cells = Vec::new();
    if any_cluster {
        footer_cells.push(Cell::from(""));
    }
    footer_cells.extend(repos_footer_cells(repos));
    // Trailing empty cell to match the cert-chip column added to node rows.
    footer_cells.push(Cell::from(""));
    rows.push(Row::new(footer_cells));

    let widths: Vec<Constraint> = if any_cluster {
        vec![
            Constraint::Length(6),  // badge
            Constraint::Fill(1),    // address + heartbeat age
            Constraint::Length(15), // heap
            Constraint::Length(15), // gc
            Constraint::Length(14), // load
            Constraint::Length(13), // cert chip
        ]
    } else {
        vec![
            Constraint::Fill(1),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(14),
            Constraint::Length(13), // cert chip
        ]
    };
    frame.render_widget(Table::new(rows, widths), area);
}

/// Build the four-cell repositories footer row.
fn repos_footer_cells(repos: &super::state::RepositoriesSummary) -> Vec<Cell<'static>> {
    vec![
        Cell::from(Span::styled("  repositories", theme::muted())),
        Cell::from(Line::from(vec![
            Span::raw("cont "),
            Span::styled(
                fill_bar(5, repos.content_percent),
                fill_style(repos.content_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.content_percent),
                fill_style(repos.content_percent),
            ),
        ])),
        Cell::from(Line::from(vec![
            Span::raw("ff  "),
            Span::styled(
                fill_bar(5, repos.flowfile_percent),
                fill_style(repos.flowfile_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.flowfile_percent),
                fill_style(repos.flowfile_percent),
            ),
        ])),
        Cell::from(Line::from(vec![
            Span::raw("prov "),
            Span::styled(
                fill_bar(4, repos.provenance_percent),
                fill_style(repos.provenance_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.provenance_percent),
                fill_style(repos.provenance_percent),
            ),
        ])),
    ]
}

/// Build the heap `Cell` for a live node row.
fn heap_cell_for(node: &crate::client::health::NodeHealthRow) -> Cell<'static> {
    let heap_style = health_severity_style(node.heap_severity);
    let heap_bar = fill_bar(5, node.heap_percent);
    Cell::from(Line::from(vec![
        Span::raw("heap "),
        Span::styled(heap_bar, heap_style),
        Span::raw(" "),
        Span::styled(format!("{:>3}%", node.heap_percent), heap_style),
    ]))
}

/// Build the GC `Cell` for a live node row.
fn gc_cell_for(node: &crate::client::health::NodeHealthRow) -> Cell<'static> {
    let gc_style = match node.gc_delta {
        Some(d) if d > 5 => theme::error(),
        Some(_) => theme::warning(),
        None => Style::default(),
    };
    let gc_str = match node.gc_delta {
        Some(d) => format!("{:>4}ms (+{})", node.gc_millis, d),
        None => format!("{:>4}ms", node.gc_millis),
    };
    Cell::from(Line::from(vec![
        Span::raw("gc "),
        Span::styled(gc_str, gc_style),
    ]))
}

/// Build the load-average `Cell` for a live node row.
fn load_cell_for(node: &crate::client::health::NodeHealthRow) -> Cell<'static> {
    let (load_str, load_style) = match (node.load_average, node.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            let ratio = l / cpus as f64;
            let style = if ratio >= 2.0 {
                theme::error()
            } else if ratio >= 1.0 {
                theme::warning()
            } else {
                theme::success()
            };
            (format!("{l:>4.1}"), style)
        }
        (Some(l), _) => (format!("{l:>4.1}"), Style::default()),
        (None, _) => ("\u{2014}   ".to_string(), theme::muted()),
    };
    let load_bar = match (node.load_average, node.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            format!("{:.2}", (l / cpus as f64).min(9.99))
        }
        _ => "    ".to_string(),
    };
    Cell::from(Line::from(vec![
        Span::raw("load "),
        Span::styled(load_bar, load_style),
        Span::raw(" "),
        Span::styled(load_str, load_style),
    ]))
}

/// Return a cert-expiry chip cell for the trailing column of a node row.
///
/// Rules:
/// - `None` or probe-failed → empty cell (silent).
/// - `earliest_not_after - now >= 30d` → empty cell (healthy, no noise).
/// - `14d <= delta < 30d` → `cert Nd` in yellow.
/// - `delta < 7d` → `cert Nd` in red/bold.
/// - Expired (`delta < 0`) → `cert expired` in red/bold.
fn cert_chip_cell_for(
    node: &crate::client::health::NodeHealthRow,
    now: time::OffsetDateTime,
) -> Cell<'static> {
    let Some(Ok(chain)) = node.tls_cert.as_ref() else {
        return Cell::from("");
    };
    let Some(earliest) = chain.earliest_not_after() else {
        return Cell::from("");
    };
    let delta = earliest - now;
    if delta.is_negative() {
        return Cell::from(Span::styled(
            "cert expired",
            theme::error().add_modifier(Modifier::BOLD),
        ));
    }
    let days = delta.whole_days();
    if days >= 30 {
        return Cell::from("");
    }
    let style = if days < 7 {
        theme::error().add_modifier(Modifier::BOLD)
    } else {
        theme::warning()
    };
    Cell::from(Span::styled(format!("cert {days}d"), style))
}

/// Convert a `NodeHealthRow` into a ratatui `Row`.
///
/// When `include_badge` is `true` a leading 6-column badge cell is
/// prepended and the address cell includes the heartbeat age span.
/// When `is_dead` is `true` the heap/gc/load cells are replaced with
/// `───` placeholders.
fn node_to_row(
    node: &crate::client::health::NodeHealthRow,
    include_badge: bool,
    is_dead: bool,
    now: time::OffsetDateTime,
) -> Row<'static> {
    // Badge cell (only when rendering cluster info).
    let badge_cell = include_badge.then(|| match &node.cluster {
        Some(c) => Cell::from(crate::widget::node_badge::node_badge(c)),
        None => Cell::from(""),
    });

    // Address + optional heartbeat age.
    let hb_span = node
        .cluster
        .as_ref()
        .map(|c| {
            let age_text = crate::timestamp::format_age(c.heartbeat_age);
            Span::styled(
                format!("  {age_text}"),
                heartbeat_age_style(c.heartbeat_age),
            )
        })
        .unwrap_or_else(|| Span::raw(""));
    let address_cell = Cell::from(Line::from(vec![
        Span::styled(format!("  {}", node.node_address), theme::accent()),
        hb_span,
    ]));

    // Heap / gc / load — collapse to ─── for dead rows.
    let (heap_cell, gc_cell, load_cell) = if is_dead {
        let dim = theme::muted();
        (
            Cell::from(Span::styled(
                "heap \u{2500}\u{2500}\u{2500} \u{2500}\u{2500}\u{2500}",
                dim,
            )),
            Cell::from(Span::styled("gc \u{2500}\u{2500}\u{2500}", dim)),
            Cell::from(Span::styled("load \u{2500}\u{2500}\u{2500}", dim)),
        )
    } else {
        (heap_cell_for(node), gc_cell_for(node), load_cell_for(node))
    };

    let cert_cell = if is_dead {
        Cell::from("")
    } else {
        cert_chip_cell_for(node, now)
    };

    let mut cells = Vec::with_capacity(6);
    if let Some(b) = badge_cell {
        cells.push(b);
    }
    cells.push(address_cell);
    cells.push(heap_cell);
    cells.push(gc_cell);
    cells.push(load_cell);
    cells.push(cert_cell);
    Row::new(cells)
}

/// Style for the heartbeat age text appended to the address cell.
fn heartbeat_age_style(age: Option<std::time::Duration>) -> Style {
    match age {
        None => theme::muted(),
        Some(d) if d.as_secs() < 30 => theme::muted(),
        Some(d) if d.as_secs() < 120 => theme::warning(),
        Some(_) => theme::error(),
    }
}

fn render_bulletins_and_noisy(
    frame: &mut Frame,
    bulletins_area: Rect,
    noisy_area: Rect,
    state: &OverviewState,
) {
    let bulletins_block = Panel::new(" Bulletins / min ").into_block();
    let bulletins_inner = bulletins_block.inner(bulletins_area);
    frame.render_widget(bulletins_block, bulletins_area);
    render_bulletin_sparkline(frame, bulletins_inner, &state.sparkline);

    let noisy_focused = state.focus == OverviewFocus::Noisy;
    let noisy_block = Panel::new(" Noisy components ")
        .focused(noisy_focused)
        .into_block();
    let noisy_inner = noisy_block.inner(noisy_area);
    frame.render_widget(noisy_block, noisy_area);
    render_noisy_components(
        frame,
        noisy_inner,
        &state.noisy,
        noisy_focused,
        state.noisy_selected,
    );
}

fn render_bulletin_sparkline(frame: &mut Frame, area: Rect, buckets: &[BulletinBucket]) {
    // Trim leading zero-count buckets so bars start from the left edge
    // rather than appearing mid-chart while the window is still filling up.
    let first_nonempty = buckets
        .iter()
        .position(|b| b.count > 0)
        .unwrap_or(buckets.len());
    let visible = &buckets[first_nonempty..];

    // Layout: legend (1) | grouped bars (3) | time axis (1).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    // ── Legend ────────────────────────────────────────────────────────────────
    let legend = if visible.is_empty() {
        Line::from(Span::styled(
            format!("{}m  no bulletins", SPARKLINE_MINUTES),
            theme::muted(),
        ))
    } else {
        Line::from(vec![
            Span::styled(format!("{}m  ", SPARKLINE_MINUTES), theme::muted()),
            Span::styled("■ Err  ", severity_style(Severity::Error)),
            Span::styled("■ Warn  ", severity_style(Severity::Warning)),
            Span::styled("■ Info", severity_style(Severity::Info)),
        ])
    };
    frame.render_widget(Paragraph::new(legend), chunks[0]);

    if visible.is_empty() {
        return;
    }

    // One BarGroup per visible minute with three bars (Error, Warning, Info).
    // All bars share the same chart so they scale to a common maximum —
    // this makes them grow as a unit rather than each track independently.
    //
    // bar_width: fill available space across visible groups, 3 bars each,
    // with a 1-column gap between groups for readability.
    const GROUP_GAP: u16 = 1;
    let n = visible.len() as u16;
    let bar_width = area
        .width
        .saturating_sub(n.saturating_sub(1) * GROUP_GAP)
        .checked_div(n * 3)
        .unwrap_or(1)
        .max(1);

    let groups: Vec<BarGroup> = visible
        .iter()
        .map(|b| {
            BarGroup::default().bars(&[
                Bar::default()
                    .value(b.error_count as u64)
                    .style(severity_style(Severity::Error))
                    .text_value(""),
                Bar::default()
                    .value(b.warning_count as u64)
                    .style(severity_style(Severity::Warning))
                    .text_value(""),
                Bar::default()
                    .value(b.info_count as u64)
                    .style(severity_style(Severity::Info))
                    .text_value(""),
            ])
        })
        .collect();

    frame.render_widget(
        BarChart::grouped(groups)
            .bar_width(bar_width)
            .bar_gap(0)
            .group_gap(GROUP_GAP)
            .bar_set(symbols::bar::NINE_LEVELS),
        chunks[1],
    );

    // ── Time-axis grid ────────────────────────────────────────────────────────
    let oldest_min = SPARKLINE_MINUTES - first_nonempty;
    let left_label = format!("←{}m", oldest_min);
    let right_label = "now→";
    let fill = (area.width as usize).saturating_sub(left_label.len() + right_label.len());
    let axis_line = Line::from(vec![
        Span::styled(left_label, theme::muted()),
        Span::raw(" ".repeat(fill)),
        Span::styled(right_label, theme::muted()),
    ]);
    frame.render_widget(Paragraph::new(axis_line), chunks[2]);
}

fn render_noisy_components(
    frame: &mut Frame,
    area: Rect,
    noisy: &[NoisyComponent],
    focused: bool,
    selected: usize,
) {
    let rows: Vec<Row> = if noisy.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled("no bulletins yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        let visible_rows = area.height.saturating_sub(1) as usize;
        let window = compute_scroll_window(selected, noisy.len(), visible_rows);
        noisy
            .iter()
            .skip(window.offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, n)| {
                let sev_style = severity_style(n.max_severity);
                let row_style = if focused && idx == window.selected_local {
                    theme::cursor_row()
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(format!("{:>3}", n.count)).style(theme::bold()),
                    Cell::from(n.source_name.clone()),
                    Cell::from(format!("{:?}", n.max_severity)).style(sev_style),
                ])
                .style(row_style)
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Fill(1),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["cnt", "source", "worst"]).style(theme::bold()));
    frame.render_widget(table, area);
}

fn render_unhealthy_queues(
    frame: &mut Frame,
    area: Rect,
    queues: &[UnhealthyQueue],
    focused: bool,
    selected: usize,
) {
    let rows: Vec<Row> = if queues.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(Span::styled("no queues reported yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        let visible_rows = area.height.saturating_sub(1) as usize;
        let window = compute_scroll_window(selected, queues.len(), visible_rows);
        queues
            .iter()
            .skip(window.offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, q)| {
                let style = fill_style(q.fill_percent);
                let row_style = if focused && idx == window.selected_local {
                    theme::cursor_row()
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(format!("{:>3}%", q.fill_percent)).style(style),
                    Cell::from(q.name.clone()),
                    Cell::from(format!("{} → {}", q.source_name, q.destination_name)),
                    Cell::from(q.flow_files_queued.to_string()),
                ])
                .style(row_style)
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Fill(2),
            Constraint::Fill(3),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["fill", "queue", "src → dst", "ffiles"]).style(theme::bold()));
    frame.render_widget(table, area);
}

pub(super) fn fill_style(percent: u32) -> Style {
    match percent {
        0..=49 => theme::success(),
        50..=79 => theme::warning(),
        _ => theme::error(),
    }
}

/// Convert a `client::health::Severity` (Green/Yellow/Red) into a theme style.
pub(super) fn health_severity_style(s: crate::client::health::Severity) -> Style {
    use crate::client::health::Severity as H;
    match s {
        H::Green => theme::success(),
        H::Yellow => theme::warning(),
        H::Red => theme::error(),
    }
}

/// Convert a bulletin `Severity` (Error/Warning/Info/Unknown) into a theme style.
fn severity_style(s: Severity) -> Style {
    match s {
        Severity::Error => theme::error(),
        Severity::Warning => theme::warning(),
        Severity::Info => theme::info(),
        Severity::Unknown => theme::muted(),
    }
}

#[cfg(test)]
mod tests {
    use crate::client::{
        BulletinSnapshot, ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot,
    };
    use crate::view::overview::state::OverviewState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    // 2026-04-11T10:14:22Z in unix seconds. Same constant as the reducer
    // tests so time-dependent rendering stays deterministic. Verified
    // with `date -u -d @1775902462`.
    const T0: u64 = 1_775_902_462;

    fn render_to_string(state: &OverviewState) -> String {
        let backend = TestBackend::new(100, 25);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render(f, f.area(), state)).unwrap();
        format!("{}", term.backend())
    }

    /// Mirror `redraw_components`'s effect on the `OverviewState`
    /// (without threading through `&mut AppState`): set `root_pg` and
    /// derive `unhealthy` via the shared `derive_unhealthy` helper so
    /// this test path can't drift from the live reducer.
    fn seed_root_pg(state: &mut OverviewState, root_pg: RootPgStatusSnapshot) {
        state.unhealthy = crate::view::overview::state::derive_unhealthy(&root_pg);
        state.root_pg = Some(root_pg);
    }

    /// Mirror what `redraw_controller_status` would do: write the
    /// supplied snapshot into `state.controller`. Replaces the pre-Task-8
    /// `apply_payload(PgStatus(..))` path in render tests.
    fn seed_controller_status(state: &mut OverviewState, controller: ControllerStatusSnapshot) {
        state.controller = Some(controller);
    }

    /// Mirror what `redraw_sysdiag` would do: write the `nodes` /
    /// `repositories_summary` projections without threading an
    /// `&mut AppState`. Replaces the pre-Task-8
    /// `apply_payload(SystemDiag(..))` path in render tests.
    fn seed_sysdiag(state: &mut OverviewState, diag: &crate::client::health::SystemDiagSnapshot) {
        use crate::view::overview::state::RepositoriesSummary;
        crate::client::health::update_nodes(&mut state.nodes, diag, None, None);
        let avg = |repos: &[crate::client::health::RepoUsage]| -> u32 {
            if repos.is_empty() {
                0
            } else {
                repos.iter().map(|r| r.utilization_percent).sum::<u32>() / repos.len() as u32
            }
        };
        let agg = &diag.aggregate;
        state.repositories_summary = RepositoriesSummary {
            content_percent: avg(&agg.content_repos),
            flowfile_percent: agg
                .flowfile_repo
                .as_ref()
                .map(|r| r.utilization_percent)
                .unwrap_or(0),
            provenance_percent: avg(&agg.provenance_repos),
        };
    }

    /// Render-test shim: build the sparkline + noisy-components
    /// projections that `redraw_bulletin_projections` would produce
    /// against the cluster ring, without constructing a full
    /// `AppState`. Keeps the pre-Task-7 snapshot expectations stable
    /// for snapshot tests that feed bulletins as a vector.
    fn seed_bulletin_projections_from_bulletins(
        state: &mut OverviewState,
        bulletins: &[BulletinSnapshot],
        fetched_secs: i64,
    ) {
        use super::super::state::{SPARKLINE_MINUTES, parse_iso_seconds};
        use std::collections::HashMap;

        // Sparkline — mirror the bulk of `redraw_bulletin_projections`.
        let epoch_secs = state
            .sparkline_epoch_secs
            .unwrap_or_else(|| (fetched_secs / 60) * 60);
        let minutes_elapsed = ((fetched_secs - epoch_secs) / 60).max(0) as usize;
        let new_epoch = if minutes_elapsed > 0 {
            let shift = minutes_elapsed.min(SPARKLINE_MINUTES);
            state.sparkline.rotate_left(shift);
            for i in (SPARKLINE_MINUTES - shift)..SPARKLINE_MINUTES {
                state.sparkline[i] = super::BulletinBucket::default();
            }
            epoch_secs + (minutes_elapsed as i64 * 60)
        } else {
            epoch_secs
        };
        state.sparkline_epoch_secs = Some(new_epoch);

        for b in bulletins {
            let Some(ts) = parse_iso_seconds(&b.timestamp_iso) else {
                continue;
            };
            let age_secs = fetched_secs - ts;
            if age_secs < 0 {
                continue;
            }
            let minute = (age_secs / 60) as usize;
            if minute >= SPARKLINE_MINUTES {
                continue;
            }
            let bucket = &mut state.sparkline[SPARKLINE_MINUTES - 1 - minute];
            bucket.count = bucket.count.saturating_add(1);
            let sev = super::Severity::parse(&b.level);
            match sev {
                super::Severity::Error => {
                    bucket.error_count = bucket.error_count.saturating_add(1);
                }
                super::Severity::Warning => {
                    bucket.warning_count = bucket.warning_count.saturating_add(1);
                }
                super::Severity::Info => {
                    bucket.info_count = bucket.info_count.saturating_add(1);
                }
                super::Severity::Unknown => {}
            }
            if sev > bucket.max_severity {
                bucket.max_severity = sev;
            }
        }

        // Noisy components.
        let mut by_source: HashMap<String, super::NoisyComponent> = HashMap::new();
        for b in bulletins {
            if b.source_id.is_empty() {
                continue;
            }
            let entry =
                by_source
                    .entry(b.source_id.clone())
                    .or_insert_with(|| super::NoisyComponent {
                        source_id: b.source_id.clone(),
                        source_name: b.source_name.clone(),
                        group_id: b.group_id.clone(),
                        ..super::NoisyComponent::default()
                    });
            entry.count = entry.count.saturating_add(1);
            let sev = super::Severity::parse(&b.level);
            if sev > entry.max_severity {
                entry.max_severity = sev;
            }
        }
        let mut noisy: Vec<super::NoisyComponent> = by_source.into_values().collect();
        noisy.sort_by(|a, b| {
            b.count
                .cmp(&a.count)
                .then_with(|| b.max_severity.cmp(&a.max_severity))
                .then_with(|| a.source_name.cmp(&b.source_name))
        });
        noisy.truncate(super::super::state::TOP_NOISY);
        state.noisy = noisy;
    }

    #[test]
    fn snapshot_empty_state() {
        let state = OverviewState::new();
        insta::assert_snapshot!("overview_empty", render_to_string(&state));
    }

    #[test]
    fn snapshot_healthy_cluster() {
        let mut state = OverviewState::new();
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 5,
                flow_files_queued: 120,
                bytes_queued: 4096,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
        );
        // Render reads `state.root_pg` and `state.cs_counts` directly
        // — seed the projections that `redraw_components` would
        // normally populate from the cluster snapshot.
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                flow_files_queued: 120,
                bytes_queued: 4096,
                connections: vec![QueueSnapshot {
                    id: "c1".into(),
                    group_id: "root".into(),
                    name: "ingest → enrich".into(),
                    source_name: "Generate".into(),
                    destination_name: "Enrich".into(),
                    fill_percent: 12,
                    flow_files_queued: 40,
                    bytes_queued: 512,
                    queued_display: "40 / 512 B".into(),
                }],
                process_group_count: 5,
                input_port_count: 2,
                output_port_count: 1,
                processors: crate::client::ProcessorStateCounts {
                    running: 42,
                    stopped: 3,
                    invalid: 0,
                    disabled: 1,
                },
                process_group_ids: vec![],
                nodes: vec![],
            },
        );
        state.cs_counts = Some(crate::client::ControllerServiceCounts {
            enabled: 12,
            disabled: 0,
            invalid: 0,
        });
        insta::assert_snapshot!("overview_healthy", render_to_string(&state));
    }

    #[test]
    fn snapshot_drift() {
        use crate::client::{ControllerServiceCounts, ProcessorStateCounts};
        let mut state = OverviewState::new();
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                stale: 1,
                locally_modified: 2,
                sync_failure: 0,
                up_to_date: 4,
                ..Default::default()
            },
        );
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                process_group_count: 7,
                input_port_count: 2,
                output_port_count: 1,
                processors: ProcessorStateCounts {
                    running: 42,
                    stopped: 3,
                    invalid: 0,
                    disabled: 1,
                },
                ..Default::default()
            },
        );
        state.cs_counts = Some(ControllerServiceCounts {
            enabled: 12,
            disabled: 0,
            invalid: 0,
        });
        insta::assert_snapshot!("overview_drift", render_to_string(&state));
    }

    #[test]
    fn snapshot_cs_unavailable() {
        let mut state = OverviewState::new();
        seed_controller_status(&mut state, ControllerStatusSnapshot::default());
        seed_root_pg(&mut state, RootPgStatusSnapshot::default());
        // `state.cs_counts` is left as the default `None` to exercise
        // the "cs list unavailable" degradation path.
        insta::assert_snapshot!("overview_cs_unavailable", render_to_string(&state));
    }

    #[test]
    fn snapshot_unhealthy_cluster() {
        let mut state = OverviewState::new();
        let queues = (0..5)
            .map(|i| QueueSnapshot {
                id: format!("c{i}"),
                group_id: "root".into(),
                name: format!("q{i}"),
                source_name: "Generate".into(),
                destination_name: format!("Proc{i}"),
                fill_percent: 99 - i,
                flow_files_queued: 9_000 + i * 100,
                bytes_queued: 1_000_000,
                queued_display: format!("{}k / 1 MB", 9 + i),
            })
            .collect();
        // Pre-Task-7 the bulletins rode on the PG-status payload and
        // drove sparkline+noisy via `apply_payload`. Task 7 moved that
        // path to `redraw_bulletin_projections` on `&mut AppState`.
        // This render test drives `OverviewState` directly, so we
        // pre-populate the projections that the reducer would have
        // built, keeping the rendered output stable.
        let bulletins: Vec<BulletinSnapshot> = (0..6)
            .map(|i| BulletinSnapshot {
                id: i,
                level: if i % 2 == 0 {
                    "ERROR".into()
                } else {
                    "WARN".into()
                },
                message: "msg".into(),
                source_id: format!("proc-{}", i % 3),
                source_name: format!("Proc{}", i % 3),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:10Z".into(),
                timestamp_human: String::new(),
            })
            .collect();
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 20,
                stopped: 10,
                invalid: 2,
                disabled: 0,
                active_threads: 17,
                flow_files_queued: 50_000,
                bytes_queued: 8_000_000,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
        );
        // Hand-build the bulletin-derived projections matching the
        // pre-Task-7 output.
        seed_bulletin_projections_from_bulletins(&mut state, &bulletins, T0 as i64);
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                flow_files_queued: 50_000,
                bytes_queued: 8_000_000,
                connections: queues,
                process_group_count: 4,
                input_port_count: 0,
                output_port_count: 0,
                processors: crate::client::ProcessorStateCounts {
                    running: 20,
                    stopped: 10,
                    invalid: 2,
                    disabled: 0,
                },
                process_group_ids: vec![],
                nodes: vec![],
            },
        );
        state.cs_counts = Some(crate::client::ControllerServiceCounts {
            enabled: 6,
            disabled: 1,
            invalid: 1,
        });
        insta::assert_snapshot!("overview_unhealthy", render_to_string(&state));
    }

    #[test]
    fn snapshot_with_nodes_populated() {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use std::time::Instant;

        let mut state = OverviewState::new();

        // Seed controller_status so the processor info line has data.
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 5,
                flow_files_queued: 120,
                bytes_queued: 4096,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
        );

        // Then a two-node sysdiag projection. Fixture mirrors the
        // reducer test in src/view/overview/state.rs.
        let node = |address: &str| NodeDiagnostics {
            address: address.into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            gc: vec![GcSnapshot {
                name: "G1 Young".into(),
                collection_count: 10,
                collection_millis: 50,
            }],
            load_average: Some(1.5),
            available_processors: Some(4),
            total_threads: 50,
            uptime: "1h".into(),
            content_repos: vec![RepoUsage {
                identifier: "content".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "flowfile".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "provenance".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        };
        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![RepoUsage {
                    identifier: "content".into(),
                    used_bytes: 60,
                    total_bytes: 100,
                    free_bytes: 40,
                    utilization_percent: 60,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "flowfile".into(),
                    used_bytes: 30,
                    total_bytes: 100,
                    free_bytes: 70,
                    utilization_percent: 30,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "provenance".into(),
                    used_bytes: 20,
                    total_bytes: 100,
                    free_bytes: 80,
                    utilization_percent: 20,
                }],
            },
            nodes: vec![node("node1:8080"), node("node2:8080")],
            fetched_at: Instant::now(),
        };
        seed_sysdiag(&mut state, &diag);
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                flow_files_queued: 120,
                bytes_queued: 4096,
                connections: vec![],
                process_group_count: 5,
                input_port_count: 2,
                output_port_count: 1,
                processors: crate::client::ProcessorStateCounts {
                    running: 42,
                    stopped: 3,
                    invalid: 0,
                    disabled: 1,
                },
                process_group_ids: vec![],
                nodes: vec![],
            },
        );
        state.cs_counts = Some(crate::client::ControllerServiceCounts {
            enabled: 12,
            disabled: 0,
            invalid: 0,
        });

        insta::assert_snapshot!("overview_with_nodes", render_to_string(&state));
    }

    #[test]
    fn nodes_panel_scrolls_to_selected() {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use std::time::Instant;

        let mut state = OverviewState::new();
        state.focus = crate::view::overview::state::OverviewFocus::Nodes;

        let node = |i: usize| NodeDiagnostics {
            address: format!("node{}:8080", i),
            heap_used_bytes: 256 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            gc: vec![GcSnapshot {
                name: "G1 Young".into(),
                collection_count: 1,
                collection_millis: 5,
            }],
            load_average: Some(0.5),
            available_processors: Some(4),
            total_threads: 20,
            uptime: "1h".into(),
            content_repos: vec![RepoUsage {
                identifier: "c".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "f".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "p".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }],
        };

        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![RepoUsage {
                    identifier: "c".into(),
                    used_bytes: 10,
                    total_bytes: 100,
                    free_bytes: 90,
                    utilization_percent: 10,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "f".into(),
                    used_bytes: 10,
                    total_bytes: 100,
                    free_bytes: 90,
                    utilization_percent: 10,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "p".into(),
                    used_bytes: 10,
                    total_bytes: 100,
                    free_bytes: 90,
                    utilization_percent: 10,
                }],
            },
            nodes: (0..10).map(node).collect(),
            fetched_at: Instant::now(),
        };
        seed_sysdiag(&mut state, &diag);
        state.nodes.selected = 9;

        let output = render_to_string(&state);

        assert!(
            output.contains("node9:8080"),
            "selected row must be visible after scroll"
        );
        assert!(
            !output.contains("node0:8080"),
            "node0 must be scrolled out of view"
        );
        assert!(
            !output.contains("node1:8080"),
            "node1 must be scrolled out of view"
        );
        assert!(
            !output.contains("more"),
            "'... +N more' placeholder must not appear"
        );
    }

    #[test]
    fn noisy_panel_scrolls_to_selected() {
        use crate::client::{ControllerStatusSnapshot, RootPgStatusSnapshot};
        use crate::view::overview::state::{NoisyComponent, Severity as OvSev};

        let mut state = OverviewState::new();
        state.focus = crate::view::overview::state::OverviewFocus::Noisy;

        // Populate enough state for the layout to render properly.
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 1,
                stopped: 0,
                invalid: 0,
                disabled: 0,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
        );
        seed_root_pg(&mut state, RootPgStatusSnapshot::default());

        // Five noisy components with distinct names. zone[2] inner height=4,
        // visible_rows=3, so selected=4 forces scroll_offset=2. "alfa" scrolls away; "echo" appears.
        state.noisy = vec![
            NoisyComponent {
                source_id: "a".into(),
                group_id: "g".into(),
                source_name: "alfa".into(),
                count: 1,
                max_severity: OvSev::Info,
            },
            NoisyComponent {
                source_id: "b".into(),
                group_id: "g".into(),
                source_name: "bravo".into(),
                count: 1,
                max_severity: OvSev::Info,
            },
            NoisyComponent {
                source_id: "c".into(),
                group_id: "g".into(),
                source_name: "charlie".into(),
                count: 1,
                max_severity: OvSev::Info,
            },
            NoisyComponent {
                source_id: "d".into(),
                group_id: "g".into(),
                source_name: "delta".into(),
                count: 1,
                max_severity: OvSev::Info,
            },
            NoisyComponent {
                source_id: "e".into(),
                group_id: "g".into(),
                source_name: "echo".into(),
                count: 1,
                max_severity: OvSev::Info,
            },
        ];
        state.noisy_selected = 4;

        let output = render_to_string(&state);

        assert!(
            output.contains("echo"),
            "selected row 'echo' must be visible after scroll"
        );
        assert!(
            !output.contains("alfa"),
            "'alfa' must be scrolled out of view"
        );
    }

    #[test]
    fn queues_panel_scrolls_to_selected() {
        use crate::client::{ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot};

        let mut state = OverviewState::new();
        state.focus = crate::view::overview::state::OverviewFocus::Queues;

        // Ten queues with distinct names. With 0 nodes the queues inner area
        // is 10 rows tall, giving visible_rows=9 (one row is the header).
        // selected=9 forces scroll_offset=1. "alfa" scrolls away; "juliet" appears.
        let names = [
            "alfa", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india",
            "juliet",
        ];
        let connections: Vec<QueueSnapshot> = names
            .iter()
            .enumerate()
            .map(|(i, &name)| QueueSnapshot {
                id: format!("c{i}"),
                group_id: "root".into(),
                name: name.into(),
                source_name: "Src".into(),
                destination_name: "Dst".into(),
                fill_percent: 99,
                flow_files_queued: 100,
                bytes_queued: 0,
                queued_display: "100".into(),
            })
            .collect();

        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 1,
                stopped: 0,
                invalid: 0,
                disabled: 0,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
        );
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                flow_files_queued: 0,
                bytes_queued: 0,
                connections,
                ..Default::default()
            },
        );
        state.queues_selected = 9;

        let output = render_to_string(&state);

        assert!(
            output.contains("juliet"),
            "selected row 'juliet' must be visible after scroll"
        );
        assert!(
            !output.contains("alfa"),
            "'alfa' must be scrolled out of view"
        );
    }

    // ── T21 helpers and snapshot tests ───────────────────────────────────────

    /// Build a two-node `AppState` with sysdiag pre-seeded and basic
    /// controller/root-pg/cs data for a complete render.  The cluster-nodes
    /// snapshot is NOT yet applied, so every `NodeHealthRow` has
    /// `cluster = None` — this is the `any_cluster = false` baseline.
    fn seed_state_with_two_nodes() -> crate::app::state::AppState {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::Instant;

        let mut state = crate::test_support::fresh_state();

        // Seed controller_status.
        state.cluster.snapshot.controller_status = EndpointState::Ready {
            data: ControllerStatusSnapshot {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 5,
                flow_files_queued: 120,
                bytes_queued: 4096,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: std::time::Duration::from_millis(5),
                next_interval: std::time::Duration::from_secs(10),
            },
        };
        crate::view::overview::state::redraw_controller_status(&mut state);

        // Seed root-pg status.
        let root_pg = RootPgStatusSnapshot {
            flow_files_queued: 120,
            bytes_queued: 4096,
            connections: vec![],
            process_group_count: 5,
            input_port_count: 2,
            output_port_count: 1,
            processors: crate::client::ProcessorStateCounts {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
            },
            process_group_ids: vec![],
            nodes: vec![],
        };
        state.cluster.snapshot.root_pg_status = EndpointState::Ready {
            data: root_pg,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: std::time::Duration::from_millis(5),
                next_interval: std::time::Duration::from_secs(10),
            },
        };
        crate::view::overview::state::redraw_components(&mut state);

        // Seed sysdiag with two nodes.
        let node = |address: &str| NodeDiagnostics {
            address: address.into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            gc: vec![GcSnapshot {
                name: "G1 Young".into(),
                collection_count: 10,
                collection_millis: 50,
            }],
            load_average: Some(1.5),
            available_processors: Some(4),
            total_threads: 50,
            uptime: "1h".into(),
            content_repos: vec![RepoUsage {
                identifier: "content".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "flowfile".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "provenance".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        };
        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![RepoUsage {
                    identifier: "content".into(),
                    used_bytes: 60,
                    total_bytes: 100,
                    free_bytes: 40,
                    utilization_percent: 60,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "flowfile".into(),
                    used_bytes: 30,
                    total_bytes: 100,
                    free_bytes: 70,
                    utilization_percent: 30,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "provenance".into(),
                    used_bytes: 20,
                    total_bytes: 100,
                    free_bytes: 80,
                    utilization_percent: 20,
                }],
            },
            nodes: vec![node("node1:8080"), node("node2:8080")],
            fetched_at: Instant::now(),
        };
        state.cluster.snapshot.system_diagnostics = EndpointState::Ready {
            data: diag,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: std::time::Duration::from_millis(5),
                next_interval: std::time::Duration::from_secs(10),
            },
        };
        crate::view::overview::state::redraw_sysdiag(&mut state);

        state
    }

    #[test]
    fn snapshot_overview_with_cluster_roles() {
        use crate::client::health::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        use crate::cluster::snapshot::FetchMeta;

        // Seed two-node sysdiag, then apply cluster-nodes with primary +
        // coordinator.  Title: "Nodes (2/2 connected)".
        let mut state = seed_state_with_two_nodes();
        let cluster = ClusterNodesSnapshot {
            rows: vec![
                ClusterNodeRow {
                    node_id: "id-1".into(),
                    address: "node1:8080".into(),
                    status: ClusterNodeStatus::Connected,
                    is_primary: true,
                    is_coordinator: false,
                    heartbeat_iso: None,
                    node_start_iso: None,
                    active_thread_count: 4,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                    events: vec![],
                },
                ClusterNodeRow {
                    node_id: "id-2".into(),
                    address: "node2:8080".into(),
                    status: ClusterNodeStatus::Connected,
                    is_primary: false,
                    is_coordinator: true,
                    heartbeat_iso: None,
                    node_start_iso: None,
                    active_thread_count: 3,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                    events: vec![],
                },
            ],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        state.cluster.snapshot.cluster_nodes.apply(
            Ok(cluster),
            FetchMeta {
                fetched_at: std::time::Instant::now(),
                fetch_duration: std::time::Duration::from_millis(1),
                next_interval: std::time::Duration::from_secs(5),
            },
        );
        crate::view::overview::state::redraw_cluster_nodes(&mut state);
        insta::assert_snapshot!(
            "overview_with_cluster_roles",
            render_to_string(&state.overview)
        );
    }

    #[test]
    fn snapshot_overview_with_dead_node() {
        use crate::client::health::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        use crate::cluster::snapshot::FetchMeta;

        // node1 connected primary+coordinator; node2 disconnected.
        // Expected dim/─── cells on the dead row; title "Nodes (1/2 connected)".
        let mut state = seed_state_with_two_nodes();
        let cluster = ClusterNodesSnapshot {
            rows: vec![
                ClusterNodeRow {
                    node_id: "id-1".into(),
                    address: "node1:8080".into(),
                    status: ClusterNodeStatus::Connected,
                    is_primary: true,
                    is_coordinator: true,
                    heartbeat_iso: None,
                    node_start_iso: None,
                    active_thread_count: 4,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                    events: vec![],
                },
                ClusterNodeRow {
                    node_id: "id-2".into(),
                    address: "node2:8080".into(),
                    status: ClusterNodeStatus::Disconnected,
                    is_primary: false,
                    is_coordinator: false,
                    heartbeat_iso: None,
                    node_start_iso: None,
                    active_thread_count: 0,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                    events: vec![],
                },
            ],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        state.cluster.snapshot.cluster_nodes.apply(
            Ok(cluster),
            FetchMeta {
                fetched_at: std::time::Instant::now(),
                fetch_duration: std::time::Duration::from_millis(1),
                next_interval: std::time::Duration::from_secs(5),
            },
        );
        crate::view::overview::state::redraw_cluster_nodes(&mut state);
        insta::assert_snapshot!("overview_with_dead_node", render_to_string(&state.overview));
    }

    #[test]
    fn snapshot_overview_standalone_no_badges() {
        // No cluster-nodes snapshot applied. Every row has cluster = None,
        // so any_cluster = false and the pre-T20 4-column layout is
        // preserved (no badge column, old title format).
        let state = seed_state_with_two_nodes();
        insta::assert_snapshot!(
            "overview_standalone_no_badges",
            render_to_string(&state.overview)
        );
    }

    // ── T24 cert-chip column tests ────────────────────────────────────────────

    /// Fixed "now" shared with node_detail tests: 2026-04-24T00:00Z.
    fn fixed_now() -> time::OffsetDateTime {
        time::macros::datetime!(2026-04-24 00:00 UTC)
    }

    /// Build a minimal `NodeHealthRow` with the given address and `tls_cert`.
    fn row_with_tls(
        address: &str,
        tls_cert: Option<
            Result<crate::client::tls_cert::NodeCertChain, crate::client::tls_cert::TlsProbeError>,
        >,
    ) -> crate::client::health::NodeHealthRow {
        use crate::client::health::Severity;
        crate::client::health::NodeHealthRow {
            node_address: address.into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            heap_percent: 50,
            heap_severity: Severity::Green,
            gc_collection_count: 5,
            gc_delta: None,
            gc_millis: 20,
            load_average: Some(0.5),
            available_processors: Some(4),
            uptime: "1h".into(),
            total_threads: 20,
            gc: vec![],
            content_repos: vec![],
            flowfile_repo: None,
            provenance_repos: vec![],
            cluster: None,
            tls_cert,
        }
    }

    /// Build a `NodeCertChain` whose earliest `not_after` is `fixed_now() + days`.
    fn chain_expiring_in(days: i64) -> crate::client::tls_cert::NodeCertChain {
        use crate::client::tls_cert::{CertEntry, NodeCertChain};
        NodeCertChain {
            entries: vec![CertEntry {
                subject_cn: Some("n".into()),
                not_after: fixed_now() + time::Duration::days(days),
                is_leaf: true,
            }],
        }
    }

    /// Build a minimal `OverviewState` containing exactly the supplied node rows.
    fn state_with_node_rows(
        rows: Vec<crate::client::health::NodeHealthRow>,
    ) -> crate::view::overview::state::OverviewState {
        use crate::view::overview::state::{OverviewState, RepositoriesSummary};
        let mut state = OverviewState::new();
        state.nodes.nodes = rows;
        state.repositories_summary = RepositoriesSummary {
            content_percent: 42,
            flowfile_percent: 18,
            provenance_percent: 7,
        };
        state
    }

    #[test]
    fn snapshot_nodes_list_cert_chips_mixed() {
        use crate::client::tls_cert::TlsProbeError;
        let rows = vec![
            // n1: no TLS data → empty chip
            row_with_tls("n1:8443", None),
            // n2: expires in 400 days → silent (>= 30d threshold)
            row_with_tls("n3:8443", Some(Ok(chain_expiring_in(400)))),
            // n3: expires in 14 days → yellow "cert 14d"
            row_with_tls("n3:8443", Some(Ok(chain_expiring_in(14)))),
            // n4: expires in 3 days → red/bold "cert 3d"
            row_with_tls("n4:8443", Some(Ok(chain_expiring_in(3)))),
            // n5: expired 2 days ago → red/bold "cert expired"
            row_with_tls("n5:8443", Some(Ok(chain_expiring_in(-2)))),
            // n6: probe failed → empty chip (silent)
            row_with_tls(
                "n6:8443",
                Some(Err(TlsProbeError::Connect("refused".into()))),
            ),
        ];
        let state = state_with_node_rows(rows);
        let backend = ratatui::backend::TestBackend::new(110, 12);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render_nodes_zone_at(f, f.area(), &state, false, fixed_now()))
            .unwrap();
        insta::assert_snapshot!("nodes_list_cert_chips_mixed", format!("{}", term.backend()));
    }
}
