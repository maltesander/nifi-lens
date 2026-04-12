//! Ratatui renderer for the Tracer tab.
//!
//! Layout dispatches by `TracerMode`:
//!
//! ```text
//! ┌─ Tracer ───────────────────────────────────────┐
//! │                                                 │
//! │          (mode-specific content)                │
//! │                                                 │
//! └─────────────────────────────────────────────────┘
//! ```

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table};

use crate::theme;
use crate::view::tracer::state::{
    EntryState, LatestEventsView, LineageRunningState, TracerMode, TracerState,
};

pub fn render(frame: &mut Frame, area: Rect, state: &TracerState) {
    let title = match &state.mode {
        TracerMode::Entry(_) => " Tracer ",
        TracerMode::LineageRunning(_) => " Tracer — Running Lineage Query ",
        TracerMode::Lineage(_) => " Tracer — Lineage ",
        TracerMode::LatestEvents(v) => {
            // We borrow `v` temporarily but need a &'static-ish str for the title —
            // instead we format dynamically and render the block manually below.
            let _ = v;
            " Tracer — Latest Events "
        }
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title_top(Line::from(title));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &state.mode {
        TracerMode::Entry(entry) => render_entry(frame, inner, entry, state.last_error.as_deref()),
        TracerMode::LineageRunning(running) => render_lineage_running(frame, inner, running),
        TracerMode::Lineage(_) => render_lineage_stub(frame, inner),
        TracerMode::LatestEvents(view) => {
            render_latest_events(frame, inner, view, state.last_error.as_deref())
        }
    }
}

// ── Entry ─────────────────────────────────────────────────────────────────────

fn render_entry(frame: &mut Frame, area: Rect, entry: &EntryState, last_error: Option<&str>) {
    // Three vertical sections: prompt, input box, footer hint.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1), // prompt
            Constraint::Length(1), // blank
            Constraint::Length(1), // input
            Constraint::Length(1), // blank
            Constraint::Length(1), // footer
            Constraint::Fill(1),
        ])
        .split(area);

    // Prompt.
    let prompt = Paragraph::new(Span::styled(
        "Paste a flowfile UUID to trace its lineage",
        theme::muted(),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(prompt, rows[1]);

    // Input box.
    let input_text = if entry.input.is_empty() {
        Line::from(vec![
            Span::styled("UUID: ", theme::muted()),
            Span::styled("_", theme::muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("UUID: ", theme::muted()),
            Span::styled(entry.input.clone(), theme::accent()),
            Span::styled("_", theme::muted()),
        ])
    };
    let input_para = Paragraph::new(input_text).alignment(Alignment::Center);
    frame.render_widget(input_para, rows[3]);

    // Footer: error message or hints.
    let footer = if let Some(err) = last_error {
        Paragraph::new(Span::styled(err.to_string(), theme::error())).alignment(Alignment::Center)
    } else {
        Paragraph::new(Span::styled(
            "Enter submit · Esc clear · ? help",
            theme::muted(),
        ))
        .alignment(Alignment::Center)
    };
    frame.render_widget(footer, rows[5]);
}

// ── LineageRunning ────────────────────────────────────────────────────────────

fn render_lineage_running(frame: &mut Frame, area: Rect, running: &LineageRunningState) {
    let elapsed_secs = SystemTime::now()
        .duration_since(running.started_at)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1), // status line
            Constraint::Length(1), // blank
            Constraint::Length(1), // gauge
            Constraint::Length(1), // blank
            Constraint::Length(1), // elapsed + cancel hint
            Constraint::Fill(1),
        ])
        .split(area);

    // Status.
    let status = Paragraph::new(Line::from(vec![
        Span::raw("Running lineage query for "),
        Span::styled(running.uuid.clone(), theme::accent()),
        Span::raw("…"),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(status, rows[1]);

    // Progress gauge.
    let gauge = Gauge::default()
        .gauge_style(Style::default().fg(ratatui::style::Color::Cyan))
        .percent(running.percent as u16)
        .label(format!("{}%", running.percent));
    // Centre the gauge horizontally to avoid it spanning full width.
    let gauge_area = horizontal_center(rows[3], 60);
    frame.render_widget(gauge, gauge_area);

    // Elapsed + cancel hint.
    let hint = Paragraph::new(Line::from(vec![
        Span::styled(format!("elapsed {elapsed_secs}s"), theme::muted()),
        Span::raw("   "),
        Span::styled("Esc to cancel", theme::muted()),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(hint, rows[5]);
}

// ── Lineage stub ──────────────────────────────────────────────────────────────

fn render_lineage_stub(frame: &mut Frame, area: Rect) {
    let para = Paragraph::new("(lineage view — Task 15)").alignment(Alignment::Center);
    let mid = area.height.saturating_sub(1) / 2;
    let spot = Rect {
        x: area.x,
        y: area.y + mid,
        width: area.width,
        height: 1,
    };
    frame.render_widget(para, spot);
}

// ── LatestEvents ──────────────────────────────────────────────────────────────

fn render_latest_events(
    frame: &mut Frame,
    area: Rect,
    view: &LatestEventsView,
    last_error: Option<&str>,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // component label
            Constraint::Fill(1),   // event list
            Constraint::Length(1), // footer
        ])
        .split(area);

    // Component label header.
    let header = Paragraph::new(Line::from(vec![
        Span::styled("Component: ", theme::muted()),
        Span::styled(view.component_label.clone(), theme::accent()),
    ]));
    frame.render_widget(header, rows[0]);

    // Event list (or placeholder).
    if view.loading {
        let placeholder = Paragraph::new(Span::styled(
            "loading latest provenance events…",
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = rows[1].height.saturating_sub(1) / 2;
        let spot = Rect {
            x: rows[1].x,
            y: rows[1].y + mid,
            width: rows[1].width,
            height: 1,
        };
        frame.render_widget(placeholder, spot);
    } else if view.events.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            "no recent events cached for this component",
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = rows[1].height.saturating_sub(1) / 2;
        let spot = Rect {
            x: rows[1].x,
            y: rows[1].y + mid,
            width: rows[1].width,
            height: 1,
        };
        frame.render_widget(placeholder, spot);
    } else {
        render_event_table(frame, rows[1], view);
    }

    // Footer.
    let footer_text = if let Some(err) = last_error {
        Paragraph::new(Span::styled(err.to_string(), theme::error()))
    } else {
        Paragraph::new(Span::styled(
            "Enter trace flowfile · Esc back · r refresh · c copy uuid · ? help",
            theme::muted(),
        ))
    };
    frame.render_widget(footer_text, rows[2]);
}

fn render_event_table(frame: &mut Frame, area: Rect, view: &LatestEventsView) {
    let visible_rows = area.height as usize;
    let scroll_offset = if visible_rows == 0 || view.selected < visible_rows {
        0
    } else {
        view.selected + 1 - visible_rows
    };

    let window_end = view.events.len().min(scroll_offset + visible_rows);
    let window = &view.events[scroll_offset..window_end];
    let selected_in_window = view.selected.saturating_sub(scroll_offset);

    let table_rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let style = if idx == selected_in_window {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            let marker = if idx == selected_in_window { ">" } else { " " };
            let time = format_hhmmss(&e.event_time_iso);
            let uuid_short = short_uuid(&e.flow_file_uuid);
            let relationship = e.relationship.as_deref().unwrap_or("-").to_string();
            let details = e.details.as_deref().unwrap_or("").to_string();
            Row::new(vec![
                Cell::from(marker),
                Cell::from(time),
                Cell::from(e.event_type.clone()),
                Cell::from(uuid_short),
                Cell::from(relationship),
                Cell::from(details),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),
            Constraint::Length(8),
            Constraint::Length(16),
            Constraint::Length(13),
            Constraint::Length(16),
            Constraint::Fill(1),
        ],
    );
    frame.render_widget(table, area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn format_hhmmss(iso: &str) -> String {
    if iso.len() >= 19 && iso.as_bytes()[10] == b'T' {
        iso[11..19].to_string()
    } else {
        "--:--:--".to_string()
    }
}

fn short_uuid(uuid: &str) -> String {
    // Show first 8 chars of UUID (the first segment).
    uuid.chars().take(8).collect()
}

fn horizontal_center(area: Rect, pct: u16) -> Rect {
    let margin = (100u16.saturating_sub(pct)) / 2;
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin),
            Constraint::Percentage(pct),
            Constraint::Percentage(margin),
        ])
        .split(area)[1]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::render;
    use crate::view::tracer::state::{
        LineageRunningState, TracerMode, TracerState, start_latest_events,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, SystemTime};

    fn snap(state: &TracerState) -> String {
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state))
            .unwrap();
        format!("{}", terminal.backend())
    }

    #[test]
    fn snapshot_entry_empty() {
        let state = TracerState::new();
        insta::assert_snapshot!("entry_empty", snap(&state));
    }

    #[test]
    fn snapshot_entry_with_input() {
        let mut state = TracerState::new();
        if let TracerMode::Entry(ref mut e) = state.mode {
            e.input = "550e8400-e29b-41d4-a716-446655440000".to_string();
        }
        insta::assert_snapshot!("entry_with_input", snap(&state));
    }

    #[test]
    fn snapshot_entry_invalid_uuid_banner() {
        let mut state = TracerState::new();
        if let TracerMode::Entry(ref mut e) = state.mode {
            e.input = "not-a-uuid".to_string();
        }
        state.last_error = Some("invalid UUID: not-a-uuid".to_string());
        insta::assert_snapshot!("entry_invalid_uuid_banner", snap(&state));
    }

    #[test]
    fn snapshot_latest_events_loading() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, "abc-component-id-123".to_string());
        insta::assert_snapshot!("latest_events_loading", snap(&state));
    }

    #[test]
    fn snapshot_lineage_running_low_percent() {
        let mut state = TracerState::new();
        state.mode = TracerMode::LineageRunning(LineageRunningState {
            uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            query_id: "qry-001".to_string(),
            percent: 12,
            started_at: SystemTime::now() - Duration::from_secs(2),
            abort: None,
        });
        insta::with_settings!(
            { filters => vec![(r"elapsed \d+s", "elapsed <N>s")] },
            { insta::assert_snapshot!("lineage_running_low_percent", snap(&state)); }
        );
    }
}
