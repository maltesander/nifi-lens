//! Ratatui renderer for the Overview tab.
//!
//! Layout:
//!
//! ```text
//! ┌─ identity ────────────────────────────────────┐
//! ├─ component counts ────────────────────────────┤
//! ├─ bulletin-rate sparkline (last 15 minutes) ───┤
//! │                                                │
//! ├─ unhealthy queues ───── noisy components ─────┤
//! │                     │                          │
//! └────────────────────────────────────────────────┘
//! ```

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Sparkline, Table};

use super::state::{
    BulletinBucket, NoisyComponent, OverviewSnapshot, OverviewState, SPARKLINE_MINUTES, Severity,
    UnhealthyQueue,
};
use crate::theme;

pub fn render(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let block = Block::default().title(" Overview ").borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // identity strip
            Constraint::Length(1), // component counts
            Constraint::Length(4), // sparkline (title + 3 rows of bars)
            Constraint::Fill(1),   // bottom two-column leaderboards
        ])
        .split(inner);

    render_identity(frame, rows[0], state.snapshot.as_ref());
    render_counts(frame, rows[1], state.snapshot.as_ref());
    render_sparkline(frame, rows[2], &state.sparkline);

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(60), Constraint::Percentage(40)])
        .split(rows[3]);

    render_unhealthy(frame, cols[0], &state.unhealthy);
    render_noisy(frame, cols[1], &state.noisy);
}

fn render_identity(frame: &mut Frame, area: Rect, snapshot: Option<&OverviewSnapshot>) {
    let line = match snapshot {
        Some(s) => {
            let title = if s.about.title.is_empty() {
                "NiFi".to_string()
            } else {
                s.about.title.clone()
            };
            Line::from(vec![
                Span::styled(title, theme::accent()),
                Span::raw("   version "),
                Span::styled(s.about.version.clone(), theme::accent()),
                Span::raw("   flowfiles queued "),
                Span::styled(s.controller.flow_files_queued.to_string(), theme::bold()),
            ])
        }
        None => Line::from(Span::styled("loading cluster identity…", theme::muted())),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_counts(frame: &mut Frame, area: Rect, snapshot: Option<&OverviewSnapshot>) {
    let line = match snapshot {
        Some(s) => {
            let c = &s.controller;
            Line::from(vec![
                Span::raw("running "),
                Span::styled(c.running.to_string(), theme::success()),
                Span::raw("  stopped "),
                Span::styled(c.stopped.to_string(), theme::warning()),
                Span::raw("  invalid "),
                Span::styled(c.invalid.to_string(), theme::error()),
                Span::raw("  disabled "),
                Span::styled(c.disabled.to_string(), theme::muted()),
                Span::raw("  threads "),
                Span::styled(c.active_threads.to_string(), theme::accent()),
            ])
        }
        None => Line::from(Span::styled("loading component counts…", theme::muted())),
    };
    frame.render_widget(Paragraph::new(line), area);
}

fn render_sparkline(frame: &mut Frame, area: Rect, buckets: &[BulletinBucket]) {
    let title = {
        let max_sev = buckets
            .iter()
            .map(|b| b.max_severity)
            .max()
            .unwrap_or(Severity::Unknown);
        let style = severity_style(max_sev);
        Line::from(vec![
            Span::raw("bulletins/min (last "),
            Span::styled(SPARKLINE_MINUTES.to_string(), theme::accent()),
            Span::raw(" min, worst "),
            Span::styled(format!("{max_sev:?}"), style),
            Span::raw(")"),
        ])
    };
    let data: Vec<u64> = buckets.iter().map(|b| b.count as u64).collect();
    let spark = Sparkline::default()
        .block(Block::default().title(title))
        .data(&data)
        .style(theme::accent());
    frame.render_widget(spark, area);
}

fn render_unhealthy(frame: &mut Frame, area: Rect, queues: &[UnhealthyQueue]) {
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

fn render_noisy(frame: &mut Frame, area: Rect, noisy: &[NoisyComponent]) {
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
                    Cell::from(n.count.to_string()).style(theme::bold()),
                    Cell::from(n.source_name.clone()),
                    Cell::from(format!("{:?}", n.max_severity)).style(sev_style),
                ])
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(6),
            Constraint::Fill(1),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["count", "source", "worst"]).style(theme::bold()))
    .block(
        Block::default()
            .title(" Noisy components ")
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
    use crate::event::OverviewPayload;
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
        let payload = OverviewPayload {
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
        };
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
        let payload = OverviewPayload {
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
        };
        apply_payload(&mut state, payload);
        insta::assert_snapshot!("overview_unhealthy", render_to_string(&state));
    }
}
