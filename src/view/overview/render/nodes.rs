//! Renderer for the Overview tab's Nodes panel.
//!
//! Draws one `Row` per `NodeHealthRow` plus a trailing repositories
//! aggregate row. Columns: optional badge, address + heartbeat age,
//! heap cell, gc cell, load cell, cert-expiry chip.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use super::super::state::{OverviewFocus, OverviewState};
use super::{fill_style, health_severity_style};
use crate::app::navigation::compute_scroll_window;
use crate::theme;
use crate::widget::gauge::fill_bar;

pub(super) fn render_nodes_zone(frame: &mut Frame, area: Rect, state: &OverviewState) {
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
fn repos_footer_cells(repos: &super::super::state::RepositoriesSummary) -> Vec<Cell<'static>> {
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
/// - `None` or probe-failed → empty cell (silent — no data to show).
/// - Expired (`delta < 0`) → `cert expired` in red/bold.
/// - `delta < 7d` → `cert Nd` in red/bold.
/// - `7d <= delta < 30d` → `cert Nd` in yellow.
/// - `delta >= 30d` → `cert Nd` / `cert Ny Mmo` in muted grey (healthy).
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
    let style = if days < 7 {
        theme::error().add_modifier(Modifier::BOLD)
    } else if days < 30 {
        theme::warning()
    } else {
        theme::muted()
    };
    let text = if days >= 365 {
        let years = days / 365;
        let months = (days % 365) / 30;
        if months > 0 {
            format!("cert {years}y {months}mo")
        } else {
            format!("cert {years}y")
        }
    } else {
        format!("cert {days}d")
    };
    Cell::from(Span::styled(text, style))
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
