//! Ratatui renderer for the Bulletins tab.
//!
//! Layout:
//!
//! ```text
//! ┌─ Bulletins ──────────────────── last 3s ago ┐
//! │  [E] [W] [I]  type: All  /foo_     +12 new  │
//! │  — 1/2/3 severity · T type · / text · G group · M mute · c copy · P pause · R clear — │
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
use ratatui::widgets::{Cell, Paragraph, Row, Table};
use time;

use crate::theme;
use crate::view::bulletins::state::{BulletinsState, ComponentType, GroupedRow};
use crate::widget::panel::Panel;
use crate::widget::severity::{format_severity_label, severity_style};

const FILTER_BAR_ROWS: u16 = 2;
const DETAIL_PANE_ROWS: u16 = 8;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &BulletinsState,
    browser: &crate::view::browser::state::BrowserState,
    cfg: &crate::timestamp::TimestampConfig,
) {
    let age_label = state
        .last_fetched_at
        .and_then(|fetched| SystemTime::now().duration_since(fetched).ok())
        .map(|d| format!(" last {} ago ", format_age(d.as_secs())))
        .unwrap_or_else(|| " connecting… ".to_string());

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(FILTER_BAR_ROWS + 2), // +2 for panel border
            Constraint::Fill(1),                     // bulletins list
            Constraint::Length(DETAIL_PANE_ROWS + 2), // +2 for panel border
        ])
        .split(area);

    // Filters panel
    let filters_block = Panel::new(" Filters ").into_block();
    let filters_inner = filters_block.inner(rows[0]);
    frame.render_widget(filters_block, rows[0]);
    render_filter_bar(frame, filters_inner, state);

    // Bulletins list panel (with age label on the right)
    let list_block = Panel::new(" Bulletins ")
        .right(Line::from(Span::styled(age_label, theme::muted())))
        .into_block();
    let list_inner = list_block.inner(rows[1]);
    frame.render_widget(list_block, rows[1]);
    render_list(frame, list_inner, state, browser, cfg);

    // Detail panel
    let detail_block = Panel::new(" Detail ").into_block();
    let detail_inner = detail_block.inner(rows[2]);
    frame.render_widget(detail_block, rows[2]);
    render_detail(frame, detail_inner, state, browser);
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

    let counts = state.severity_counts();

    // Row 0: chips-with-counts + type + text + mutes badge + +N new.
    let mut row0: Vec<Span<'static>> = vec![
        chip_with_count("E", counts.error, state.filters.show_error, theme::error()),
        Span::raw(" "),
        chip_with_count(
            "W",
            counts.warning,
            state.filters.show_warning,
            theme::warning(),
        ),
        Span::raw(" "),
        chip_with_count("I", counts.info, state.filters.show_info, theme::info()),
        Span::raw("   type: "),
        Span::styled(
            component_type_label(state.filters.component_type),
            theme::accent(),
        ),
        Span::raw("   "),
    ];
    // Text-input display folded into the chip row.
    let text_display = if let Some(buf) = state.text_input.as_deref() {
        Span::styled(format!("text: {buf}_"), theme::accent())
    } else if state.filters.text.is_empty() {
        Span::styled("text: (none)".to_string(), theme::muted())
    } else {
        Span::styled(format!("text: {}", state.filters.text), theme::accent())
    };
    row0.push(text_display);
    // Mute-count badge.
    if !state.mutes.is_empty() {
        row0.push(Span::raw("   "));
        row0.push(Span::styled(
            format!("muted: {}", state.mutes.len()),
            theme::muted(),
        ));
    }
    // Pause +N new badge.
    if state.new_since_pause > 0 {
        row0.push(Span::raw("   "));
        row0.push(Span::styled(
            format!("+{} new", state.new_since_pause),
            Style::default().add_modifier(Modifier::BOLD),
        ));
    }
    frame.render_widget(Paragraph::new(Line::from(row0)), chunks[0]);

    // Row 1: static per-view hint line. Non-text-input mode only; while
    // text-input is active the chip row already shows the live cursor.
    let row1 = if state.text_input.is_some() {
        Line::from(Span::styled(
            "Enter commit · Esc cancel".to_string(),
            theme::muted(),
        ))
    } else {
        let group_label = state.group_mode.label();
        Line::from(Span::styled(
            format!(
                "— 1/2/3 severity · T type · / text · G group: {group_label} · M mute · c copy · P pause · R clear —"
            ),
            theme::muted(),
        ))
    };
    frame.render_widget(Paragraph::new(row1), chunks[1]);
}

fn chip_with_count(label: &'static str, count: usize, on: bool, on_style: Style) -> Span<'static> {
    let text = format!("[{label} {count}]");
    if on {
        Span::styled(text, on_style.add_modifier(Modifier::BOLD))
    } else {
        Span::styled(text, theme::muted())
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

fn render_list(
    frame: &mut Frame,
    area: Rect,
    state: &BulletinsState,
    browser: &crate::view::browser::state::BrowserState,
    cfg: &crate::timestamp::TimestampConfig,
) {
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
    let groups: Vec<GroupedRow> = state.grouped_view();
    if groups.is_empty() {
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
    let window = &groups[scroll_offset..groups.len().min(scroll_offset + visible_rows)];
    let selected_in_window = state.selected.saturating_sub(scroll_offset);
    let now = time::OffsetDateTime::now_utc();
    let rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, group)| {
            let b = &state.ring[group.latest_ring_idx];
            let style = if idx == selected_in_window {
                theme::cursor_row()
            } else {
                Style::default()
            };
            let count_cell = if group.count > 1 {
                Cell::from(format!("\u{00D7}{}", group.count))
                    .style(theme::muted().add_modifier(Modifier::BOLD))
            } else {
                Cell::from("")
            };
            let stripped = crate::view::bulletins::state::strip_component_prefix(&b.message);
            let normalized = crate::view::bulletins::state::normalize_dynamic_brackets(stripped);
            let pg_cell = match browser.pg_path(&b.group_id) {
                Some(path) => Cell::from(truncate_left(&path, 24)),
                None => {
                    let tail: String = b
                        .group_id
                        .chars()
                        .rev()
                        .take(8)
                        .collect::<String>()
                        .chars()
                        .rev()
                        .collect();
                    Cell::from(format!("\u{2026}{tail}")).style(theme::muted())
                }
            };
            Row::new(vec![
                Cell::from(format_bulletin_time(
                    &b.timestamp_iso,
                    &b.timestamp_human,
                    now,
                    cfg,
                )),
                Cell::from(format_severity_label(&b.level)).style(severity_style(&b.level)),
                count_cell,
                Cell::from(truncate_right(&b.source_name, 20)),
                pg_cell,
                Cell::from(normalized),
            ])
            .style(style)
        })
        .collect();
    let table = Table::new(
        rows,
        [
            Constraint::Length(15), // time
            Constraint::Length(5),  // sev
            Constraint::Length(4),  // count (×999)
            Constraint::Length(20), // source
            Constraint::Length(24), // pg path
            Constraint::Fill(1),    // message
        ],
    )
    .header(
        Row::new(vec!["time", "sev", "#", "source", "pg path", "message"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(table, area);
}

fn render_detail(
    frame: &mut Frame,
    area: Rect,
    state: &BulletinsState,
    browser: &crate::view::browser::state::BrowserState,
) {
    let inner = area;

    let Some(d) = state.group_details() else {
        return;
    };

    let pg_path = browser
        .pg_path(&d.group_id)
        .unwrap_or_else(|| format!("\u{2026}{}", tail8(&d.group_id)));

    // Row 0: header — source · pg path · count · first · last.
    let header = Line::from(vec![
        Span::styled(
            d.source_name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" · "),
        Span::styled(pg_path, theme::accent()),
        Span::raw("   "),
        Span::styled(
            format!(
                "{} occurrence{}",
                d.count,
                if d.count == 1 { "" } else { "s" }
            ),
            theme::muted(),
        ),
        Span::raw(" · "),
        Span::styled(
            format!("first {}", short_time(&d.first_seen_iso)),
            theme::muted(),
        ),
        Span::raw(" · "),
        Span::styled(
            format!("last {}", short_time(&d.last_seen_iso)),
            theme::muted(),
        ),
    ]);

    // Row 1: severity label + raw message (wrapped up to 3 lines).
    let sev_label = match d.severity {
        crate::client::Severity::Error => "ERROR ",
        crate::client::Severity::Warning => "WARN  ",
        crate::client::Severity::Info => "INFO  ",
        crate::client::Severity::Unknown => "      ",
    };
    let sev_style = match d.severity {
        crate::client::Severity::Error => theme::error(),
        crate::client::Severity::Warning => theme::warning(),
        crate::client::Severity::Info => theme::info(),
        crate::client::Severity::Unknown => theme::muted(),
    };
    let max_msg_width = inner.width.saturating_sub(8) as usize; // "ERROR " + 2 padding
    let wrapped = wrap_lines(&d.raw_message, max_msg_width.max(1), 3);
    let mut lines: Vec<Line<'static>> = vec![header, Line::from("")];
    for (i, ml) in wrapped.into_iter().enumerate() {
        if i == 0 {
            lines.push(Line::from(vec![
                Span::styled(
                    sev_label.to_string(),
                    sev_style.add_modifier(Modifier::BOLD),
                ),
                Span::raw("  "),
                Span::raw(ml),
            ]));
        } else {
            lines.push(Line::from(vec![Span::raw("        "), Span::raw(ml)]));
        }
    }

    // Row N: ids line — source id, pg id.
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("id  ".to_string(), theme::muted()),
        Span::raw(d.source_id.clone()),
        Span::raw("   "),
        Span::styled("pg  ".to_string(), theme::muted()),
        Span::raw(d.group_id.clone()),
    ]));

    // Row N+1: action hints.
    lines.push(Line::from(Span::styled(
        "Enter Browser · g goto · M mute · c copy message · R clear filters".to_string(),
        theme::muted(),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
}

fn format_bulletin_time(
    iso: &str,
    human: &str,
    now: time::OffsetDateTime,
    cfg: &crate::timestamp::TimestampConfig,
) -> String {
    let dt = crate::timestamp::parse_nifi_timestamp(iso)
        .or_else(|| crate::timestamp::parse_nifi_timestamp(human));
    match dt {
        Some(dt) => crate::timestamp::format(dt, now, cfg, false),
        None => "--:--:--".to_string(),
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

/// Last 8 chars of a UUID-ish id, used for muted fallback display.
/// UTF-8 safe — uses `chars()` to avoid panicking on multi-byte input.
fn tail8(id: &str) -> String {
    let n = id.chars().count();
    let skip = n.saturating_sub(8);
    id.chars().skip(skip).collect()
}

/// Extract HH:MM:SS from an ISO-8601 / RFC-3339 timestamp string.
/// Returns `"--:--:--"` on parse failure.
fn short_time(iso: &str) -> String {
    let Some(dt) = crate::timestamp::parse_nifi_timestamp(iso) else {
        return "--:--:--".to_string();
    };
    let t = dt.time();
    format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
}

#[cfg(test)]
mod tests {
    use super::render;
    use crate::client::BulletinSnapshot;
    use crate::view::bulletins::state::BulletinsState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, SystemTime};

    fn b(
        id: i64,
        level: &str,
        source_type: &str,
        source_name: &str,
        message: &str,
        ts: &str,
    ) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: level.into(),
            message: message.into(),
            source_id: format!("src-{id}"),
            source_name: source_name.into(),
            source_type: source_type.into(),
            group_id: "root".into(),
            timestamp_iso: ts.into(),
            timestamp_human: String::new(),
        }
    }

    fn seed_state(rows: Vec<BulletinSnapshot>) -> BulletinsState {
        let mut s = BulletinsState::with_capacity(100);
        // Task 7: the production path populates the ring from the
        // cluster snapshot via `redraw_bulletins(&mut AppState)`.
        // Render tests construct `BulletinsState` directly, so we seed
        // the mirror ring by hand AND replicate the auto-scroll
        // bottom-snap that `redraw_bulletins` performs.
        for row in rows {
            s.ring.push_back(row);
        }
        if s.auto_scroll {
            let max = s.grouped_view().len().saturating_sub(1);
            s.selected = max;
        }
        // Pin last_fetched_at to a fixed offset from SystemTime::now() so
        // the rendered "last Ns ago" label is width-stable across test
        // runs.
        s.last_fetched_at = Some(SystemTime::now() - Duration::from_secs(3));
        s
    }

    fn render_to_string(state: &BulletinsState) -> String {
        let backend = TestBackend::new(120, 30);
        let mut term = Terminal::new(backend).unwrap();
        let cfg = crate::timestamp::TimestampConfig::default();
        let browser = crate::view::browser::state::BrowserState::new();
        term.draw(|f| render(f, f.area(), state, &browser, &cfg))
            .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn snapshot_empty() {
        let state = BulletinsState::with_capacity(100);
        insta::assert_snapshot!("bulletins_empty", render_to_string(&state));
    }

    #[test]
    fn snapshot_seeded_all_on() {
        let rows = vec![
            b(
                1,
                "INFO",
                "PROCESSOR",
                "GenerateFlowFile",
                "1 file generated",
                "2026-04-11T10:14:10Z",
            ),
            b(
                2,
                "WARN",
                "PROCESSOR",
                "UpdateAttribute",
                "expression evaluated to empty string",
                "2026-04-11T10:14:12Z",
            ),
            b(
                3,
                "ERROR",
                "PROCESSOR",
                "PutKafka",
                "NotLeaderForPartitionException: server is not the leader",
                "2026-04-11T10:14:20Z",
            ),
            b(
                4,
                "INFO",
                "CONTROLLER_SERVICE",
                "AvroReader",
                "reader initialized",
                "2026-04-11T10:14:21Z",
            ),
            b(
                5,
                "ERROR",
                "PROCESSOR",
                "PutDatabaseRecord",
                "connection refused: database unreachable",
                "2026-04-11T10:14:22Z",
            ),
        ];
        let state = seed_state(rows);
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { insta::assert_snapshot!("bulletins_seeded_all_on", render_to_string(&state)); }
        );
    }

    #[test]
    fn snapshot_filtered_severity_only_errors() {
        let rows = vec![
            b(1, "INFO", "PROCESSOR", "A", "info", "2026-04-11T10:14:10Z"),
            b(2, "WARN", "PROCESSOR", "B", "warn", "2026-04-11T10:14:12Z"),
            b(
                3,
                "ERROR",
                "PROCESSOR",
                "C",
                "error one",
                "2026-04-11T10:14:20Z",
            ),
            b(
                4,
                "ERROR",
                "PROCESSOR",
                "D",
                "error two",
                "2026-04-11T10:14:22Z",
            ),
        ];
        let mut state = seed_state(rows);
        state.toggle_info();
        state.toggle_warning();
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            {
                insta::assert_snapshot!(
                    "bulletins_filtered_severity_only_errors",
                    render_to_string(&state)
                );
            }
        );
    }

    #[test]
    fn snapshot_paused_with_badge() {
        let rows = vec![
            b(1, "ERROR", "PROCESSOR", "A", "one", "2026-04-11T10:14:10Z"),
            b(2, "ERROR", "PROCESSOR", "B", "two", "2026-04-11T10:14:12Z"),
            b(
                3,
                "ERROR",
                "PROCESSOR",
                "C",
                "three",
                "2026-04-11T10:14:14Z",
            ),
            b(4, "ERROR", "PROCESSOR", "D", "four", "2026-04-11T10:14:16Z"),
        ];
        let mut state = seed_state(rows);
        state.auto_scroll = false;
        state.selected = 1;
        state.new_since_pause = 7;
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { insta::assert_snapshot!("bulletins_paused_with_badge", render_to_string(&state)); }
        );
    }

    #[test]
    fn snapshot_text_input_active() {
        let rows = vec![
            b(
                1,
                "ERROR",
                "PROCESSOR",
                "PutKafka",
                "IOException: timeout",
                "2026-04-11T10:14:10Z",
            ),
            b(
                2,
                "INFO",
                "PROCESSOR",
                "GenerateFlowFile",
                "ok",
                "2026-04-11T10:14:12Z",
            ),
        ];
        let mut state = seed_state(rows);
        state.enter_text_input_mode();
        let prev = state.selected_ring_index();
        state.push_text_input('i', prev);
        let prev = state.selected_ring_index();
        state.push_text_input('o', prev);
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            {
                insta::assert_snapshot!("bulletins_text_input_active", render_to_string(&state));
            }
        );
    }

    #[test]
    fn snapshot_dedups_identical_stems_across_sources() {
        // Four bulletins: three from src-a (same stem "boom", should fold ×3),
        // interleaved with one from src-b. Dedup produces 2 rows.
        let rows = vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "ProcA[id=a] boom".into(),
                source_id: "src-a".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "ProcB[id=b] crash".into(),
                source_id: "src-b".into(),
                source_name: "ProcB".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:23Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "ERROR".into(),
                message: "ProcA[id=a] boom".into(),
                source_id: "src-a".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:24Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 4,
                level: "ERROR".into(),
                message: "ProcA[id=a] boom".into(),
                source_id: "src-a".into(),
                source_name: "ProcA".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:25Z".into(),
                timestamp_human: String::new(),
            },
        ];
        let state = seed_state(rows);
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            {
                insta::assert_snapshot!(
                    "bulletins_dedups_identical_stems_across_sources",
                    render_to_string(&state)
                );
            }
        );
    }
}
