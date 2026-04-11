//! Ratatui renderer for the Bulletins tab.
//!
//! Layout:
//!
//! ```text
//! ┌─ Bulletins ──────────────────── last 3s ago ┐
//! │  [E] [W] [I]  type: All  /foo_     +12 new  │
//! │  — press e/w/i/T/ /  c to clear — p pause — │
//! ├─────────────────────────────────────────────┤
//! │ HH:MM:SS  SEV   Source           group  msg │
//! │ ...                                          │
//! ├─────────────────────────────────────────────┤
//! │ ERROR  timestamp  group/source               │
//! │                                              │
//! │ wrapped message body, up to 4 lines          │
//! └──────────────────────────────────────────────┘
//! ```

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Paragraph, Row, Table};

use crate::client::Severity;
use crate::theme;
use crate::view::bulletins::state::{BulletinsState, ComponentType};

const FILTER_BAR_ROWS: u16 = 2;
const DETAIL_PANE_ROWS: u16 = 6;

pub fn render(frame: &mut Frame, area: Rect, state: &BulletinsState) {
    let title = build_title(state);
    let block = Block::default().title(title).borders(Borders::ALL);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(FILTER_BAR_ROWS),
            Constraint::Fill(1),
            Constraint::Length(DETAIL_PANE_ROWS),
        ])
        .split(inner);

    render_filter_bar(frame, rows[0], state);
    render_list(frame, rows[1], state);
    render_detail(frame, rows[2], state);
}

fn build_title(state: &BulletinsState) -> Line<'static> {
    let age = state
        .last_fetched_at
        .and_then(|fetched| {
            let now = SystemTime::now();
            now.duration_since(fetched).ok()
        })
        .map(|d| format_age(d.as_secs()))
        .unwrap_or_else(|| "…".to_string());
    Line::from(vec![
        Span::raw(" Bulletins "),
        Span::styled(format!("  last {age} ago "), theme::muted()),
    ])
}

fn format_age(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3600 {
        format!("{}m", secs / 60)
    } else {
        format!("{}h", secs / 3600)
    }
}

fn render_filter_bar(frame: &mut Frame, area: Rect, state: &BulletinsState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Row 0: chips + type + text display + badge.
    let mut row0 = vec![
        chip_span("E", state.filters.show_error, theme::error()),
        Span::raw(" "),
        chip_span("W", state.filters.show_warning, theme::warning()),
        Span::raw(" "),
        chip_span("I", state.filters.show_info, theme::info()),
        Span::raw("   type: "),
        Span::styled(
            component_type_label(state.filters.component_type),
            theme::accent(),
        ),
        Span::raw("   "),
    ];
    let text_display = if state.filters.text.is_empty() {
        Span::styled("text: (none)".to_string(), theme::muted())
    } else {
        Span::styled(format!("text: {}", state.filters.text), theme::accent())
    };
    row0.push(text_display);
    let badge = if state.new_since_pause > 0 {
        Span::styled(
            format!("  +{} new", state.new_since_pause),
            Style::default().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::raw("")
    };
    row0.push(Span::raw("   "));
    row0.push(badge);
    frame.render_widget(Paragraph::new(Line::from(row0)), chunks[0]);

    // Row 1: hints OR live editor line.
    let row1 = if let Some(buf) = state.text_input.as_deref() {
        Line::from(vec![
            Span::raw("/"),
            Span::styled(buf.to_string(), theme::accent()),
            Span::styled("_", theme::muted()),
            Span::styled("  Enter commit · Esc cancel".to_string(), theme::muted()),
        ])
    } else {
        Line::from(Span::styled(
            "— e/w/i toggle severity · T type · / text · c clear · p pause · ? help —".to_string(),
            theme::muted(),
        ))
    };
    frame.render_widget(Paragraph::new(row1), chunks[1]);
}

fn chip_span(label: &'static str, on: bool, on_style: Style) -> Span<'static> {
    if on {
        Span::styled(format!("[{label}]"), on_style.add_modifier(Modifier::BOLD))
    } else {
        Span::styled(format!("[{label}]"), theme::muted())
    }
}

fn component_type_label(ct: Option<ComponentType>) -> String {
    match ct {
        None => "All".to_string(),
        Some(ComponentType::Processor) => "Processor".to_string(),
        Some(ComponentType::ControllerService) => "ControllerService".to_string(),
        Some(ComponentType::ReportingTask) => "ReportingTask".to_string(),
        Some(ComponentType::Other) => "Other".to_string(),
    }
}

fn render_list(frame: &mut Frame, area: Rect, state: &BulletinsState) {
    if state.ring.is_empty() {
        let centered = Paragraph::new(Span::styled(
            "waiting for bulletins…".to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = area.height.saturating_sub(1) / 2;
        let spot = Rect {
            x: area.x,
            y: area.y + mid,
            width: area.width,
            height: 1,
        };
        frame.render_widget(centered, spot);
        return;
    }
    let filtered = state.filtered_indices();
    if filtered.is_empty() {
        let centered = Paragraph::new(Span::styled(
            "no bulletins match the current filters (press c to clear)".to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = area.height.saturating_sub(1) / 2;
        let spot = Rect {
            x: area.x,
            y: area.y + mid,
            width: area.width,
            height: 1,
        };
        frame.render_widget(centered, spot);
        return;
    }
    let visible_rows = area.height.saturating_sub(1) as usize; // subtract 1 for header
    let scroll_offset = if visible_rows == 0 {
        0
    } else if state.selected >= visible_rows {
        state.selected + 1 - visible_rows
    } else {
        0
    };
    let window = &filtered[scroll_offset..filtered.len().min(scroll_offset + visible_rows)];
    let selected_in_window = state.selected.saturating_sub(scroll_offset);
    let rows: Vec<Row> = window
        .iter()
        .map(|&i| &state.ring[i])
        .enumerate()
        .map(|(idx, b)| {
            let style = if idx == selected_in_window {
                Style::default().add_modifier(Modifier::REVERSED)
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(format_hhmmss(&b.timestamp_iso)),
                Cell::from(format_severity_label(&b.level)).style(severity_style(&b.level)),
                Cell::from(truncate_right(&b.source_name, 20)),
                Cell::from(truncate_left(&b.group_id, 20)),
                Cell::from(truncate_right(&b.message, 80)),
            ])
            .style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(5),
            Constraint::Length(20),
            Constraint::Length(20),
            Constraint::Fill(1),
        ],
    )
    .header(
        Row::new(vec!["time", "sev", "source", "group", "message"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(table, area);
}

fn render_detail(frame: &mut Frame, area: Rect, state: &BulletinsState) {
    let block = Block::default().borders(Borders::TOP);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(idx) = state.selected_ring_index() else {
        return;
    };
    let b = &state.ring[idx];
    let sev = format_severity_label(&b.level);
    let sev_style = severity_style(&b.level);
    let line0 = Line::from(vec![
        Span::styled(sev, sev_style.add_modifier(Modifier::BOLD)),
        Span::raw("  "),
        Span::styled(b.timestamp_iso.clone(), theme::accent()),
        Span::raw("  "),
        Span::styled(format!("{}/{}", b.group_id, b.source_name), theme::muted()),
    ]);
    let message_lines = wrap_lines(&b.message, inner.width.saturating_sub(1) as usize, 3);
    let mut lines = vec![line0, Line::from("")];
    for ml in message_lines {
        lines.push(Line::from(ml));
    }
    let para = Paragraph::new(lines);
    frame.render_widget(para, inner);
}

fn format_severity_label(level: &str) -> String {
    match Severity::parse(level) {
        Severity::Error => "ERROR".to_string(),
        Severity::Warning => "WARN ".to_string(),
        Severity::Info => "INFO ".to_string(),
        Severity::Unknown => level.to_ascii_uppercase(),
    }
}

fn severity_style(level: &str) -> Style {
    match Severity::parse(level) {
        Severity::Error => theme::error(),
        Severity::Warning => theme::warning(),
        Severity::Info => theme::info(),
        Severity::Unknown => theme::muted(),
    }
}

fn format_hhmmss(iso: &str) -> String {
    // ISO-8601 / RFC-3339: "YYYY-MM-DDTHH:MM:SS…". Positions 11..19 are
    // HH:MM:SS when the server emits a standard timestamp. Falls back to
    // "--:--:--" for unparseable inputs.
    if iso.len() >= 19 && iso.as_bytes()[10] == b'T' {
        iso[11..19].to_string()
    } else {
        "--:--:--".to_string()
    }
}

fn truncate_right(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

fn truncate_left(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else {
        let skip = count - max.saturating_sub(1);
        let mut out = String::from("…");
        out.extend(s.chars().skip(skip));
        out
    }
}

fn wrap_lines(s: &str, width: usize, max_lines: usize) -> Vec<String> {
    if width == 0 || max_lines == 0 {
        return vec![];
    }
    let mut lines: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut current_chars: usize = 0;
    for word in s.split_whitespace() {
        let word_chars = word.chars().count();
        // If the word by itself is wider than the column, truncate it to fit.
        let (fit_word, fit_chars) = if word_chars > width {
            let truncated: String = word.chars().take(width.saturating_sub(1)).collect();
            let mut t = truncated;
            t.push('…');
            let tc = t.chars().count();
            (t, tc)
        } else {
            (word.to_string(), word_chars)
        };

        let needs_space = !current.is_empty();
        let next_len = current_chars + if needs_space { 1 } else { 0 } + fit_chars;

        if next_len <= width {
            if needs_space {
                current.push(' ');
                current_chars += 1;
            }
            current.push_str(&fit_word);
            current_chars += fit_chars;
            continue;
        }

        // Word doesn't fit on the current line — push current and start a new one.
        if !current.is_empty() {
            lines.push(std::mem::take(&mut current));
        }

        // We're about to start a line that would push us over the ceiling.
        // Truncate the already-pushed last line with an ellipsis and stop.
        if lines.len() >= max_lines {
            if let Some(last) = lines.last_mut() {
                let last_chars = last.chars().count();
                if last_chars > 0 {
                    let keep = last_chars.saturating_sub(1);
                    let mut truncated: String = last.chars().take(keep).collect();
                    truncated.push('…');
                    *last = truncated;
                }
            }
            return lines;
        }

        // Otherwise start the new line with this word.
        current.push_str(&fit_word);
        current_chars = fit_chars;
    }

    if !current.is_empty() && lines.len() < max_lines {
        lines.push(current);
    }

    lines
}
