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
use ratatui::style::{Color, Modifier, Style};
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
                Span::styled(
                    s.controller.flow_files_queued.to_string(),
                    Style::default().add_modifier(Modifier::BOLD),
                ),
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
                Span::styled(c.running.to_string(), Style::default().fg(Color::Green)),
                Span::raw("  stopped "),
                Span::styled(c.stopped.to_string(), Style::default().fg(Color::Yellow)),
                Span::raw("  invalid "),
                Span::styled(c.invalid.to_string(), Style::default().fg(Color::Red)),
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
        let color = severity_color(max_sev);
        Line::from(vec![
            Span::raw("bulletins/min (last "),
            Span::styled(SPARKLINE_MINUTES.to_string(), theme::accent()),
            Span::raw(" min, worst "),
            Span::styled(format!("{max_sev:?}"), Style::default().fg(color)),
            Span::raw(")"),
        ])
    };
    let data: Vec<u64> = buckets.iter().map(|b| b.count as u64).collect();
    let spark = Sparkline::default()
        .block(Block::default().title(title))
        .data(&data)
        .style(Style::default().fg(Color::Cyan));
    frame.render_widget(spark, area);
}

fn render_unhealthy(frame: &mut Frame, area: Rect, queues: &[UnhealthyQueue]) {
    let rows: Vec<Row> = if queues.is_empty() {
        vec![Row::new(vec![Cell::from(Span::styled(
            "no queues reported yet",
            theme::muted(),
        ))])]
    } else {
        queues
            .iter()
            .map(|q| {
                let color = fill_color(q.fill_percent);
                Row::new(vec![
                    Cell::from(format!("{:>3}%", q.fill_percent)).style(Style::default().fg(color)),
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
    .header(
        Row::new(vec!["fill", "queue", "src → dst", "ffiles"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .title(" Unhealthy queues ")
            .borders(Borders::ALL),
    );
    frame.render_widget(table, area);
}

fn render_noisy(frame: &mut Frame, area: Rect, noisy: &[NoisyComponent]) {
    let rows: Vec<Row> = if noisy.is_empty() {
        vec![Row::new(vec![Cell::from(Span::styled(
            "no bulletins yet",
            theme::muted(),
        ))])]
    } else {
        noisy
            .iter()
            .map(|n| {
                let sev_color = severity_color(n.max_severity);
                Row::new(vec![
                    Cell::from(n.count.to_string())
                        .style(Style::default().add_modifier(Modifier::BOLD)),
                    Cell::from(n.source_name.clone()),
                    Cell::from(format!("{:?}", n.max_severity))
                        .style(Style::default().fg(sev_color)),
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
    .header(
        Row::new(vec!["count", "source", "worst"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    )
    .block(
        Block::default()
            .title(" Noisy components ")
            .borders(Borders::ALL),
    );
    frame.render_widget(table, area);
}

fn fill_color(percent: u32) -> Color {
    match percent {
        0..=49 => Color::Green,
        50..=79 => Color::Yellow,
        _ => Color::Red,
    }
}

fn severity_color(s: Severity) -> Color {
    match s {
        Severity::Error => Color::Red,
        Severity::Warning => Color::Yellow,
        Severity::Info => Color::Blue,
        Severity::Unknown => Color::DarkGray,
    }
}
