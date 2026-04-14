//! Ratatui renderer for the Overview tab.
//!
//! Layout 3:
//!
//! ```text
//! ┌─ Processors ──────────────────────────────────────────────────────────────┐
//! │ RUNNING 42   STOPPED 3   INVALID 0   DISABLED 1   THREADS 5              │ ← processor info panel (3 rows)
//! ├─ Nodes (N connected) ─────────────────────────────────────────────────────┤ ← nodes panel (variable, capped)
//! │   node-name   heap  N%   gc Nms/5m   load N.N                            │
//! │   repositories  content  N%   flowfile  N%   provenance  N%              │
//! ├─ Bulletins / min ──────────────┬─ Noisy components ─────────────────────┤ ← bulletins+noisy panel (6 rows)
//! │  sparkline                     │  cnt  source              worst          │
//! ├─ Unhealthy queues ─────────────────────────────────────────────────────────┤ ← unhealthy queues panel (fills rest)
//! │ fill  queue             src → dst                      ffiles             │
//! └────────────────────────────────────────────────────────────────────────────┘
//! ```

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Bar, BarChart, BarGroup, Block, Borders, Cell, Clear, Paragraph, Row, Table,
};

use super::state::{
    BulletinBucket, NoisyComponent, OverviewFocus, OverviewSnapshot, OverviewState,
    SPARKLINE_MINUTES, Severity, UnhealthyQueue,
};
use crate::theme;
use crate::widget::gauge::fill_bar;
use crate::widget::panel::Panel;

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    // Vertical panels. Nodes panel height adapts to the number of nodes
    // (1 row per node + 1 repos row), capped to keep the
    // bulletin/queue panels readable. +2 for the top/bottom border.
    let nodes_height = nodes_zone_height(state);
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),            // processors panel (1 row + 2 border)
            Constraint::Length(nodes_height), // nodes panel
            Constraint::Length(7),            // bulletins/noisy panel (5 content + 2 border)
            Constraint::Fill(1),              // unhealthy queues panel
        ])
        .split(area);

    // Processors panel
    let processors_block = Panel::new(" Processors ").into_block();
    let processors_inner = processors_block.inner(zones[0]);
    frame.render_widget(processors_block, zones[0]);
    render_processor_info_line(frame, processors_inner, state.snapshot.as_ref());

    // Nodes panel — title shows node count when populated
    let total = state.nodes.nodes.len();
    let nodes_title = if total == 0 {
        " Nodes ".to_string()
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
/// Capped so the panel never starves the lower panels.
fn nodes_zone_height(state: &OverviewState) -> u16 {
    let visible_nodes = state.nodes.nodes.len().min(8) as u16;
    // N node rows + 1 repositories row + 2 border rows. Min 4 (loading
    // state needs 1 loading msg + 1 buffer + 2 border).
    let needed = visible_nodes.max(1) + 1 + 2;
    needed.clamp(4, 14)
}

/// Format C — uppercase muted labels with severity-colored values.
fn render_processor_info_line(frame: &mut Frame, area: Rect, snapshot: Option<&OverviewSnapshot>) {
    let line = match snapshot {
        Some(s) => {
            let c = &s.controller;
            Line::from(vec![
                Span::styled("RUNNING", theme::muted()),
                Span::raw(" "),
                Span::styled(c.running.to_string(), theme::success()),
                Span::raw("   "),
                Span::styled("STOPPED", theme::muted()),
                Span::raw(" "),
                Span::styled(c.stopped.to_string(), theme::warning()),
                Span::raw("   "),
                Span::styled("INVALID", theme::muted()),
                Span::raw(" "),
                Span::styled(c.invalid.to_string(), theme::error()),
                Span::raw("   "),
                Span::styled("DISABLED", theme::muted()),
                Span::raw(" "),
                Span::styled(c.disabled.to_string(), theme::muted()),
                Span::raw("   "),
                Span::styled("THREADS", theme::muted()),
                Span::raw(" "),
                Span::styled(c.active_threads.to_string(), theme::accent()),
            ])
        }
        None => Line::from(Span::styled("loading…", theme::muted())),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_nodes_zone(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let total = state.nodes.nodes.len();
    let focused = state.focus == OverviewFocus::Nodes;

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

    let selected = state.nodes.selected;
    // Reserve 1 row at the bottom for the repositories aggregate.
    let visible_node_rows = area.height.saturating_sub(1) as usize;
    let scroll_offset = if visible_node_rows == 0 {
        0
    } else if selected >= visible_node_rows {
        selected + 1 - visible_node_rows
    } else {
        0
    };

    let mut rows: Vec<Row> = state
        .nodes
        .nodes
        .iter()
        .skip(scroll_offset)
        .take(visible_node_rows)
        .enumerate()
        .map(|(idx, node)| {
            let row_style = if focused && idx == selected.saturating_sub(scroll_offset) {
                theme::cursor_row()
            } else {
                Style::default()
            };
            node_to_row(node).style(row_style)
        })
        .collect();

    // Cluster-aggregate repositories row (not selectable).
    let repos = &state.repositories_summary;
    rows.push(Row::new(vec![
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
    ]));

    let table = Table::new(
        rows,
        [
            Constraint::Fill(1),
            Constraint::Length(15),
            Constraint::Length(15),
            Constraint::Length(14),
        ],
    );
    frame.render_widget(table, area);
}

/// Convert a `NodeHealthRow` to a ratatui `Row` with four styled cells:
/// address | heap bar+% | gc | load bar+value.
fn node_to_row(node: &crate::client::health::NodeHealthRow) -> Row<'static> {
    let gc_style = match node.gc_delta {
        Some(d) if d > 5 => theme::error(),
        Some(_) => theme::warning(),
        None => Style::default(),
    };
    let gc_str = match node.gc_delta {
        Some(d) => format!("{:>4}ms (+{})", node.gc_millis, d),
        None => format!("{:>4}ms", node.gc_millis),
    };
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
    let heap_style = health_severity_style(node.heap_severity);
    let heap_bar = fill_bar(5, node.heap_percent);
    let load_bar = match (node.load_average, node.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            format!("{:.2}", (l / cpus as f64).min(9.99))
        }
        _ => "    ".to_string(),
    };
    Row::new(vec![
        Cell::from(Span::styled(
            format!("  {}", node.node_address),
            theme::accent(),
        )),
        Cell::from(Line::from(vec![
            Span::raw("heap "),
            Span::styled(heap_bar, heap_style),
            Span::raw(" "),
            Span::styled(format!("{:>3}%", node.heap_percent), heap_style),
        ])),
        Cell::from(Line::from(vec![
            Span::raw("gc "),
            Span::styled(gc_str, gc_style),
        ])),
        Cell::from(Line::from(vec![
            Span::raw("load "),
            Span::styled(load_bar, load_style),
            Span::raw(" "),
            Span::styled(load_str, load_style),
        ])),
    ])
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
        let scroll_offset = if visible_rows == 0 {
            0
        } else if selected >= visible_rows {
            selected + 1 - visible_rows
        } else {
            0
        };
        let selected_in_window = selected.saturating_sub(scroll_offset);
        noisy
            .iter()
            .skip(scroll_offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, n)| {
                let sev_style = severity_style(n.max_severity);
                let row_style = if focused && idx == selected_in_window {
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
        let scroll_offset = if visible_rows == 0 {
            0
        } else if selected >= visible_rows {
            selected + 1 - visible_rows
        } else {
            0
        };
        let selected_in_window = selected.saturating_sub(scroll_offset);
        queues
            .iter()
            .skip(scroll_offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, q)| {
                let style = fill_style(q.fill_percent);
                let row_style = if focused && idx == selected_in_window {
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

fn fill_style(percent: u32) -> Style {
    match percent {
        0..=49 => theme::success(),
        50..=79 => theme::warning(),
        _ => theme::error(),
    }
}

/// Convert a `client::health::Severity` (Green/Yellow/Red) into a theme style.
fn health_severity_style(s: crate::client::health::Severity) -> Style {
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

pub fn render_node_detail_modal(
    frame: &mut Frame,
    area: Rect,
    row: &crate::client::health::NodeHealthRow,
) {
    let gc_rows = row.gc.len() as u16;
    // Right pane: "GC" header + gc rows + separator + "Repositories" header + 3 repo rows.
    let right_height = 1 + gc_rows + 1 + 1 + 3;
    let left_height = 4u16; // heap, load, threads, uptime
    let content_height = right_height.max(left_height);
    let popup_height = content_height + 2; // +2 for outer border

    let popup = center_rect(80, popup_height, area);
    frame.render_widget(Clear, popup);

    let outer = Panel::new(format!(" {} ", row.node_address)).into_block();
    let inner = outer.inner(popup);
    frame.render_widget(outer, popup);

    // 40% left (system summary) / 60% right (GC + repos).
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(40), Constraint::Percentage(60)])
        .split(inner);

    render_node_system_summary(frame, h[0], row);
    render_node_gc_and_repos(frame, h[1], row, gc_rows);
}

fn render_node_system_summary(
    frame: &mut Frame,
    area: Rect,
    row: &crate::client::health::NodeHealthRow,
) {
    let heap_style = health_severity_style(row.heap_severity);
    let heap_bar = fill_bar(5, row.heap_percent);

    let (load_str, load_style, load_bar) = match (row.load_average, row.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            let ratio = l / cpus as f64;
            let style = if ratio >= 2.0 {
                theme::error()
            } else if ratio >= 1.0 {
                theme::warning()
            } else {
                theme::success()
            };
            (
                format!("{l:.1}"),
                style,
                format!("{:>5.2}", ratio.min(9.99)),
            )
        }
        (Some(l), _) => (format!("{l:.1}"), Style::default(), "     ".to_string()),
        (None, _) => ("\u{2014}".to_string(), theme::muted(), "     ".to_string()),
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("  heap     ", theme::muted()),
            Span::styled(heap_bar, heap_style),
            Span::raw("  "),
            Span::styled(format!("{:>3}%", row.heap_percent), heap_style),
        ]),
        Line::from(vec![
            Span::styled("  load     ", theme::muted()),
            Span::styled(load_bar, load_style),
            Span::raw("  "),
            Span::styled(load_str, load_style),
        ]),
        Line::from(vec![
            Span::styled("  threads  ", theme::muted()),
            Span::raw(row.total_threads.to_string()),
        ]),
        Line::from(vec![
            Span::styled("  uptime   ", theme::muted()),
            Span::raw(row.uptime.clone()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_node_gc_and_repos(
    frame: &mut Frame,
    area: Rect,
    row: &crate::client::health::NodeHealthRow,
    gc_rows: u16,
) {
    // Vertical split: GC section | separator | Repos section.
    let gc_section_h = 1 + gc_rows; // "GC" header row + data rows
    let repos_section_h = 1 + 3; // "Repositories" header + 3 repo type rows
    let v = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(gc_section_h),
            Constraint::Length(1),
            Constraint::Length(repos_section_h),
        ])
        .split(area);

    // GC table.
    let gc_table_rows: Vec<Row> = row
        .gc
        .iter()
        .map(|g| {
            Row::new(vec![
                Cell::from(format!("  {}", g.name)),
                Cell::from(format!("{}", g.collection_count)),
                Cell::from(format!("{}ms", g.collection_millis)),
            ])
        })
        .collect();
    let gc_table = Table::new(
        gc_table_rows,
        [
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["GC", "count", "millis"]).style(theme::bold()));
    frame.render_widget(gc_table, v[0]);

    // Separator line.
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_style(theme::border_dim()),
        v[1],
    );

    // Repositories table.
    let content_pct = node_avg_pct(&row.content_repos);
    let flowfile_pct = row
        .flowfile_repo
        .as_ref()
        .map(|r| r.utilization_percent)
        .unwrap_or(0);
    let provenance_pct = node_avg_pct(&row.provenance_repos);

    let repo_rows: Vec<Row> = [
        ("content", content_pct),
        ("flowfile", flowfile_pct),
        ("provenance", provenance_pct),
    ]
    .iter()
    .map(|(name, pct)| {
        let style = fill_style(*pct);
        Row::new(vec![
            Cell::from(format!("  {}", name)),
            Cell::from(Span::styled(fill_bar(4, *pct), style)),
            Cell::from(Span::styled(format!("{:>3}%", pct), style)),
        ])
    })
    .collect();
    let repos_table = Table::new(
        repo_rows,
        [
            Constraint::Fill(1),
            Constraint::Length(6),
            Constraint::Length(5),
        ],
    )
    .header(Row::new(vec!["Repositories", "", ""]).style(theme::bold()));
    frame.render_widget(repos_table, v[2]);
}

/// Average `utilization_percent` across a slice of repos.
/// Returns 0 for an empty slice.
fn node_avg_pct(repos: &[crate::client::health::RepoUsage]) -> u32 {
    if repos.is_empty() {
        return 0;
    }
    let sum: u32 = repos.iter().map(|r| r.utilization_percent).sum();
    sum / repos.len() as u32
}

fn center_rect(pct_x: u16, height: u16, area: Rect) -> Rect {
    let w = area.width * pct_x / 100;
    let x = area.x + (area.width.saturating_sub(w)) / 2;
    let y = area.y + (area.height.saturating_sub(height)) / 2;
    Rect {
        x,
        y,
        width: w,
        height,
    }
}

#[cfg(test)]
mod tests {
    use crate::client::{
        AboutSnapshot, BulletinBoardSnapshot, BulletinSnapshot, ControllerStatusSnapshot,
        QueueSnapshot, RootPgStatusSnapshot,
    };
    use crate::event::{OverviewPayload, OverviewPgStatusPayload};
    use crate::view::overview::state::{OverviewState, apply_payload};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, UNIX_EPOCH};

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

    #[test]
    fn snapshot_empty_state() {
        let state = OverviewState::new();
        insta::assert_snapshot!("overview_empty", render_to_string(&state));
    }

    #[test]
    fn snapshot_healthy_cluster() {
        let mut state = OverviewState::new();
        let payload = OverviewPayload::PgStatus(OverviewPgStatusPayload {
            about: AboutSnapshot {
                version: "2.8.0".into(),
                title: "NiFi".into(),
            },
            controller: ControllerStatusSnapshot {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 5,
                flow_files_queued: 120,
                bytes_queued: 4096,
            },
            root_pg: RootPgStatusSnapshot {
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
            },
            bulletin_board: BulletinBoardSnapshot::default(),
            fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
        });
        apply_payload(&mut state, payload);
        insta::assert_snapshot!("overview_healthy", render_to_string(&state));
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
        let bulletins = (0..6)
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
        let payload = OverviewPayload::PgStatus(OverviewPgStatusPayload {
            about: AboutSnapshot {
                version: "2.8.0".into(),
                title: "NiFi".into(),
            },
            controller: ControllerStatusSnapshot {
                running: 20,
                stopped: 10,
                invalid: 2,
                disabled: 0,
                active_threads: 17,
                flow_files_queued: 50_000,
                bytes_queued: 8_000_000,
            },
            root_pg: RootPgStatusSnapshot {
                flow_files_queued: 50_000,
                bytes_queued: 8_000_000,
                connections: queues,
            },
            bulletin_board: BulletinBoardSnapshot { bulletins },
            fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
        });
        apply_payload(&mut state, payload);
        insta::assert_snapshot!("overview_unhealthy", render_to_string(&state));
    }

    #[test]
    fn snapshot_with_nodes_populated() {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use std::time::Instant;

        let mut state = OverviewState::new();

        // First a PG-status payload so the processor info line has data.
        apply_payload(
            &mut state,
            OverviewPayload::PgStatus(OverviewPgStatusPayload {
                about: AboutSnapshot {
                    version: "2.9.0".into(),
                    title: "NiFi".into(),
                },
                controller: ControllerStatusSnapshot {
                    running: 42,
                    stopped: 3,
                    invalid: 0,
                    disabled: 1,
                    active_threads: 5,
                    flow_files_queued: 120,
                    bytes_queued: 4096,
                },
                root_pg: RootPgStatusSnapshot {
                    flow_files_queued: 120,
                    bytes_queued: 4096,
                    connections: vec![],
                },
                bulletin_board: BulletinBoardSnapshot::default(),
                fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
            }),
        );

        // Then a SystemDiag payload with two nodes. Fixture copied from the
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
        apply_payload(
            &mut state,
            OverviewPayload::SystemDiag(SystemDiagSnapshot {
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
            }),
        );

        insta::assert_snapshot!("overview_with_nodes", render_to_string(&state));
    }

    #[test]
    fn snapshot_node_detail_modal() {
        use crate::client::health::{GcSnapshot, NodeHealthRow, RepoUsage, Severity as HSev};
        let backend = TestBackend::new(100, 25);
        let mut term = Terminal::new(backend).unwrap();
        let row = NodeHealthRow {
            node_address: "node1:8080".into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            heap_percent: 52,
            heap_severity: HSev::Green,
            gc_collection_count: 12,
            gc_delta: Some(2),
            gc_millis: 170,
            load_average: Some(1.5),
            available_processors: Some(4),
            uptime: "2h 30m".into(),
            total_threads: 50,
            gc: vec![
                GcSnapshot {
                    name: "G1 Young".into(),
                    collection_count: 10,
                    collection_millis: 50,
                },
                GcSnapshot {
                    name: "G1 Old".into(),
                    collection_count: 2,
                    collection_millis: 120,
                },
            ],
            content_repos: vec![RepoUsage {
                identifier: "c".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "f".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "p".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        };
        term.draw(|f| super::render_node_detail_modal(f, f.area(), &row))
            .unwrap();
        insta::assert_snapshot!("node_detail_modal", format!("{}", term.backend()));
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

        apply_payload(
            &mut state,
            OverviewPayload::SystemDiag(SystemDiagSnapshot {
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
            }),
        );
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
        use crate::client::{
            AboutSnapshot, BulletinBoardSnapshot, ControllerStatusSnapshot, RootPgStatusSnapshot,
        };
        use crate::event::{OverviewPayload, OverviewPgStatusPayload};
        use crate::view::overview::state::{NoisyComponent, Severity as OvSev, apply_payload};
        use std::time::{Duration, UNIX_EPOCH};

        let mut state = OverviewState::new();
        state.focus = crate::view::overview::state::OverviewFocus::Noisy;

        // Populate with a PG-status payload so the layout renders properly.
        apply_payload(
            &mut state,
            OverviewPayload::PgStatus(OverviewPgStatusPayload {
                about: AboutSnapshot {
                    version: "2.8.0".into(),
                    title: "NiFi".into(),
                },
                controller: ControllerStatusSnapshot {
                    running: 1,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                    active_threads: 0,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                },
                root_pg: RootPgStatusSnapshot::default(),
                bulletin_board: BulletinBoardSnapshot::default(),
                fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
            }),
        );

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
        use crate::client::{
            AboutSnapshot, BulletinBoardSnapshot, ControllerStatusSnapshot, QueueSnapshot,
            RootPgStatusSnapshot,
        };
        use crate::event::{OverviewPayload, OverviewPgStatusPayload};
        use crate::view::overview::state::apply_payload;
        use std::time::{Duration, UNIX_EPOCH};

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

        apply_payload(
            &mut state,
            OverviewPayload::PgStatus(OverviewPgStatusPayload {
                about: AboutSnapshot {
                    version: "2.8.0".into(),
                    title: "NiFi".into(),
                },
                controller: ControllerStatusSnapshot {
                    running: 1,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                    active_threads: 0,
                    flow_files_queued: 0,
                    bytes_queued: 0,
                },
                root_pg: RootPgStatusSnapshot {
                    flow_files_queued: 0,
                    bytes_queued: 0,
                    connections,
                },
                bulletin_board: BulletinBoardSnapshot::default(),
                fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
            }),
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
}
