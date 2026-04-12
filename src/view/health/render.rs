//! Two-pane renderer for the Health tab.
//!
//! Left strip (28 cols): severity dots + category names + selection marker.
//! Right pane: per-category detail (Queues / Repositories / Nodes / Processors).

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::client::health::{
    NodeHealthRow, NodeRepoFillBar, ProcessorThreadRow, QueuePressureRow, RepoKind, RepoRow,
    Severity, TimeToFull,
};
use crate::theme;
use crate::view::health::state::{HealthCategory, HealthState};

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

pub fn render(frame: &mut Frame, area: Rect, state: &HealthState) {
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(28), Constraint::Min(1)])
        .split(area);

    render_left_strip(frame, chunks[0], state);
    render_right_pane(frame, chunks[1], state);
}

// ---------------------------------------------------------------------------
// Left strip
// ---------------------------------------------------------------------------

const CATEGORIES: [(HealthCategory, &str); 4] = [
    (HealthCategory::Queues, "Queues"),
    (HealthCategory::Repositories, "Repositories"),
    (HealthCategory::Nodes, "Nodes"),
    (HealthCategory::Processors, "Processors"),
];

fn render_left_strip(frame: &mut Frame, area: Rect, state: &HealthState) {
    let block = Block::default().borders(Borders::RIGHT);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Category rows: one per line, starting from the top.
    let rows_area = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(2)])
        .split(inner);

    let cat_area = rows_area[0];
    for (i, (cat, label)) in CATEGORIES.iter().enumerate() {
        if i as u16 >= cat_area.height {
            break;
        }
        let row_rect = Rect::new(cat_area.x, cat_area.y + i as u16, cat_area.width, 1);
        let selected = *cat == state.selected_category;

        let dot_color = worst_severity_for(state, *cat);
        let badge = warn_red_count(state, *cat);

        let marker = if selected { " \u{25b8}" } else { "  " };
        let badge_span = if badge == 0 {
            Span::styled(format!("  {badge}"), theme::muted())
        } else {
            Span::styled(format!("  {badge}"), Style::default().fg(Color::Yellow))
        };

        let line = Line::from(vec![
            Span::raw("  "),
            Span::styled("\u{25cf} ", Style::default().fg(severity_color(&dot_color))),
            Span::styled(
                format!("{label:<14}"),
                if selected {
                    Style::default().add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                },
            ),
            Span::raw(marker),
            badge_span,
        ]);
        frame.render_widget(Paragraph::new(line), row_rect);
    }

    // Footer.
    let footer_area = rows_area[1];
    let fetched_line = fetched_ago_line(state);
    let hint_line = Line::from(Span::styled("1-4 select \u{00b7} ? help", theme::muted()));

    if footer_area.height >= 2 {
        let fa = Rect::new(footer_area.x, footer_area.y, footer_area.width, 1);
        let ha = Rect::new(footer_area.x, footer_area.y + 1, footer_area.width, 1);
        frame.render_widget(
            Paragraph::new(fetched_line).alignment(Alignment::Center),
            fa,
        );
        frame.render_widget(Paragraph::new(hint_line).alignment(Alignment::Center), ha);
    } else if footer_area.height >= 1 {
        let fa = Rect::new(footer_area.x, footer_area.y, footer_area.width, 1);
        frame.render_widget(Paragraph::new(hint_line).alignment(Alignment::Center), fa);
    }
}

fn fetched_ago_line(state: &HealthState) -> Line<'static> {
    let ts = match state.selected_category {
        HealthCategory::Queues | HealthCategory::Processors => state.last_pg_refresh,
        HealthCategory::Repositories | HealthCategory::Nodes => state.last_sysdiag_refresh,
    };
    let text = match ts {
        Some(t) => {
            let secs = t.elapsed().as_secs();
            format!("fetched {secs}s ago")
        }
        None => "\u{2014}".to_string(),
    };
    Line::from(Span::styled(text, theme::muted()))
}

fn worst_severity_for(state: &HealthState, cat: HealthCategory) -> Severity {
    match cat {
        HealthCategory::Queues => state
            .queues
            .rows
            .iter()
            .map(|r| severity_rank(&r.severity))
            .max()
            .map(rank_to_severity)
            .unwrap_or(Severity::Green),
        HealthCategory::Repositories => {
            let bars = state
                .repositories
                .content
                .iter()
                .chain(state.repositories.flowfile.iter())
                .chain(state.repositories.provenance.iter());
            bars.map(|b| severity_rank(&b.severity))
                .max()
                .map(rank_to_severity)
                .unwrap_or(Severity::Green)
        }
        HealthCategory::Nodes => state
            .nodes
            .nodes
            .iter()
            .map(|n| severity_rank(&n.heap_severity))
            .max()
            .map(rank_to_severity)
            .unwrap_or(Severity::Green),
        HealthCategory::Processors => Severity::Green,
    }
}

fn warn_red_count(state: &HealthState, cat: HealthCategory) -> usize {
    match cat {
        HealthCategory::Queues => state
            .queues
            .rows
            .iter()
            .filter(|r| matches!(r.severity, Severity::Yellow | Severity::Red))
            .count(),
        HealthCategory::Repositories => {
            let bars = state
                .repositories
                .content
                .iter()
                .chain(state.repositories.flowfile.iter())
                .chain(state.repositories.provenance.iter());
            bars.filter(|b| matches!(b.severity, Severity::Yellow | Severity::Red))
                .count()
        }
        HealthCategory::Nodes => state
            .nodes
            .nodes
            .iter()
            .filter(|n| matches!(n.heap_severity, Severity::Yellow | Severity::Red))
            .count(),
        HealthCategory::Processors => 0,
    }
}

fn severity_rank(s: &Severity) -> u8 {
    match s {
        Severity::Green => 0,
        Severity::Yellow => 1,
        Severity::Red => 2,
    }
}

fn rank_to_severity(r: u8) -> Severity {
    match r {
        0 => Severity::Green,
        1 => Severity::Yellow,
        _ => Severity::Red,
    }
}

fn severity_color(s: &Severity) -> Color {
    match s {
        Severity::Green => Color::Green,
        Severity::Yellow => Color::Yellow,
        Severity::Red => Color::Red,
    }
}

// ---------------------------------------------------------------------------
// Right pane dispatch
// ---------------------------------------------------------------------------

fn render_right_pane(frame: &mut Frame, area: Rect, state: &HealthState) {
    let block = Block::default().borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match state.selected_category {
        HealthCategory::Queues => render_queues(frame, inner, state),
        HealthCategory::Repositories => render_repositories(frame, inner, state),
        HealthCategory::Nodes => render_nodes(frame, inner, state),
        HealthCategory::Processors => render_processors(frame, inner, state),
    }
}

// ---------------------------------------------------------------------------
// Queues detail
// ---------------------------------------------------------------------------

fn render_queues(frame: &mut Frame, area: Rect, state: &HealthState) {
    if state.last_pg_refresh.is_none() {
        render_centered_muted(frame, area, "waiting for first poll\u{2026}");
        return;
    }
    if state.queues.rows.is_empty() {
        render_centered_muted(frame, area, "all queues healthy");
        return;
    }

    // Two lines per row; scroll if needed.
    let visible_rows = (area.height as usize) / 2;
    let scroll_offset = if visible_rows == 0 || state.queues.selected < visible_rows {
        0
    } else {
        state.queues.selected + 1 - visible_rows
    };

    let window_end = state.queues.rows.len().min(scroll_offset + visible_rows);
    let window = &state.queues.rows[scroll_offset..window_end];
    let selected_in_window = state.queues.selected.saturating_sub(scroll_offset);

    for (i, row) in window.iter().enumerate() {
        let y = area.y + (i as u16) * 2;
        if y + 1 >= area.y + area.height {
            break;
        }
        let is_selected = i == selected_in_window;
        render_queue_row(frame, area, y, row, is_selected);
    }
}

fn render_queue_row(frame: &mut Frame, area: Rect, y: u16, row: &QueuePressureRow, selected: bool) {
    let w = area.width as usize;
    let gutter = if selected { ">" } else { " " };

    // Line 1: gutter + src->dest + fill bar + fill% + time-to-full
    let label = format!(
        "{}\u{2192}{}",
        truncate(&row.source_name, 15),
        truncate(&row.destination_name, 15)
    );
    let bar = crate::widget::gauge::fill_bar(10, row.fill_percent);
    let ttf = format_time_to_full(&row.time_to_full);
    let pct = format!("{:>3}%", row.fill_percent);

    let line1 = Line::from(vec![
        Span::raw(format!("{gutter} ")),
        Span::styled(
            format!("{label:<32}"),
            if selected {
                theme::cursor_row()
            } else {
                Style::default()
            },
        ),
        Span::raw(" "),
        Span::styled(
            format!("[{bar}]"),
            Style::default().fg(severity_color(&row.severity)),
        ),
        Span::raw(" "),
        Span::styled(pct, Style::default().fg(severity_color(&row.severity))),
        Span::raw("     "),
        Span::styled(
            ttf,
            match row.time_to_full {
                TimeToFull::Stalled | TimeToFull::Overflowing => theme::error(),
                _ => theme::muted(),
            },
        ),
    ]);

    // Line 2: indented queued display + rates
    let in_rate = format_bytes(row.bytes_in_5m);
    let out_rate = format_bytes(row.bytes_out_5m);
    let line2 = Line::from(vec![
        Span::styled(format!("    {:<24}", row.queued_display), theme::muted()),
        Span::styled(
            format!("in {in_rate}/5m \u{00b7} out {out_rate}/5m"),
            theme::muted(),
        ),
    ]);

    let r1 = Rect::new(area.x, y, w.min(u16::MAX as usize) as u16, 1);
    let r2 = Rect::new(area.x, y + 1, w.min(u16::MAX as usize) as u16, 1);
    frame.render_widget(Paragraph::new(line1), r1);
    frame.render_widget(Paragraph::new(line2), r2);
}

// ---------------------------------------------------------------------------
// Repositories detail
// ---------------------------------------------------------------------------

fn render_repositories(frame: &mut Frame, area: Rect, state: &HealthState) {
    if state.last_sysdiag_refresh.is_none() {
        render_centered_muted(frame, area, "waiting for first poll\u{2026}");
        return;
    }
    if state.repositories.rows.is_empty() {
        render_centered_muted(frame, area, "no repository data");
        return;
    }

    // Split: left half for aggregate list, right half for per-node detail.
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
        .split(area);

    render_repo_aggregate_list(frame, chunks[0], state);
    render_repo_per_node_detail(frame, chunks[1], state);
}

/// Render the scrollable aggregate repository list with selection.
fn render_repo_aggregate_list(frame: &mut Frame, area: Rect, state: &HealthState) {
    let repos = &state.repositories;

    // Each row occupies 2 lines (fill bar + size details).
    let lines_per_row = 2_usize;
    let visible_rows = (area.height as usize) / lines_per_row;
    let scroll_offset = if visible_rows == 0 || repos.selected < visible_rows {
        0
    } else {
        repos.selected + 1 - visible_rows
    };

    let window_end = repos.rows.len().min(scroll_offset + visible_rows);
    let window = &repos.rows[scroll_offset..window_end];
    let selected_in_window = repos.selected.saturating_sub(scroll_offset);

    for (i, row) in window.iter().enumerate() {
        let y = area.y + (i as u16) * lines_per_row as u16;
        if y + 1 >= area.y + area.height {
            break;
        }
        let is_selected = i == selected_in_window;
        render_repo_aggregate_row(frame, area, y, row, is_selected, &state.repositories);
    }
}

/// Render one aggregate repository row (fill bar + sizes).
fn render_repo_aggregate_row(
    frame: &mut Frame,
    area: Rect,
    y: u16,
    row: &RepoRow,
    selected: bool,
    repos: &crate::client::health::RepositoryState,
) {
    let gutter = if selected { ">" } else { " " };
    let kind_prefix = match row.kind {
        RepoKind::Content => "C",
        RepoKind::FlowFile => "F",
        RepoKind::Provenance => "P",
    };

    let fill = crate::widget::gauge::fill_bar(15, row.fill_percent);
    let pct = format!("{:>3}%", row.fill_percent);

    let line1 = Line::from(vec![
        Span::raw(format!("{gutter} ")),
        Span::styled(
            format!("[{kind_prefix}] "),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{:<20}", truncate(&row.identifier, 18)),
            if selected {
                theme::cursor_row()
            } else {
                Style::default()
            },
        ),
        Span::styled(
            format!("[{fill}]"),
            Style::default().fg(severity_color(&row.severity)),
        ),
        Span::raw(" "),
        Span::styled(pct, Style::default().fg(severity_color(&row.severity))),
    ]);
    frame.render_widget(Paragraph::new(line1), Rect::new(area.x, y, area.width, 1));

    // Line 2: used / total / free from the corresponding RepoFillBar.
    let bar = match row.kind {
        RepoKind::Content => repos
            .content
            .iter()
            .find(|b| b.identifier == row.identifier),
        RepoKind::FlowFile => repos.flowfile.as_ref(),
        RepoKind::Provenance => repos
            .provenance
            .iter()
            .find(|b| b.identifier == row.identifier),
    };
    if let Some(bar) = bar {
        let used = format_bytes(bar.used_bytes);
        let total = format_bytes(bar.total_bytes);
        let free = format_bytes(bar.free_bytes);
        let line2 = Line::from(Span::styled(
            format!("      {used} / {total} \u{00b7} free {free}"),
            theme::muted(),
        ));
        frame.render_widget(
            Paragraph::new(line2),
            Rect::new(area.x, y + 1, area.width, 1),
        );
    }
}

/// Render per-node fill bars for the selected repository.
fn render_repo_per_node_detail(frame: &mut Frame, area: Rect, state: &HealthState) {
    let per_node = &state.repositories.per_node;
    if per_node.is_empty() {
        render_centered_muted(frame, area, "no per-node data");
        return;
    }

    let mut y = area.y;
    let max_y = area.y + area.height;

    // Header
    if y < max_y {
        let hdr = Line::from(vec![
            Span::styled(
                format!("  {:<22}", "Node"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<20}", "Used / Total"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("Fill", Style::default().add_modifier(Modifier::BOLD)),
        ]);
        frame.render_widget(Paragraph::new(hdr), Rect::new(area.x, y, area.width, 1));
        y += 1;
    }

    for node in per_node {
        if y >= max_y {
            break;
        }
        render_node_repo_row(frame, area.x, y, area.width, node);
        y += 1;
    }
}

/// Render one per-node repository row.
fn render_node_repo_row(frame: &mut Frame, x: u16, y: u16, width: u16, node: &NodeRepoFillBar) {
    let bar = crate::widget::gauge::fill_bar(10, node.utilization_percent);
    let pct = format!("{:>3}%", node.utilization_percent);
    let used = format_bytes(node.used_bytes);
    let total = format_bytes(node.total_bytes);

    let line = Line::from(vec![
        Span::raw(format!("  {:<22}", truncate(&node.node_address, 20))),
        Span::raw(format!("{used:>8} / {total:<8}  ")),
        Span::styled(
            format!("[{bar}]"),
            Style::default().fg(severity_color(&node.severity)),
        ),
        Span::raw(" "),
        Span::styled(pct, Style::default().fg(severity_color(&node.severity))),
    ]);
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

// ---------------------------------------------------------------------------
// Nodes detail
// ---------------------------------------------------------------------------

fn render_nodes(frame: &mut Frame, area: Rect, state: &HealthState) {
    if state.last_sysdiag_refresh.is_none() {
        render_centered_muted(frame, area, "waiting for first poll\u{2026}");
        return;
    }
    if state.nodes.nodes.is_empty() {
        render_centered_muted(frame, area, "no cluster node data yet");
        return;
    }

    let mut y = area.y;
    let max_y = area.y + area.height;

    // Header
    if y < max_y {
        let hdr = Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{:<22}", "Node"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<18}", "Heap"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<14}", "GC"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<10}", "Load"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<10}", "Threads"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("Uptime", Style::default().add_modifier(Modifier::BOLD)),
        ]);
        frame.render_widget(Paragraph::new(hdr), Rect::new(area.x, y, area.width, 1));
        y += 1;
    }

    // Scrollable rows
    let visible = (max_y - y) as usize;
    let scroll_offset = if visible == 0 || state.nodes.selected < visible {
        0
    } else {
        state.nodes.selected + 1 - visible
    };

    let window_end = state.nodes.nodes.len().min(scroll_offset + visible);
    let window = &state.nodes.nodes[scroll_offset..window_end];
    let selected_in_window = state.nodes.selected.saturating_sub(scroll_offset);

    for (i, node) in window.iter().enumerate() {
        if y >= max_y {
            break;
        }
        let is_selected = i == selected_in_window;
        render_node_row(frame, area.x, y, area.width, node, is_selected);
        y += 1;
    }
}

fn load_style(load: f32, cpus: u32) -> Style {
    if cpus == 0 {
        return Style::default();
    }
    let busy = cpus as f32;
    if load >= 1.5 * busy {
        crate::theme::error()
    } else if load >= busy {
        crate::theme::warning()
    } else {
        Style::default()
    }
}

fn render_node_row(
    frame: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    node: &NodeHealthRow,
    selected: bool,
) {
    let gutter = if selected { ">" } else { " " };
    let bar = crate::widget::gauge::fill_bar(10, node.heap_percent);
    let pct = format!("{:>3}%", node.heap_percent);

    let gc_delta_str = match node.gc_delta {
        Some(d) if d > 5 => format!("{} (+{})", node.gc_collection_count, d),
        Some(d) => format!("{} (+{})", node.gc_collection_count, d),
        None => format!("{}", node.gc_collection_count),
    };
    let gc_style = match node.gc_delta {
        Some(d) if d > 5 => Style::default().fg(Color::Red),
        _ => Style::default(),
    };

    let (load_str, style) = match (node.load_average, node.available_processors) {
        (Some(l), Some(cpus)) if cpus > 0 => {
            let max = (cpus as f32) * 2.0;
            let gauge = crate::widget::gauge::spark_bar(l as f32, max, 4);
            (format!("{gauge} {l:>4.1}"), load_style(l as f32, cpus))
        }
        (Some(l), _) => (format!("     {l:>4.1}"), Style::default()),
        (None, _) => ("     \u{2014}   ".to_string(), Style::default()),
    };

    let line = Line::from(vec![
        Span::raw(format!("{gutter} ")),
        Span::styled(
            format!("{:<22}", truncate(&node.node_address, 20)),
            if selected {
                theme::cursor_row()
            } else {
                Style::default()
            },
        ),
        Span::styled(
            format!("[{bar}]"),
            Style::default().fg(severity_color(&node.heap_severity)),
        ),
        Span::raw(" "),
        Span::styled(
            format!("{pct}  "),
            Style::default().fg(severity_color(&node.heap_severity)),
        ),
        Span::styled(format!("{gc_delta_str:<14}"), gc_style),
        Span::styled(format!("{load_str:<10}"), style),
        Span::raw(format!("{:<10}", node.total_threads)),
        Span::raw(&node.uptime),
    ]);
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

// ---------------------------------------------------------------------------
// Processors detail
// ---------------------------------------------------------------------------

fn render_processors(frame: &mut Frame, area: Rect, state: &HealthState) {
    if state.last_pg_refresh.is_none() {
        render_centered_muted(frame, area, "waiting for first poll\u{2026}");
        return;
    }
    if state.processors.rows.is_empty() {
        render_centered_muted(frame, area, "no active processors");
        return;
    }

    let mut y = area.y;
    let max_y = area.y + area.height;

    // Header
    if y < max_y {
        let hdr = Line::from(vec![
            Span::styled("  ", Style::default()),
            Span::styled(
                format!("{:<24}", "Processor"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<22}", "PG Path"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<10}", "Threads"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!("{:<12}", "Status"),
                Style::default().add_modifier(Modifier::BOLD),
            ),
            Span::styled("CPU (5m)", Style::default().add_modifier(Modifier::BOLD)),
        ]);
        frame.render_widget(Paragraph::new(hdr), Rect::new(area.x, y, area.width, 1));
        y += 1;
    }

    // Scrollable rows
    let visible = (max_y - y) as usize;
    let scroll_offset = if visible == 0 || state.processors.selected < visible {
        0
    } else {
        state.processors.selected + 1 - visible
    };

    let window_end = state.processors.rows.len().min(scroll_offset + visible);
    let window = &state.processors.rows[scroll_offset..window_end];
    let selected_in_window = state.processors.selected.saturating_sub(scroll_offset);

    for (i, proc) in window.iter().enumerate() {
        if y >= max_y {
            break;
        }
        let is_selected = i == selected_in_window;
        render_processor_row(frame, area.x, y, area.width, proc, is_selected);
        y += 1;
    }
}

fn render_processor_row(
    frame: &mut Frame,
    x: u16,
    y: u16,
    width: u16,
    proc: &ProcessorThreadRow,
    selected: bool,
) {
    let gutter = if selected { ">" } else { " " };
    let cpu = format_duration_nanos(proc.tasks_duration_nanos);

    let line = Line::from(vec![
        Span::raw(format!("{gutter} ")),
        Span::styled(
            format!("{:<24}", truncate(&proc.name, 22)),
            if selected {
                theme::cursor_row()
            } else {
                Style::default()
            },
        ),
        Span::raw(format!("{:<22}", truncate(&proc.group_path, 20))),
        Span::raw(format!("{:<10}", proc.active_threads)),
        Span::styled(
            format!("{:<12}", proc.run_status),
            Style::default().fg(match proc.run_status.as_str() {
                "RUNNING" => Color::Green,
                "STOPPED" => Color::Yellow,
                "DISABLED" => Color::DarkGray,
                "INVALID" => Color::Red,
                _ => Color::White,
            }),
        ),
        Span::raw(cpu),
    ]);
    frame.render_widget(Paragraph::new(line), Rect::new(x, y, width, 1));
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn format_time_to_full(ttf: &TimeToFull) -> String {
    match ttf {
        TimeToFull::Seconds(s) if *s < 60 => "~<1m".to_string(),
        TimeToFull::Seconds(s) if *s < 3600 => format!("~{}m", s / 60),
        TimeToFull::Seconds(s) => format!("~{}h", s / 3600),
        TimeToFull::Stable => "stable".to_string(),
        TimeToFull::Overflowing => "overflowing".to_string(),
        TimeToFull::Stalled => "\u{221E} (stalled)".to_string(),
    }
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.1} MB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

fn format_duration_nanos(nanos: u64) -> String {
    let secs = nanos as f64 / 1_000_000_000.0;
    format!("{secs:.1}s")
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.to_string()
    } else {
        format!("{}\u{2026}", &s[..max.saturating_sub(1)])
    }
}

fn render_centered_muted(frame: &mut Frame, area: Rect, text: &str) {
    let para = Paragraph::new(Span::styled(text, theme::muted())).alignment(Alignment::Center);
    // Centre vertically.
    let y = area.y + area.height / 2;
    if y < area.y + area.height {
        frame.render_widget(para, Rect::new(area.x, y, area.width, 1));
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::render;
    use crate::client::health::{
        NodeHealthRow, NodeRepoFillBar, ProcessorThreadRow, QueuePressureRow, RepoFillBar,
        RepoKind, RepoRow, Severity, TimeToFull,
    };
    use crate::view::health::state::{HealthCategory, HealthState};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::Instant;

    fn snap(state: &HealthState) -> String {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        format!("{}", terminal.backend())
    }

    #[test]
    fn health_initial_loading() {
        let state = HealthState::new();
        insta::assert_snapshot!("health_initial_loading", snap(&state));
    }

    #[test]
    fn health_queues_populated() {
        let mut state = HealthState::new();
        state.last_pg_refresh = Some(Instant::now());
        state.queues.rows = vec![
            QueuePressureRow {
                connection_id: "c1".into(),
                group_id: "g1".into(),
                name: "q1".into(),
                source_name: "GenerateFlowFile".into(),
                destination_name: "PutDatabaseRecord".into(),
                fill_percent: 95,
                flow_files_queued: 12345,
                bytes_queued: 5_900_000,
                queued_display: "12,345 / 5.6 MB".into(),
                bytes_in_5m: 12_900_000,
                bytes_out_5m: 8_500_000,
                time_to_full: TimeToFull::Overflowing,
                severity: Severity::Red,
            },
            QueuePressureRow {
                connection_id: "c2".into(),
                group_id: "g1".into(),
                name: "q2".into(),
                source_name: "ListenHTTP".into(),
                destination_name: "RouteOnAttribute".into(),
                fill_percent: 72,
                flow_files_queued: 500,
                bytes_queued: 2_100_000,
                queued_display: "500 / 2.0 MB".into(),
                bytes_in_5m: 1_048_576,
                bytes_out_5m: 524_288,
                time_to_full: TimeToFull::Seconds(1800),
                severity: Severity::Yellow,
            },
            QueuePressureRow {
                connection_id: "c3".into(),
                group_id: "g2".into(),
                name: "q3".into(),
                source_name: "ConsumeKafka".into(),
                destination_name: "MergeContent".into(),
                fill_percent: 30,
                flow_files_queued: 50,
                bytes_queued: 102_400,
                queued_display: "50 / 100.0 KB".into(),
                bytes_in_5m: 51_200,
                bytes_out_5m: 51_200,
                time_to_full: TimeToFull::Stable,
                severity: Severity::Green,
            },
        ];
        insta::assert_snapshot!("health_queues_populated", snap(&state));
    }

    #[test]
    fn health_queues_empty() {
        let mut state = HealthState::new();
        state.last_pg_refresh = Some(Instant::now());
        // queues.rows stays empty — "all queues healthy"
        insta::assert_snapshot!("health_queues_empty", snap(&state));
    }

    #[test]
    fn health_repositories_populated() {
        let mut state = HealthState::new();
        state.selected_category = HealthCategory::Repositories;
        state.last_sysdiag_refresh = Some(Instant::now());
        state.repositories.content = vec![
            RepoFillBar {
                identifier: "content-1".into(),
                used_bytes: 60_500_000_000,
                total_bytes: 77_500_000_000,
                free_bytes: 17_000_000_000,
                utilization_percent: 78,
                severity: Severity::Yellow,
            },
            RepoFillBar {
                identifier: "content-2".into(),
                used_bytes: 10_000_000_000,
                total_bytes: 50_000_000_000,
                free_bytes: 40_000_000_000,
                utilization_percent: 20,
                severity: Severity::Green,
            },
        ];
        state.repositories.flowfile = Some(RepoFillBar {
            identifier: "flowfile-repo".into(),
            used_bytes: 95_000_000_000,
            total_bytes: 100_000_000_000,
            free_bytes: 5_000_000_000,
            utilization_percent: 95,
            severity: Severity::Red,
        });
        state.repositories.provenance = vec![];
        // Populate rows for the new selection-aware rendering.
        state.repositories.rows = vec![
            RepoRow {
                kind: RepoKind::Content,
                identifier: "content-1".into(),
                fill_percent: 78,
                severity: Severity::Yellow,
            },
            RepoRow {
                kind: RepoKind::Content,
                identifier: "content-2".into(),
                fill_percent: 20,
                severity: Severity::Green,
            },
            RepoRow {
                kind: RepoKind::FlowFile,
                identifier: "flowfile-repo".into(),
                fill_percent: 95,
                severity: Severity::Red,
            },
        ];
        state.repositories.per_node = vec![
            NodeRepoFillBar {
                node_address: "nifi-node-1:8443".into(),
                used_bytes: 30_000_000_000,
                total_bytes: 40_000_000_000,
                free_bytes: 10_000_000_000,
                utilization_percent: 75,
                severity: Severity::Yellow,
            },
            NodeRepoFillBar {
                node_address: "nifi-node-2:8443".into(),
                used_bytes: 30_500_000_000,
                total_bytes: 37_500_000_000,
                free_bytes: 7_000_000_000,
                utilization_percent: 81,
                severity: Severity::Red,
            },
        ];
        insta::assert_snapshot!("health_repositories_populated", snap(&state));
    }

    #[test]
    fn health_nodes_populated() {
        let mut state = HealthState::new();
        state.selected_category = HealthCategory::Nodes;
        state.last_sysdiag_refresh = Some(Instant::now());
        state.nodes.nodes = vec![
            NodeHealthRow {
                node_address: "nifi-node-1:8443".into(),
                heap_used_bytes: 6_000_000_000,
                heap_max_bytes: 8_000_000_000,
                heap_percent: 75,
                heap_severity: Severity::Yellow,
                gc_collection_count: 142,
                gc_delta: Some(3),
                gc_millis: 5200,
                load_average: Some(2.4),
                available_processors: Some(8),
                uptime: "4d 12h".into(),
                total_threads: 287,
            },
            NodeHealthRow {
                node_address: "nifi-node-2:8443".into(),
                heap_used_bytes: 7_200_000_000,
                heap_max_bytes: 8_000_000_000,
                heap_percent: 90,
                heap_severity: Severity::Red,
                gc_collection_count: 210,
                gc_delta: Some(12),
                gc_millis: 8900,
                load_average: Some(5.1),
                available_processors: Some(8),
                uptime: "2d 3h".into(),
                total_threads: 310,
            },
            NodeHealthRow {
                node_address: "nifi-node-3:8443".into(),
                heap_used_bytes: 2_000_000_000,
                heap_max_bytes: 8_000_000_000,
                heap_percent: 25,
                heap_severity: Severity::Green,
                gc_collection_count: 80,
                gc_delta: None,
                gc_millis: 2100,
                load_average: None,
                available_processors: None,
                uptime: "10d 0h".into(),
                total_threads: 150,
            },
        ];
        insta::assert_snapshot!("health_nodes_populated", snap(&state));
    }

    #[test]
    fn health_processors_populated() {
        let mut state = HealthState::new();
        state.selected_category = HealthCategory::Processors;
        state.last_pg_refresh = Some(Instant::now());
        state.processors.rows = vec![
            ProcessorThreadRow {
                processor_id: "p1".into(),
                group_id: "g1".into(),
                name: "PutDatabaseRecord".into(),
                group_path: "root/persist".into(),
                active_threads: 8,
                run_status: "RUNNING".into(),
                tasks_duration_nanos: 4_200_000_000,
            },
            ProcessorThreadRow {
                processor_id: "p2".into(),
                group_id: "g1".into(),
                name: "ConsumeKafka_2_6".into(),
                group_path: "root/ingest".into(),
                active_threads: 4,
                run_status: "RUNNING".into(),
                tasks_duration_nanos: 1_800_000_000,
            },
            ProcessorThreadRow {
                processor_id: "p3".into(),
                group_id: "g2".into(),
                name: "MergeContent".into(),
                group_path: "root/enrich".into(),
                active_threads: 2,
                run_status: "RUNNING".into(),
                tasks_duration_nanos: 900_000_000,
            },
            ProcessorThreadRow {
                processor_id: "p4".into(),
                group_id: "g2".into(),
                name: "UpdateAttribute".into(),
                group_path: "root/enrich/transform".into(),
                active_threads: 0,
                run_status: "STOPPED".into(),
                tasks_duration_nanos: 0,
            },
        ];
        insta::assert_snapshot!("health_processors_populated", snap(&state));
    }
}
