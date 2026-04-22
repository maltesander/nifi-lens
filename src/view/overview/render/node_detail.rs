//! Node detail modal for the Overview tab.
//!
//! Extracted from `render/mod.rs` so Task 22 can rewrite the four-quadrant
//! modal without touching the list rendering code.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table};

use super::{fill_style, health_severity_style};
use crate::client::health::NodeHealthRow;
use crate::theme;
use crate::widget::gauge::fill_bar;
use crate::widget::panel::Panel;

pub fn render_node_detail_modal(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
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

fn render_node_system_summary(frame: &mut Frame, area: Rect, row: &NodeHealthRow) {
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

fn render_node_gc_and_repos(frame: &mut Frame, area: Rect, row: &NodeHealthRow, gc_rows: u16) {
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
    use crate::client::health::{GcSnapshot, NodeHealthRow, RepoUsage, Severity as HSev};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn snapshot_node_detail_modal() {
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
            cluster: None,
        };
        term.draw(|f| super::render_node_detail_modal(f, f.area(), &row))
            .unwrap();
        insta::assert_snapshot!("node_detail_modal", format!("{}", term.backend()));
    }
}
