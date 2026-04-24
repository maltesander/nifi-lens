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
    render_node_detail_modal_at(frame, area, row, time::OffsetDateTime::now_utc());
}

pub(crate) fn render_node_detail_modal_at(
    frame: &mut Frame,
    area: Rect,
    row: &NodeHealthRow,
    now: time::OffsetDateTime,
) {
    // Popup: 90% width, min(26, area.height - 2) rows, clamped >= 10.
    let height = 26u16.min(area.height.saturating_sub(2)).max(10);
    let popup = center_rect(90, height, area);
    frame.render_widget(Clear, popup);
    let outer = Panel::new(format!(" {} ", row.node_address)).into_block();
    let inner = outer.inner(popup);
    frame.render_widget(outer, popup);

    // cert_rows: number of extra lines the cert sub-block occupies in the header.
    let cert_rows = match row.tls_cert.as_ref() {
        None => 1u16,
        Some(Ok(chain)) => chain.entries.len() as u16,
        Some(Err(_)) => 1u16,
    };

    // Vertical: header (2 + cert_rows) / separator (1) / top row / separator (1) / bottom row.
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
        inner.height.saturating_sub(4 + cert_rows) / 2
    } else {
        0
    };
    let top_h = inner.height.saturating_sub(4 + cert_rows + bottom_h);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2 + cert_rows),
            Constraint::Length(1),
            Constraint::Length(top_h),
            Constraint::Length(1),
            Constraint::Length(bottom_h),
        ])
        .split(inner);

    render_header(frame, chunks[0], row, now);
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

fn render_header(frame: &mut Frame, area: Rect, row: &NodeHealthRow, now: time::OffsetDateTime) {
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

    let cert_rows = match row.tls_cert.as_ref() {
        None => 1u16,
        Some(Ok(chain)) => chain.entries.len() as u16,
        Some(Err(_)) => 1u16,
    };

    let header_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Length(cert_rows)])
        .split(area);
    frame.render_widget(Paragraph::new(vec![line1, line2]), header_chunks[0]);
    render_cert_block(frame, header_chunks[1], row, now);
}

fn render_cert_block(
    frame: &mut Frame,
    area: Rect,
    row: &NodeHealthRow,
    now: time::OffsetDateTime,
) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    match row.tls_cert.as_ref() {
        None => {
            lines.push(Line::from(vec![
                Span::styled("  cert     ", theme::muted()),
                Span::styled("\u{2014} fetching", theme::muted()),
            ]));
        }
        Some(Err(err)) => {
            lines.push(Line::from(vec![
                Span::styled("  cert     ", theme::muted()),
                Span::styled(
                    format!("probe failed: {}", probe_error_msg(err)),
                    theme::warning(),
                ),
            ]));
        }
        Some(Ok(chain)) => {
            for (i, entry) in chain.entries.iter().enumerate() {
                let label_cell = if i == 0 { "  cert     " } else { "           " };
                let kind = if entry.is_leaf { "leaf" } else { "CA  " };
                let (days_text, days_style) = days_until_style(entry.not_after, now);
                let date_str = entry
                    .not_after
                    .format(&time::format_description::well_known::Iso8601::DATE)
                    .unwrap_or_else(|_| "\u{2014}".into());
                let cn = entry
                    .subject_cn
                    .clone()
                    .unwrap_or_else(|| "\u{2014}".into());
                lines.push(Line::from(vec![
                    Span::styled(label_cell, theme::muted()),
                    Span::raw(format!("{kind}  ")),
                    Span::raw(format!("{date_str}  ")),
                    Span::styled(days_text, days_style),
                    Span::raw("          "),
                    Span::styled(cn, theme::muted()),
                ]));
            }
        }
    }
    frame.render_widget(Paragraph::new(lines), area);
}

fn probe_error_msg(err: &crate::client::tls_cert::TlsProbeError) -> String {
    use crate::client::tls_cert::TlsProbeError;
    match err {
        TlsProbeError::Connect(s) => format!("connect: {s}"),
        TlsProbeError::Handshake(s) => format!("handshake: {s}"),
        TlsProbeError::NoCerts => "server sent no certs".into(),
        TlsProbeError::ParseCert(s) => format!("parse: {s}"),
    }
}

/// Format a days-until string and severity style.
fn days_until_style(not_after: time::OffsetDateTime, now: time::OffsetDateTime) -> (String, Style) {
    let delta = not_after - now;
    if delta.is_negative() {
        return (
            "EXPIRED".into(),
            theme::error().add_modifier(Modifier::BOLD),
        );
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
            format!("{years}y {months}mo")
        } else {
            format!("{years}y")
        }
    } else {
        format!("{days}d")
    };
    (text, style)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::health::{
        ClusterMembership, ClusterNodeEvent, ClusterNodeStatus, GcSnapshot, NodeHealthRow,
        RepoUsage, Severity as HSev,
    };
    use crate::client::tls_cert::{CertEntry, NodeCertChain, TlsProbeError};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn fixed_now() -> time::OffsetDateTime {
        time::macros::datetime!(2026-04-24 00:00 UTC)
    }

    #[test]
    fn snapshot_node_detail_modal_clustered() {
        let backend = TestBackend::new(110, 28);
        let mut term = Terminal::new(backend).unwrap();
        // 2-entry chain: leaf expires in ~2.5y, CA expires in 12d (yellow).
        let leaf_not_after = time::macros::datetime!(2028-11-22 00:00 UTC);
        let ca_not_after = time::macros::datetime!(2026-05-06 00:00 UTC);
        let row = NodeHealthRow {
            node_address: "node2.nifi:8443".into(),
            heap_used_bytes: 620 * crate::bytes::MIB,
            heap_max_bytes: crate::bytes::HEAP_1_GIB,
            heap_percent: 60,
            heap_severity: HSev::Yellow,
            gc_collection_count: 12,
            gc_delta: Some(2),
            gc_millis: 170,
            load_average: Some(1.24),
            available_processors: Some(4),
            uptime: "2h 30m".into(),
            total_threads: 104,
            gc: vec![
                GcSnapshot {
                    name: "G1 Young Generation".into(),
                    collection_count: 10,
                    collection_millis: 50,
                },
                GcSnapshot {
                    name: "G1 Old Generation".into(),
                    collection_count: 2,
                    collection_millis: 120,
                },
            ],
            content_repos: vec![
                RepoUsage {
                    identifier: "/data/c1".into(),
                    used_bytes: 118 * 1024_u64.pow(3),
                    total_bytes: 190 * 1024_u64.pow(3),
                    free_bytes: 72 * 1024_u64.pow(3),
                    utilization_percent: 62,
                },
                RepoUsage {
                    identifier: "/data/c2".into(),
                    used_bytes: 78 * 1024_u64.pow(3),
                    total_bytes: 190 * 1024_u64.pow(3),
                    free_bytes: 112 * 1024_u64.pow(3),
                    utilization_percent: 41,
                },
            ],
            flowfile_repo: Some(RepoUsage {
                identifier: "/data/ff".into(),
                used_bytes: 9 * 1024_u64.pow(3),
                total_bytes: 50 * 1024_u64.pow(3),
                free_bytes: 41 * 1024_u64.pow(3),
                utilization_percent: 18,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "/data/p1".into(),
                used_bytes: 124 * 1024_u64.pow(3),
                total_bytes: 200 * 1024_u64.pow(3),
                free_bytes: 76 * 1024_u64.pow(3),
                utilization_percent: 62,
            }],
            cluster: Some(ClusterMembership {
                node_id: "5f2b8a17-1234-1234-1234-c394e97c3000".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: true,
                is_coordinator: true,
                heartbeat_age: Some(std::time::Duration::from_secs(3)),
                node_start_iso: Some("04/22/2026 09:12:04 UTC".into()),
                active_thread_count: 42,
                flow_files_queued: 1234,
                bytes_queued: 456 * crate::bytes::MIB,
                events: vec![
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 10:14:03 UTC".into(),
                        category: Some("CONNECTED".into()),
                        message: String::new(),
                    },
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 10:13:44 UTC".into(),
                        category: Some("HEARTBEAT_RECEIVED".into()),
                        message: String::new(),
                    },
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 09:12:04 UTC".into(),
                        category: Some("DISCONNECTED".into()),
                        message: String::new(),
                    },
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 09:11:58 UTC".into(),
                        category: Some("CONNECTION_REQUESTED".into()),
                        message: String::new(),
                    },
                ],
            }),
            tls_cert: Some(Ok(NodeCertChain {
                entries: vec![
                    CertEntry {
                        subject_cn: Some("node2.nifi".into()),
                        not_after: leaf_not_after,
                        is_leaf: true,
                    },
                    CertEntry {
                        subject_cn: Some("NiFi CA".into()),
                        not_after: ca_not_after,
                        is_leaf: false,
                    },
                ],
            })),
        };
        term.draw(|f| render_node_detail_modal_at(f, f.area(), &row, fixed_now()))
            .unwrap();
        insta::assert_snapshot!("node_detail_modal_clustered", format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_node_detail_modal_standalone() {
        let backend = TestBackend::new(110, 28);
        let mut term = Terminal::new(backend).unwrap();
        let row = NodeHealthRow {
            node_address: "nifi:8443".into(),
            heap_used_bytes: crate::bytes::HEAP_512_MIB,
            heap_max_bytes: crate::bytes::HEAP_1_GIB,
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
            cluster: None,
            tls_cert: None, // exercises "— fetching" path
        };
        term.draw(|f| render_node_detail_modal_at(f, f.area(), &row, fixed_now()))
            .unwrap();
        insta::assert_snapshot!(
            "node_detail_modal_standalone",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn snapshot_node_detail_modal_disconnected() {
        let backend = TestBackend::new(110, 28);
        let mut term = Terminal::new(backend).unwrap();
        let row = NodeHealthRow {
            node_address: "node3.nifi:8443".into(),
            heap_used_bytes: 0,
            heap_max_bytes: crate::bytes::HEAP_1_GIB,
            heap_percent: 0,
            heap_severity: HSev::Green,
            gc_collection_count: 0,
            gc_delta: None,
            gc_millis: 0,
            load_average: None,
            available_processors: None,
            uptime: "\u{2014}".into(),
            total_threads: 0,
            gc: vec![],
            content_repos: vec![],
            flowfile_repo: None,
            provenance_repos: vec![],
            cluster: Some(ClusterMembership {
                node_id: "8a2f3b1c-...-aaaa".into(),
                status: ClusterNodeStatus::Disconnected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_age: Some(std::time::Duration::from_secs(300)),
                node_start_iso: Some("04/22/2026 08:00:00 UTC".into()),
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 10:05:00 UTC".into(),
                        category: Some("DISCONNECTED".into()),
                        message: String::new(),
                    },
                    ClusterNodeEvent {
                        timestamp_iso: "04/22/2026 09:59:00 UTC".into(),
                        category: Some("HEARTBEAT_RECEIVED".into()),
                        message: String::new(),
                    },
                ],
            }),
            tls_cert: Some(Err(TlsProbeError::Connect("refused".into()))), // exercises probe-failed path
        };
        term.draw(|f| render_node_detail_modal_at(f, f.area(), &row, fixed_now()))
            .unwrap();
        insta::assert_snapshot!(
            "node_detail_modal_disconnected",
            format!("{}", term.backend())
        );
    }
}
