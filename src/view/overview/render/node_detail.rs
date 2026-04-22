//! Overview tab node detail modal — four-quadrant dashboard.
//!
//! Layout:
//!
//! ```text
//! ┌ {node_address} ─────────────────────────────────────────────────────────────┐
//! │  [BADGE] STATUS    Roles…                          heartbeat   Ns ago       │
//! │  node_id …                                joined   YYYY-MM-DD HH:MM:SS      │
//! ├── Resources ─────────────────────┬── Repositories ───────────────────────────│
//! │   heap / load / threads / …      │  content /path  bar  %  used/total GB    │
//! │                                  │  flowfile …                              │
//! ├── Events ────────────────────────┼── GC ─────────────────────────────────────│
//! │   HH:MM:SS  CATEGORY             │  collector   count   millis              │
//! └──────────────────────────────────────────────────────────────────────────────┘
//! ```
//!
//! Standalone (`row.cluster == None`): header shows `standalone node`, the
//! Events quadrant is hidden, and GC expands to fill the bottom row.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use super::health_severity_style;
use crate::client::health::{ClusterNodeEvent, ClusterNodeStatus, NodeHealthRow, RepoUsage};
use crate::theme;
use crate::timestamp::{format_age, format_bytes};
use crate::widget::gauge::fill_bar;
use crate::widget::panel::Panel;

pub fn render_node_detail_modal(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    // Popup: 90% width, min(22, area.height - 2) rows, clamped >= 10.
    let height = 22u16.min(area.height.saturating_sub(2)).max(10);
    let popup = center_rect(90, height, area);
    frame.render_widget(Clear, popup);
    let outer = Panel::new(format!(" {} ", row.node_address)).into_block();
    let inner = outer.inner(popup);
    frame.render_widget(outer, popup);

    // Vertical: header (2) / separator (1) / top row / separator (1) / bottom row.
    // We always reserve the bottom row; when a node has no events AND
    // has no GC data the bottom renders an empty "GC" stub — that's
    // visually correct and avoids jumpy layout.
    let has_events = row
        .cluster
        .as_ref()
        .map(|c| !c.events.is_empty())
        .unwrap_or(false);
    let show_bottom = !row.gc.is_empty() || has_events;
    let bottom_h = if show_bottom {
        inner.height.saturating_sub(4) / 2
    } else {
        0
    };
    let top_h = inner.height.saturating_sub(4 + bottom_h);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(top_h),
            Constraint::Length(1),
            Constraint::Length(bottom_h),
        ])
        .split(inner);

    render_header(frame, chunks[0], row);
    render_separator(frame, chunks[1]);
    render_top_row(frame, chunks[2], row);
    if show_bottom {
        render_separator(frame, chunks[3]);
        render_bottom_row(frame, chunks[4], row);
    }
}

fn render_separator(frame: &mut Frame, area: Rect) {
    frame.render_widget(
        Block::new()
            .borders(Borders::TOP)
            .border_style(theme::border_dim()),
        area,
    );
}

fn render_header(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let line1 = match &row.cluster {
        Some(c) => {
            let badge = crate::widget::node_badge::node_badge(c);
            let status_word = status_word(c.status);
            let roles = roles_label(c.is_primary, c.is_coordinator);
            let hb_text = format_age(c.heartbeat_age);
            Line::from(vec![
                Span::raw("  "),
                badge,
                Span::raw(" "),
                Span::styled(status_word, status_word_style(c.status)),
                Span::raw("    "),
                Span::raw(roles),
                Span::raw("            "),
                Span::styled("heartbeat ", theme::muted()),
                Span::styled(hb_text, heartbeat_age_style(c.heartbeat_age)),
                Span::styled(" ago", theme::muted()),
            ])
        }
        None => Line::from(vec![
            Span::raw("  "),
            Span::styled(
                "standalone node",
                theme::muted().add_modifier(Modifier::ITALIC),
            ),
        ]),
    };
    let line2 = match &row.cluster {
        Some(c) => Line::from(vec![
            Span::styled("  node_id  ", theme::muted()),
            Span::raw(c.node_id.clone()),
            Span::raw("           "),
            Span::styled("joined  ", theme::muted()),
            Span::raw(
                c.node_start_iso
                    .clone()
                    .unwrap_or_else(|| "\u{2014}".into()),
            ),
        ]),
        None => Line::from(""),
    };
    frame.render_widget(Paragraph::new(vec![line1, line2]), area);
}

fn status_word(status: ClusterNodeStatus) -> &'static str {
    match status {
        ClusterNodeStatus::Connected => "CONNECTED",
        ClusterNodeStatus::Connecting => "CONNECTING",
        ClusterNodeStatus::Disconnected => "DISCONNECTED",
        ClusterNodeStatus::Disconnecting => "DISCONNECTING",
        ClusterNodeStatus::Offloading => "OFFLOADING",
        ClusterNodeStatus::Offloaded => "OFFLOADED",
        ClusterNodeStatus::Other => "UNKNOWN",
    }
}

fn status_word_style(status: ClusterNodeStatus) -> Style {
    match status {
        ClusterNodeStatus::Connected => theme::success().add_modifier(Modifier::BOLD),
        ClusterNodeStatus::Connecting | ClusterNodeStatus::Disconnecting => theme::warning(),
        ClusterNodeStatus::Offloading | ClusterNodeStatus::Offloaded => theme::warning(),
        ClusterNodeStatus::Disconnected => theme::error().add_modifier(Modifier::BOLD),
        ClusterNodeStatus::Other => theme::muted(),
    }
}

fn roles_label(primary: bool, coord: bool) -> String {
    match (primary, coord) {
        (true, true) => "Primary \u{00b7} Coordinator".into(),
        (true, false) => "Primary".into(),
        (false, true) => "Coordinator".into(),
        (false, false) => String::new(),
    }
}

fn heartbeat_age_style(age: Option<std::time::Duration>) -> Style {
    match age {
        None => theme::muted(),
        Some(d) if d.as_secs() < 30 => theme::muted(),
        Some(d) if d.as_secs() < 120 => theme::warning(),
        Some(_) => theme::error(),
    }
}

fn render_top_row(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(45), Constraint::Percentage(55)])
        .split(area);
    render_resources(frame, h[0], row);
    render_repositories(frame, h[1], row);
}

fn render_bottom_row(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let has_events = row
        .cluster
        .as_ref()
        .map(|c| !c.events.is_empty())
        .unwrap_or(false);
    if row.cluster.is_none() || !has_events {
        // Standalone or cluster-connected-but-no-events: GC fills the row.
        render_gc(frame, area, row);
        return;
    }
    let h = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);
    render_events(frame, h[0], row);
    render_gc(frame, h[1], row);
}

fn render_resources(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let heap_style = health_severity_style(row.heap_severity);
    let heap_bar = fill_bar(5, row.heap_percent);

    let (load_str, load_style) = match (row.load_average, row.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            let ratio = l / cpus as f64;
            let style = if ratio >= 2.0 {
                theme::error()
            } else if ratio >= 1.0 {
                theme::warning()
            } else {
                theme::success()
            };
            (format!("{l:.2}   ({ratio:.2} / core)"), style)
        }
        (Some(l), _) => (format!("{l:.2}"), Style::default()),
        (None, _) => ("\u{2014}".to_string(), theme::muted()),
    };

    let nifi_threads = row
        .cluster
        .as_ref()
        .map(|c| c.active_thread_count.to_string())
        .unwrap_or_else(|| "\u{2014}".into());
    let queued = row
        .cluster
        .as_ref()
        .map(|c| {
            format!(
                "{} ff \u{00b7} {}",
                c.flow_files_queued,
                format_bytes(c.bytes_queued)
            )
        })
        .unwrap_or_else(|| "\u{2014}".into());

    let lines = vec![
        Line::from(vec![Span::styled("  Resources", theme::bold())]),
        Line::from(vec![
            Span::styled("    heap     ", theme::muted()),
            Span::styled(heap_bar, heap_style),
            Span::raw("  "),
            Span::styled(format!("{:>3}%", row.heap_percent), heap_style),
        ]),
        Line::from(vec![
            Span::styled("    load     ", theme::muted()),
            Span::styled(load_str, load_style),
        ]),
        Line::from(vec![
            Span::styled("    threads  ", theme::muted()),
            Span::raw(format!("JVM {}   NiFi {}", row.total_threads, nifi_threads)),
        ]),
        Line::from(vec![
            Span::styled("    queued   ", theme::muted()),
            Span::raw(queued),
        ]),
        Line::from(vec![
            Span::styled("    uptime   ", theme::muted()),
            Span::raw(row.uptime.clone()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_repositories(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let mut rows: Vec<Row> = Vec::new();
    rows.extend(row.content_repos.iter().map(|r| repo_row("content", r)));
    if let Some(ff) = row.flowfile_repo.as_ref() {
        rows.push(repo_row("flowfile", ff));
    }
    rows.extend(
        row.provenance_repos
            .iter()
            .map(|r| repo_row("provenance", r)),
    );

    let header = Row::new(vec![Cell::from(Span::styled(
        "  Repositories",
        theme::bold(),
    ))])
    .style(theme::bold());
    let table = Table::new(
        rows,
        [
            Constraint::Length(12),
            Constraint::Fill(1),
            Constraint::Length(6),
            Constraint::Length(5),
            Constraint::Length(17),
        ],
    )
    .header(header);
    frame.render_widget(table, area);
}

fn repo_row(kind: &str, r: &RepoUsage) -> Row<'static> {
    let pct = r.utilization_percent;
    let style = match pct {
        0..=49 => theme::success(),
        50..=79 => theme::warning(),
        _ => theme::error(),
    };
    Row::new(vec![
        Cell::from(format!("  {kind}")),
        Cell::from(r.identifier.clone()),
        Cell::from(Span::styled(fill_bar(4, pct), style)),
        Cell::from(Span::styled(format!("{pct:>3}%"), style)),
        Cell::from(format!(
            "{} / {}",
            format_bytes(r.used_bytes),
            format_bytes(r.total_bytes),
        )),
    ])
}

fn render_events(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let Some(cluster) = row.cluster.as_ref() else {
        return;
    };
    let header = Paragraph::new(Line::from(Span::styled("  Events", theme::bold())));
    let h = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1)])
        .split(area);
    frame.render_widget(header, h[0]);

    let rows: Vec<Row> = cluster
        .events
        .iter()
        .take(h[1].height as usize)
        .map(|ev| {
            let ts = event_hms(ev);
            let cat = ev.category.clone().unwrap_or_default();
            let style = event_category_style(&cat);
            Row::new(vec![
                Cell::from(format!("  {ts}")),
                Cell::from(Span::styled(cat, style)),
            ])
        })
        .collect();
    if rows.is_empty() {
        frame.render_widget(
            Paragraph::new(Span::styled("  no events recorded", theme::muted())),
            h[1],
        );
        return;
    }
    let table = Table::new(rows, [Constraint::Length(10), Constraint::Fill(1)]);
    frame.render_widget(table, h[1]);
}

fn event_hms(ev: &ClusterNodeEvent) -> String {
    crate::timestamp::parse_nifi_timestamp(&ev.timestamp_iso)
        .map(|dt| format!("{:02}:{:02}:{:02}", dt.hour(), dt.minute(), dt.second()))
        .unwrap_or_else(|| "\u{2014}".into())
}

fn event_category_style(category: &str) -> Style {
    match category {
        "DISCONNECTED" | "OFFLOAD_REQUESTED" | "OFFLOADED" => theme::error(),
        "CONNECTED" | "CONNECTION_REQUESTED" => theme::success(),
        "HEARTBEAT_RECEIVED" => theme::muted(),
        _ => Style::default(),
    }
}

fn render_gc(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
    let header = Row::new(vec![
        Cell::from(Span::styled("  GC", theme::bold())),
        Cell::from(Span::styled("collector", theme::bold())),
        Cell::from(Span::styled("count", theme::bold())),
        Cell::from(Span::styled("millis", theme::bold())),
    ]);
    let rows: Vec<Row> = row
        .gc
        .iter()
        .map(|g| {
            Row::new(vec![
                Cell::from(""),
                Cell::from(format!("  {}", g.name)),
                Cell::from(g.collection_count.to_string()),
                Cell::from(format!("{}ms", g.collection_millis)),
            ])
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Fill(1),
            Constraint::Length(8),
            Constraint::Length(8),
        ],
    )
    .header(header);
    frame.render_widget(table, area);
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
