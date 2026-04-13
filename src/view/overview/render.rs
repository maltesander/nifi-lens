//! Ratatui renderer for the Overview tab.
//!
//! Layout 3:
//!
//! ```text
//! ┌─ Overview ────────────────────────────────────────────────────────────────┐
//! │ RUNNING 42   STOPPED 3   INVALID 0   DISABLED 1   THREADS 5              │ ← processor info line (1 row)
//! ├─ Nodes (N connected) ─────────────────────────────────────────────────────┤ ← nodes zone (variable, capped)
//! │   node-name   heap  N%   gc Nms/5m   load N.N                            │
//! │   repositories  content  N%   flowfile  N%   provenance  N%              │
//! ├─ Bulletins / min ──────────────┬─ Noisy components (top 5) ──────────────┤ ← bulletins+noisy (8 rows)
//! │  sparkline                     │  cnt  source              worst          │
//! ├─ Unhealthy queues ─────────────────────────────────────────────────────────┤ ← unhealthy queues (fills rest)
//! │ fill  queue             src → dst                      ffiles             │
//! └────────────────────────────────────────────────────────────────────────────┘
//! ```

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table};

use super::state::{
    BulletinBucket, NoisyComponent, OverviewSnapshot, OverviewState, Severity, UnhealthyQueue,
};
use crate::theme;
use crate::widget::gauge::{fill_bar, spark_bar};

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let block = Block::default().title(" Overview ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Vertical zones. Nodes zone height adapts to the number of nodes
    // (1 row per node + 1 title + 1 repos row), capped to keep the
    // bulletin/queue zones readable.
    let nodes_height = nodes_zone_height(state);
    let zones = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),            // processor info line
            Constraint::Length(nodes_height), // nodes zone
            Constraint::Length(8),            // bulletins/noisy zone
            Constraint::Fill(1),              // unhealthy queues
        ])
        .split(inner);

    render_processor_info_line(frame, zones[0], state.snapshot.as_ref());
    render_nodes_zone(frame, zones[1], state);
    render_bulletins_and_noisy(frame, zones[2], state);
    render_unhealthy_queues(frame, zones[3], &state.unhealthy);
}

/// Compute how many rows the nodes zone needs. One title row + one row
/// per visible node + one row for the repositories aggregate. Capped so
/// the zone never starves the lower zones.
fn nodes_zone_height(state: &OverviewState) -> u16 {
    let visible_nodes = state.nodes.nodes.len().min(8) as u16;
    // 1 title row + N node rows + 1 repositories row. Min 3 (loading
    // state needs 1 title + 1 loading msg + 1 buffer).
    let needed = 1 + visible_nodes.max(1) + 1;
    needed.clamp(3, 12)
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
    let mut lines: Vec<Line> = Vec::new();

    // Title row.
    let total = state.nodes.nodes.len();
    let title_text = if total == 0 {
        "Nodes".to_string()
    } else {
        format!("Nodes ({total} connected)")
    };
    lines.push(Line::from(Span::styled(title_text, theme::bold())));

    if state.nodes.nodes.is_empty() {
        lines.push(Line::from(Span::styled(
            "  loading system diagnostics…",
            theme::muted(),
        )));
    } else {
        // Up to 8 node rows, then "+N more" if there are extras.
        let max_visible = 8;
        let visible = state.nodes.nodes.iter().take(max_visible);
        for node in visible {
            lines.push(format_node_row(node));
        }
        if total > max_visible {
            lines.push(Line::from(Span::styled(
                format!("  … +{} more", total - max_visible),
                theme::muted(),
            )));
        }

        // Repositories aggregate row with inline fill bars.
        let repos = &state.repositories_summary;
        lines.push(Line::from(vec![
            Span::styled("  repositories  ", theme::muted()),
            Span::raw("content "),
            Span::styled(
                fill_bar(4, repos.content_percent),
                fill_style(repos.content_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.content_percent),
                fill_style(repos.content_percent),
            ),
            Span::raw("   flowfile "),
            Span::styled(
                fill_bar(4, repos.flowfile_percent),
                fill_style(repos.flowfile_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.flowfile_percent),
                fill_style(repos.flowfile_percent),
            ),
            Span::raw("   provenance "),
            Span::styled(
                fill_bar(4, repos.provenance_percent),
                fill_style(repos.provenance_percent),
            ),
            Span::raw(" "),
            Span::styled(
                format!("{:>3}%", repos.provenance_percent),
                fill_style(repos.provenance_percent),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

/// Format a single node row using fields from `NodeHealthRow`.
fn format_node_row(node: &crate::client::health::NodeHealthRow) -> Line<'static> {
    // GC severity: treat > 5 new collections per poll as warning-level.
    let gc_style = match node.gc_delta {
        Some(d) if d > 5 => theme::error(),
        Some(_) => theme::warning(),
        None => Style::default(),
    };
    let gc_str = match node.gc_delta {
        Some(d) => format!("{:>4}ms (+{})", node.gc_millis, d),
        None => format!("{:>4}ms", node.gc_millis),
    };

    // Load severity derived from load_average vs available_processors.
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
    // Load bar shows load average as a fraction of available processors
    // (1.0 = fully loaded). Falls back to an empty bar when either value
    // is missing.
    let load_bar = match (node.load_average, node.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => spark_bar(l as f32, cpus as f32, 4),
        _ => "░░░░".to_string(),
    };
    Line::from(vec![
        Span::raw("  "),
        Span::styled(node.node_address.clone(), theme::accent()),
        Span::raw("   heap "),
        Span::styled(heap_bar, heap_style),
        Span::raw(" "),
        Span::styled(format!("{:>3}%", node.heap_percent), heap_style),
        Span::raw("   gc "),
        Span::styled(gc_str, gc_style),
        Span::raw("   load "),
        Span::styled(load_bar, load_style),
        Span::raw(" "),
        Span::styled(load_str, load_style),
    ])
}

fn render_bulletins_and_noisy(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_bulletin_sparkline(frame, cols[0], &state.sparkline);
    render_noisy_components(frame, cols[1], &state.noisy);
}

fn render_bulletin_sparkline(frame: &mut Frame, area: Rect, buckets: &[BulletinBucket]) {
    let max_sev = buckets
        .iter()
        .map(|b| b.max_severity)
        .max()
        .unwrap_or(Severity::Unknown);
    let title = Line::from(vec![
        Span::raw("Bulletins / min "),
        Span::styled(format!("(15m, worst {max_sev:?})"), theme::muted()),
    ]);
    let data: Vec<u64> = buckets.iter().map(|b| b.count as u64).collect();
    let spark = Sparkline::default()
        .block(Block::default().title(title))
        .data(&data)
        .style(severity_style(max_sev));
    frame.render_widget(spark, area);
}

fn render_noisy_components(frame: &mut Frame, area: Rect, noisy: &[NoisyComponent]) {
    let rows: Vec<Row> = if noisy.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled("no bulletins yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        noisy
            .iter()
            .map(|n| {
                let sev_style = severity_style(n.max_severity);
                Row::new(vec![
                    Cell::from(format!("{:>3}", n.count)).style(theme::bold()),
                    Cell::from(n.source_name.clone()),
                    Cell::from(format!("{:?}", n.max_severity)).style(sev_style),
                ])
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
    .header(Row::new(vec!["cnt", "source", "worst"]).style(theme::bold()))
    .block(Block::default().title("Noisy components (top 5)"));
    frame.render_widget(table, area);
}

fn render_unhealthy_queues(frame: &mut Frame, area: Rect, queues: &[UnhealthyQueue]) {
    let rows: Vec<Row> = if queues.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(Span::styled("no queues reported yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        queues
            .iter()
            .map(|q| {
                let style = fill_style(q.fill_percent);
                Row::new(vec![
                    Cell::from(format!("{:>3}%", q.fill_percent)).style(style),
                    Cell::from(q.name.clone()),
                    Cell::from(format!("{} → {}", q.source_name, q.destination_name)),
                    Cell::from(q.flow_files_queued.to_string()),
                ])
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
    .header(Row::new(vec!["fill", "queue", "src → dst", "ffiles"]).style(theme::bold()))
    .block(
        Block::default()
            .title(" Unhealthy queues ")
            .borders(Borders::ALL),
    );
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
}
