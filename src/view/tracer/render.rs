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
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Gauge, Paragraph, Row, Table};

use crate::client::tracer::{AttributeTriple, ContentRender, ContentSide};
use crate::theme;
use crate::view::tracer::state::{
    AttributeDiffMode, ContentPane, EntryState, EventDetail, LatestEventsView, LineageRunningState,
    LineageView, TracerMode, TracerState,
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
        TracerMode::Lineage(view) => render_lineage(frame, inner, view),
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

// ── Lineage ───────────────────────────────────────────────────────────────────

const DETAIL_HEIGHT: u16 = 14;

fn render_lineage(frame: &mut Frame, area: Rect, view: &LineageView) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),                // timeline
            Constraint::Length(DETAIL_HEIGHT), // detail pane
        ])
        .split(area);

    render_lineage_timeline(frame, rows[0], view);
    render_lineage_detail(frame, rows[1], view);
}

fn render_lineage_timeline(frame: &mut Frame, area: Rect, view: &LineageView) {
    let events = &view.snapshot.events;
    let visible = area.height as usize;
    let scroll_offset = if visible == 0 || view.selected_event < visible {
        0
    } else {
        view.selected_event + 1 - visible
    };

    let window_end = events.len().min(scroll_offset + visible);
    let window = &events[scroll_offset..window_end];
    let selected_in_window = view.selected_event.saturating_sub(scroll_offset);

    let table_rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let is_selected = idx == selected_in_window;
            let base_style = if is_selected {
                theme::cursor_row()
            } else {
                Style::default()
            };

            let marker = if is_selected { ">" } else { " " };
            let time = format_hhmmss_ms(&e.event_time_iso);
            let event_type = format!("{:<16}", truncate(&e.event_type, 16));
            let comp_name = format!("{:<22}", truncate(&e.component_name, 22));
            let group = truncate(&e.group_id, 24).to_string();

            let is_fail = e.relationship.as_deref().is_some_and(|r| r == "failure");

            if is_fail {
                Row::new(vec![
                    Cell::from(Span::styled(marker, base_style)),
                    Cell::from(Span::styled(time, base_style)),
                    Cell::from(Span::styled(event_type, base_style)),
                    Cell::from(Span::styled(comp_name, base_style)),
                    Cell::from(Span::styled(group, base_style)),
                    Cell::from(Span::styled("  \u{2190} fail", theme::error())),
                ])
            } else {
                Row::new(vec![
                    Cell::from(Span::styled(marker, base_style)),
                    Cell::from(Span::styled(time, base_style)),
                    Cell::from(Span::styled(event_type, base_style)),
                    Cell::from(Span::styled(comp_name, base_style)),
                    Cell::from(Span::styled(group, base_style)),
                    Cell::from(Span::raw("")),
                ])
            }
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),  // gutter
            Constraint::Length(12), // HH:MM:SS.mmm
            Constraint::Length(17), // event type (16 + 1 space)
            Constraint::Length(23), // component name (22 + 1 space)
            Constraint::Length(25), // group path (24 + 1 space)
            Constraint::Fill(1),    // fail tag or empty
        ],
    );
    frame.render_widget(table, area);
}

fn render_lineage_detail(frame: &mut Frame, area: Rect, view: &LineageView) {
    // Draw a top border for the detail pane separator.
    let block = Block::default().borders(Borders::TOP);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    match &view.event_detail {
        EventDetail::NotLoaded => {
            let para = Paragraph::new(Span::styled(
                "Press Enter to load this event's detail",
                theme::muted(),
            ))
            .alignment(Alignment::Center);
            let mid = inner.height.saturating_sub(1) / 2;
            let spot = Rect {
                x: inner.x,
                y: inner.y + mid,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(para, spot);
        }
        EventDetail::Loading => {
            let para = Paragraph::new(Span::styled("Loading event detail\u{2026}", theme::muted()))
                .alignment(Alignment::Center);
            let mid = inner.height.saturating_sub(1) / 2;
            let spot = Rect {
                x: inner.x,
                y: inner.y + mid,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(para, spot);
        }
        EventDetail::Failed(err) => {
            let para = Paragraph::new(Span::styled(
                format!("failed to load event detail: {err}"),
                theme::error(),
            ));
            frame.render_widget(para, inner);
        }
        EventDetail::Loaded { event, content } => {
            render_lineage_detail_loaded(frame, inner, event, content, view.diff_mode);
        }
    }
}

fn render_lineage_detail_loaded(
    frame: &mut Frame,
    area: Rect,
    event: &crate::client::tracer::ProvenanceEventDetail,
    content: &ContentPane,
    diff_mode: AttributeDiffMode,
) {
    let s = &event.summary;
    let rel = s.relationship.as_deref().unwrap_or("");
    let details = s.details.as_deref().unwrap_or("");

    // Split into: header (1), attributes (variable), content pane (variable)
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // header line
            Constraint::Fill(1),   // attributes
            Constraint::Length(3), // content pane (header + 2 lines)
        ])
        .split(area);

    // ── Header ──
    let header_line = Line::from(vec![
        Span::styled(format!("Event #{} \u{2014} ", s.event_id), theme::accent()),
        Span::raw(s.component_name.clone()),
        Span::styled(
            format!("  ({} \u{00b7} {})", s.event_type, rel),
            theme::muted(),
        ),
        Span::raw(if details.is_empty() {
            String::new()
        } else {
            format!("  {details}")
        }),
    ]);
    frame.render_widget(Paragraph::new(header_line), rows[0]);

    // ── Attribute table ──
    render_attribute_table(frame, rows[1], &event.attributes, diff_mode);

    // ── Content pane ──
    render_content_pane(frame, rows[2], content);
}

fn render_attribute_table(
    frame: &mut Frame,
    area: Rect,
    attributes: &[AttributeTriple],
    diff_mode: AttributeDiffMode,
) {
    if area.height == 0 {
        return;
    }

    // First row is a header.
    let changed_count = attributes.iter().filter(|a| a.is_changed()).count();
    let mode_indicator = match diff_mode {
        AttributeDiffMode::All => "[ \u{25ba} All | Changed ]",
        AttributeDiffMode::Changed => "[ All | \u{25ba} Changed ]",
    };
    let attr_header = Line::from(vec![
        Span::styled("Attributes  ", theme::muted()),
        Span::styled(
            format!("{mode_indicator} ({changed_count} changed)"),
            theme::muted(),
        ),
    ]);

    let visible_attrs: Vec<&AttributeTriple> =
        attributes.iter().filter(|a| diff_mode.matches(a)).collect();

    // We have area.height rows total. Use row 0 for header, rest for data rows.
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(attr_header), header_area);

    if area.height <= 1 {
        return;
    }
    let table_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height - 1,
    };

    let table_rows: Vec<Row> = visible_attrs
        .iter()
        .map(|attr| {
            let prev = attr.previous.as_deref().unwrap_or("(none)");
            let curr = attr.current.as_deref().unwrap_or("(none)");
            let gutter = if attr.is_changed() { "\u{00b7}" } else { " " };
            let curr_style = if attr.is_changed() {
                theme::warning()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(gutter),
                Cell::from(truncate(&attr.key, 22).to_string()),
                Cell::from(truncate(prev, 28).to_string()),
                Cell::from(Span::styled(truncate(curr, 28).to_string(), curr_style)),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),  // gutter
            Constraint::Length(23), // key (22 + 1)
            Constraint::Length(29), // previous (28 + 1)
            Constraint::Fill(1),    // current
        ],
    );
    frame.render_widget(table, table_area);
}

fn render_content_pane(frame: &mut Frame, area: Rect, content: &ContentPane) {
    if area.height == 0 {
        return;
    }

    match content {
        ContentPane::Collapsed => {
            let header = Line::from(vec![
                Span::styled("Content  ", theme::muted()),
                Span::styled(
                    "[ i input \u{00b7} o output \u{00b7} s save ]",
                    theme::muted(),
                ),
            ]);
            let hint = Paragraph::new(Span::styled(
                "(collapsed \u{2014} press i or o to load)",
                theme::muted(),
            ));
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Fill(1)])
                .split(area);
            frame.render_widget(Paragraph::new(header), rows[0]);
            frame.render_widget(hint, rows[1]);
        }
        ContentPane::LoadingInput => {
            let para = Paragraph::new(Span::styled("loading input\u{2026}", theme::muted()));
            frame.render_widget(para, area);
        }
        ContentPane::LoadingOutput => {
            let para = Paragraph::new(Span::styled("loading output\u{2026}", theme::muted()));
            frame.render_widget(para, area);
        }
        ContentPane::Shown {
            side,
            render,
            total_bytes,
            ..
        } => {
            render_content_shown(frame, area, side, render, *total_bytes);
        }
        ContentPane::Failed(err) => {
            let para = Paragraph::new(Span::styled(
                format!("content error: {err}"),
                theme::error(),
            ));
            frame.render_widget(para, area);
        }
    }
}

fn render_content_shown(
    frame: &mut Frame,
    area: Rect,
    side: &ContentSide,
    render: &ContentRender,
    total_bytes: usize,
) {
    let side_label = match side {
        ContentSide::Input => "input",
        ContentSide::Output => "output",
    };
    let header = Line::from(vec![
        Span::styled(format!("Content ({side_label})  "), theme::muted()),
        Span::styled(format!("{total_bytes} bytes"), theme::muted()),
    ]);

    let body_text = match render {
        ContentRender::Text { pretty } => pretty.as_str(),
        ContentRender::Hex { first_4k } => first_4k.as_str(),
        ContentRender::Empty => "(empty)",
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Fill(1)])
        .split(area);
    frame.render_widget(Paragraph::new(header), rows[0]);

    let available = rows[1].height as usize;
    let lines: Vec<&str> = body_text.lines().collect();
    let shown = lines.len().min(available.saturating_sub(1).max(1));
    let mut display_lines: Vec<Line> = lines[..shown]
        .iter()
        .map(|l| Line::from(Span::raw(l.to_string())))
        .collect();
    if lines.len() > shown {
        let remaining = lines.len() - shown;
        display_lines.push(Line::from(Span::styled(
            format!("\u{2026} ({remaining} more lines)"),
            theme::muted(),
        )));
    }
    frame.render_widget(Paragraph::new(display_lines), rows[1]);
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
                theme::cursor_row()
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

fn format_hhmmss(ts: &str) -> String {
    if let Some(time) = extract_time_part(ts)
        && time.len() >= 8
    {
        return time[..8].to_string();
    }
    "--:--:--".to_string()
}

/// Format timestamp as `HH:MM:SS.mmm`. Falls back to `--:--:--.---`.
fn format_hhmmss_ms(ts: &str) -> String {
    if let Some(time) = extract_time_part(ts) {
        let base = if time.len() >= 8 {
            &time[..8]
        } else {
            "00:00:00"
        };
        let ms = if time.len() >= 12 && time.as_bytes()[8] == b'.' {
            &time[9..12]
        } else {
            "000"
        };
        format!("{base}.{ms}")
    } else {
        "--:--:--.---".to_string()
    }
}

/// Extracts the time portion from either ISO-8601 or NiFi's human-readable
/// timestamp format.
///
/// Supported formats:
/// - ISO-8601: `2026-01-15T12:34:56.789Z` → `"12:34:56.789Z"`
/// - NiFi:     `04/12/2026 10:14:22.001 UTC` → `"10:14:22.001 UTC"`
fn extract_time_part(ts: &str) -> Option<&str> {
    if ts.len() >= 19 && ts.as_bytes()[10] == b'T' {
        // ISO-8601: split at 'T'
        Some(&ts[11..])
    } else if ts.len() >= 19 && ts.as_bytes()[2] == b'/' && ts.as_bytes()[5] == b'/' {
        // NiFi human-readable: MM/dd/yyyy HH:mm:ss.SSS TZ
        Some(&ts[11..])
    } else {
        None
    }
}

/// Truncates `s` to at most `max_chars` Unicode scalar values.
fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
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
            cluster_node_id: None,
            percent: 12,
            started_at: SystemTime::now() - Duration::from_secs(2),
            abort: None,
        });
        insta::with_settings!(
            { filters => vec![(r"elapsed \d+s", "elapsed <N>s")] },
            { insta::assert_snapshot!("lineage_running_low_percent", snap(&state)); }
        );
    }

    // ── Lineage mode snapshot helpers ─────────────────────────────────────────

    fn make_lineage_summary(
        id: i64,
        event_type: &str,
        rel: Option<&str>,
    ) -> crate::client::tracer::ProvenanceEventSummary {
        crate::client::tracer::ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-04-12T10:30:45.123Z".to_string(),
            event_type: event_type.to_string(),
            component_id: "proc-1111-2222-3333-4444".to_string(),
            component_name: "LogAttribute".to_string(),
            component_type: "LogAttribute".to_string(),
            group_id: "pg-root-aaaa-bbbb".to_string(),
            flow_file_uuid: "ff000001-0000-0000-0000-000000000001".to_string(),
            relationship: rel.map(|s| s.to_string()),
            details: None,
        }
    }

    fn make_lineage_detail(event_id: i64) -> crate::client::tracer::ProvenanceEventDetail {
        crate::client::tracer::ProvenanceEventDetail {
            summary: make_lineage_summary(event_id, "CONTENT_MODIFIED", None),
            attributes: vec![
                crate::client::tracer::AttributeTriple {
                    key: "filename".to_string(),
                    previous: Some("old_file.csv".to_string()),
                    current: Some("new_file.csv".to_string()),
                },
                crate::client::tracer::AttributeTriple {
                    key: "mime.type".to_string(),
                    previous: Some("text/plain".to_string()),
                    current: Some("text/plain".to_string()),
                },
            ],
            transit_uri: None,
            input_available: true,
            output_available: true,
        }
    }

    fn seed_lineage_state(state: &mut TracerState) {
        use crate::client::tracer::LineageSnapshot;
        use crate::view::tracer::state::{AttributeDiffMode, EventDetail, LineageView};

        state.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff000001-0000-0000-0000-000000000001".to_string(),
            snapshot: LineageSnapshot {
                events: vec![
                    make_lineage_summary(1, "RECEIVE", None),
                    make_lineage_summary(2, "CONTENT_MODIFIED", None),
                    make_lineage_summary(3, "SEND", Some("failure")),
                ],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 1,
            event_detail: EventDetail::NotLoaded,
            diff_mode: AttributeDiffMode::All,
            fetched_at: SystemTime::now(),
        }));
    }

    #[test]
    fn snapshot_lineage_view_loading_detail() {
        use crate::view::tracer::state::EventDetail;

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.event_detail = EventDetail::Loading;
        }
        insta::assert_snapshot!("lineage_view_loading_detail", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_collapsed_content() {
        use crate::view::tracer::state::{ContentPane, EventDetail};

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Collapsed,
            };
        }
        insta::assert_snapshot!("lineage_view_collapsed_content", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_expanded_text_content() {
        use crate::client::tracer::{ContentRender, ContentSide};
        use crate::view::tracer::state::{ContentPane, EventDetail};
        use std::sync::Arc;

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Shown {
                    side: ContentSide::Output,
                    render: ContentRender::Text {
                        pretty: "{\n  \"key\": \"value\",\n  \"count\": 42\n}".to_string(),
                    },
                    total_bytes: 36,
                    raw: Arc::from(b"{}".as_ref()),
                },
            };
        }
        insta::assert_snapshot!("lineage_view_expanded_text_content", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_diff_mode_changed() {
        use crate::view::tracer::state::{AttributeDiffMode, ContentPane, EventDetail};

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.diff_mode = AttributeDiffMode::Changed;
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Collapsed,
            };
        }
        insta::assert_snapshot!("lineage_view_diff_mode_changed", snap(&state));
    }

    // ── timestamp helper tests ─────────────────────────────────────────────

    use super::{format_hhmmss, format_hhmmss_ms};

    #[test]
    fn format_hhmmss_ms_iso8601() {
        assert_eq!(format_hhmmss_ms("2026-01-15T12:34:56.789Z"), "12:34:56.789");
    }

    #[test]
    fn format_hhmmss_ms_nifi_human() {
        assert_eq!(
            format_hhmmss_ms("04/12/2026 10:14:22.001 UTC"),
            "10:14:22.001"
        );
    }

    #[test]
    fn format_hhmmss_ms_fallback() {
        assert_eq!(format_hhmmss_ms("garbage"), "--:--:--.---");
        assert_eq!(format_hhmmss_ms(""), "--:--:--.---");
    }

    #[test]
    fn format_hhmmss_iso8601() {
        assert_eq!(format_hhmmss("2026-01-15T12:34:56.789Z"), "12:34:56");
    }

    #[test]
    fn format_hhmmss_nifi_human() {
        assert_eq!(format_hhmmss("04/12/2026 10:14:22.001 UTC"), "10:14:22");
    }

    #[test]
    fn format_hhmmss_fallback() {
        assert_eq!(format_hhmmss("garbage"), "--:--:--");
    }
}
