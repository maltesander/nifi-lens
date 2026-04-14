//! Ratatui renderer for the Events tab.
//!
//! Layout (top to bottom):
//!
//! ```text
//! ┌─ Events ────────────────────── last 3s ago ┐
//! │  [filter bar row 0]                        │
//! │  [filter bar row 1 — hint line]            │
//! ├────────────────────────────────────────────┤
//! │ time · type · component · uuid · attrs · … │
//! │ ...                                        │
//! ├────────────────────────────────────────────┤
//! │ [detail pane — 8 rows]                     │
//! └────────────────────────────────────────────┘
//! ```
//!
//! The middle region shows the empty-state help text when there are
//! no results and no active query. Task 10 lays the filter bar + empty
//! state + running/failed placeholders; Tasks 11 and 12 add the real
//! results list and detail pane.

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;
use crate::view::events::state::{EventsQueryStatus, EventsState, FilterField};
use crate::widget::panel::Panel;

const FILTER_BAR_ROWS: u16 = 2;
const DETAIL_PANE_ROWS: u16 = 8;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &EventsState,
    _cfg: &crate::timestamp::TimestampConfig,
) {
    let age_label = match &state.status {
        EventsQueryStatus::Done { fetched_at, .. } => SystemTime::now()
            .duration_since(*fetched_at)
            .ok()
            .map(|d| format!(" last {} ago ", format_age(d.as_secs())))
            .unwrap_or_else(|| "  ".to_string()),
        _ => "  ".to_string(),
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(FILTER_BAR_ROWS + 2), // +2 for panel border
            Constraint::Fill(1),                     // events list
            Constraint::Length(DETAIL_PANE_ROWS + 2), // +2 for panel border
        ])
        .split(area);

    // Filters panel
    let filters_block = Panel::new(" Filters ").into_block();
    let filters_inner = filters_block.inner(rows[0]);
    frame.render_widget(filters_block, rows[0]);
    render_filter_bar(frame, filters_inner, state);

    // Events list panel (with age label on the right)
    let list_block = Panel::new(" Events ")
        .right(Line::from(Span::styled(age_label, theme::muted())))
        .into_block();
    let list_inner = list_block.inner(rows[1]);
    frame.render_widget(list_block, rows[1]);
    render_body(frame, list_inner, state);

    // Detail panel
    let detail_block = Panel::new(" Detail ").into_block();
    let detail_inner = detail_block.inner(rows[2]);
    frame.render_widget(detail_block, rows[2]);
    render_detail_pane(frame, detail_inner, state);
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

fn render_filter_bar(frame: &mut Frame, area: Rect, state: &EventsState) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    // Row 0: filter chips.
    let row0 = Line::from(vec![
        Span::styled(
            field_chip("t time", FilterField::Time, state),
            theme::accent(),
        ),
        Span::raw("   "),
        Span::styled(
            field_chip("T type", FilterField::Types, state),
            theme::accent(),
        ),
        Span::raw("   "),
        Span::styled(
            field_chip("s source", FilterField::Source, state),
            theme::accent(),
        ),
        Span::raw("   "),
        Span::styled(
            field_chip("u file uuid", FilterField::Uuid, state),
            theme::accent(),
        ),
        Span::raw("   "),
        Span::styled(
            field_chip("a attr", FilterField::Attr, state),
            theme::accent(),
        ),
    ]);
    frame.render_widget(Paragraph::new(row0), chunks[0]);

    // Row 1: status + result count + hints.
    let status_span = match &state.status {
        EventsQueryStatus::Idle => Span::styled("status \u{25cb} idle".to_string(), theme::muted()),
        EventsQueryStatus::Running { percent, .. } => Span::styled(
            format!("status \u{25cf} running {percent}%"),
            theme::info().add_modifier(Modifier::BOLD),
        ),
        EventsQueryStatus::Done {
            truncated, took_ms, ..
        } => {
            let label = if *truncated {
                "done (truncated)"
            } else {
                "done"
            };
            Span::styled(
                format!("status \u{25cf} {label}   took {took_ms}ms"),
                theme::success(),
            )
        }
        EventsQueryStatus::Failed { .. } => Span::styled(
            "status \u{25cf} failed".to_string(),
            theme::error().add_modifier(Modifier::BOLD),
        ),
    };
    let results_span = Span::styled(
        format!("results {} / cap {}", state.events.len(), state.cap),
        theme::muted(),
    );
    let hint_span = if state.filter_edit.is_some() {
        Span::styled(
            "Enter commit \u{00b7} Esc cancel".to_string(),
            theme::muted(),
        )
    } else {
        Span::styled(
            "\u{2014} t/T/s/u/a edit \u{00b7} Enter run \u{00b7} n new \u{00b7} r reset \u{00b7} L raise cap \u{2014}"
                .to_string(),
            theme::muted(),
        )
    };
    let row1 = Line::from(vec![
        status_span,
        Span::raw("   "),
        results_span,
        Span::raw("   "),
        hint_span,
    ]);
    frame.render_widget(Paragraph::new(row1), chunks[1]);
}

fn field_chip(label_prefix: &'static str, field: FilterField, state: &EventsState) -> String {
    let value = state.filters.get(field);
    let editing = state
        .filter_edit
        .as_ref()
        .map(|(f, _)| *f)
        .is_some_and(|f| f == field);
    let shown = if value.is_empty() {
        "(any)".to_string()
    } else {
        value.to_string()
    };
    if editing {
        format!("{label_prefix} {shown}_")
    } else {
        format!("{label_prefix} {shown}")
    }
}

fn render_body(frame: &mut Frame, area: Rect, state: &EventsState) {
    if matches!(state.status, EventsQueryStatus::Idle) && state.events.is_empty() {
        render_empty_state(frame, area);
        return;
    }
    if let EventsQueryStatus::Running { percent, .. } = &state.status {
        let centered = Paragraph::new(Span::styled(
            format!("running query\u{2026} {percent}%"),
            theme::info(),
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
    // Otherwise (Done with events, or Failed), render the real results list.
    // On Failed the results list falls through to its empty-state message;
    // the global footer banner is the single source of truth for the error.
    render_results_list(frame, area, state);
}

fn render_results_list(frame: &mut Frame, area: Rect, state: &EventsState) {
    use ratatui::style::Style;
    use ratatui::widgets::{Cell, Row, Table};

    if state.events.is_empty() {
        let centered = Paragraph::new(Span::styled(
            "no events matched the query".to_string(),
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

    let visible_rows = area.height.saturating_sub(1) as usize; // header row
    let selected = state.selected_row.unwrap_or(0);
    let scroll_offset = if visible_rows == 0 {
        0
    } else if selected >= visible_rows {
        selected + 1 - visible_rows
    } else {
        0
    };
    let end = state.events.len().min(scroll_offset + visible_rows);
    let window = &state.events[scroll_offset..end];

    let rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let abs_idx = scroll_offset + idx;
            let row_style = if Some(abs_idx) == state.selected_row {
                theme::cursor_row()
            } else {
                Style::default()
            };
            Row::new(vec![
                Cell::from(short_time(&e.event_time_iso)),
                Cell::from(e.event_type.clone()).style(event_type_style(&e.event_type)),
                Cell::from(e.component_name.clone()),
                Cell::from(short_uuid(&e.flow_file_uuid)),
                Cell::from(
                    e.relationship
                        .as_deref()
                        .or(e.details.as_deref())
                        .unwrap_or_default()
                        .to_string(),
                ),
            ])
            .style(row_style)
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(10), // time (HH:MM:SS)
            Constraint::Length(22), // type (longest: ATTRIBUTES_MODIFIED = 20 chars)
            Constraint::Length(24), // component
            Constraint::Length(14), // file uuid
            Constraint::Fill(1),    // relationship (ROUTE) or event details
        ],
    )
    .header(
        Row::new(vec!["time", "type", "component", "uuid", "details"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );
    frame.render_widget(table, area);
}

fn short_uuid(uuid: &str) -> String {
    let n = uuid.chars().count();
    if n <= 10 {
        uuid.to_string()
    } else {
        let head: String = uuid.chars().take(8).collect();
        let tail: String = uuid.chars().skip(n.saturating_sub(2)).collect();
        format!("{head}\u{2026}{tail}")
    }
}

fn short_time(iso: &str) -> String {
    if iso.len() >= 19 {
        let t = &iso[11..19];
        if t.as_bytes().get(2) == Some(&b':') && t.as_bytes().get(5) == Some(&b':') {
            return t.to_string();
        }
    }
    "--:--:--".to_string()
}

/// Colorize event types by category.
/// - DROP / EXPIRE → error
/// - ROUTE → accent
/// - RECEIVE / SEND / FETCH / DOWNLOAD → success
/// - FORK / JOIN / CREATE / CLONE / ATTRIBUTES_MODIFIED / CONTENT_MODIFIED → muted
/// - anything else → default
fn event_type_style(event_type: &str) -> ratatui::style::Style {
    use ratatui::style::Style;
    match event_type {
        "DROP" | "EXPIRE" => theme::error().add_modifier(Modifier::BOLD),
        "ROUTE" => theme::accent(),
        "RECEIVE" | "SEND" | "FETCH" | "DOWNLOAD" => theme::success(),
        "FORK" | "JOIN" | "CREATE" | "CLONE" | "ATTRIBUTES_MODIFIED" | "CONTENT_MODIFIED" => {
            theme::muted()
        }
        _ => Style::default(),
    }
}

fn render_empty_state(frame: &mut Frame, area: Rect) {
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "Events \u{2014} cluster-wide provenance search".to_string(),
            theme::accent().add_modifier(Modifier::BOLD),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled(
            "Set filters above, then press Enter to run.".to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center),
        Line::from(""),
        Line::from(Span::styled("Typical queries:".to_string(), theme::muted()))
            .alignment(Alignment::Center),
        Line::from(Span::styled(
            "  t=last 15m  T=DROP,EXPIRE    \u{2192} what dropped recently?".to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "  t=last 1h   s=UpdateRecord   \u{2192} what has this processor touched?".to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center),
        Line::from(Span::styled(
            "  u=8f2c\u{2026}                      \u{2192} jump straight to Tracer instead"
                .to_string(),
            theme::muted(),
        ))
        .alignment(Alignment::Center),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_detail_pane(frame: &mut Frame, area: Rect, state: &EventsState) {
    let inner = area;

    let Some(e) = state.selected_event() else {
        let hint = if state.events.is_empty() {
            "".to_string()
        } else {
            "press ↑/↓ to select a row for detail".to_string()
        };
        let p = Paragraph::new(Span::styled(hint, theme::muted())).alignment(Alignment::Center);
        frame.render_widget(p, inner);
        return;
    };

    let type_style = event_type_style(&e.event_type);
    let header = Line::from(vec![
        Span::styled(
            e.event_type.clone(),
            type_style.add_modifier(Modifier::BOLD),
        ),
        Span::raw(" \u{00b7} "),
        Span::styled(e.component_name.clone(), theme::accent()),
        Span::raw(" \u{00b7} "),
        Span::styled(short_time(&e.event_time_iso), theme::muted()),
        Span::raw(" \u{00b7} "),
        Span::styled(format!("flowfile {}", e.flow_file_uuid), theme::muted()),
    ]);

    let relationship_line = Line::from(vec![
        Span::styled("relationship".to_string(), theme::muted()),
        Span::raw("   "),
        Span::raw(
            e.relationship
                .clone()
                .unwrap_or_else(|| "(none)".to_string()),
        ),
    ]);
    let component_line = Line::from(vec![
        Span::styled("component  ".to_string(), theme::muted()),
        Span::raw(e.component_name.clone()),
        Span::raw("   "),
        Span::styled("group  ".to_string(), theme::muted()),
        Span::raw(e.group_id.clone()),
    ]);
    let hints_line = Line::from(Span::styled(
        "g t trace lineage \u{00b7} g b open in browser \u{00b7} Esc back \u{00b7} c copy uuid"
            .to_string(),
        theme::muted(),
    ));

    let lines = vec![
        header,
        Line::from(""),
        relationship_line,
        component_line,
        Line::from(""),
        hints_line,
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_to_string(state: &EventsState) -> String {
        let cfg = crate::timestamp::TimestampConfig::default();
        let backend = TestBackend::new(100, 24);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), state, &cfg)).unwrap();
        format!("{}", terminal.backend())
    }

    #[test]
    fn events_empty_state_renders() {
        let state = EventsState::new();
        insta::with_settings!({
            filters => vec![(r"last \d+[smhd] ago", "last __ ago")],
        }, {
            assert_snapshot!("events_empty_state", render_to_string(&state));
        });
    }

    #[test]
    fn events_running_state_renders() {
        let mut state = EventsState::new();
        state.status = EventsQueryStatus::Running {
            query_id: Some("q-1".into()),
            submitted_at: SystemTime::UNIX_EPOCH,
            percent: 45,
        };
        insta::with_settings!({
            filters => vec![(r"last \d+[smhd] ago", "last __ ago")],
        }, {
            assert_snapshot!("events_running_state", render_to_string(&state));
        });
    }

    #[test]
    fn events_failed_state_renders() {
        let mut state = EventsState::new();
        state.status = EventsQueryStatus::Failed {
            error: "connection refused".into(),
        };
        insta::with_settings!({
            filters => vec![(r"last \d+[smhd] ago", "last __ ago")],
        }, {
            assert_snapshot!("events_failed_state", render_to_string(&state));
        });
    }

    #[test]
    fn events_done_with_results_renders() {
        use crate::client::ProvenanceEventSummary;
        let mut state = EventsState::new();
        state.status = EventsQueryStatus::Done {
            fetched_at: SystemTime::UNIX_EPOCH,
            truncated: false,
            took_ms: 842,
        };
        state.events = vec![
            ProvenanceEventSummary {
                event_id: 1,
                event_time_iso: "2026-04-13T08:12:15Z".into(),
                event_type: "DROP".into(),
                component_id: "proc-1".into(),
                component_name: "ControlRate".into(),
                component_type: "PROCESSOR".into(),
                group_id: "noisy-pipeline".into(),
                flow_file_uuid: "8f2ce90a-019d-1000-ffff-ffffe8c7c7a9".into(),
                relationship: Some("failure".into()),
                details: None,
            },
            ProvenanceEventSummary {
                event_id: 2,
                event_time_iso: "2026-04-13T08:12:16Z".into(),
                event_type: "ROUTE".into(),
                component_id: "proc-2".into(),
                component_name: "UpdateRecord".into(),
                component_type: "PROCESSOR".into(),
                group_id: "healthy-pipeline".into(),
                flow_file_uuid: "3b0e1234-019d-1000-ffff-ffffabcd1212".into(),
                relationship: Some("matched".into()),
                details: None,
            },
        ];
        insta::with_settings!({
            filters => vec![(r"last \d+[smhd] ago", "last __ ago")],
        }, {
            assert_snapshot!("events_done_with_results", render_to_string(&state));
        });
    }

    #[test]
    fn events_detail_pane_with_selected_row_renders() {
        use crate::client::ProvenanceEventSummary;
        let mut state = EventsState::new();
        state.status = EventsQueryStatus::Done {
            fetched_at: SystemTime::UNIX_EPOCH,
            truncated: false,
            took_ms: 100,
        };
        state.events = vec![ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T08:12:15Z".into(),
            event_type: "DROP".into(),
            component_id: "proc-1".into(),
            component_name: "ControlRate".into(),
            component_type: "PROCESSOR".into(),
            group_id: "noisy-pipeline".into(),
            flow_file_uuid: "8f2ce90a-019d-1000-ffff-ffffe8c7c7a9".into(),
            relationship: Some("failure".into()),
            details: None,
        }];
        state.selected_row = Some(0);
        insta::with_settings!({
            filters => vec![(r"last \d+[smhd] ago", "last __ ago")],
        }, {
            assert_snapshot!(
                "events_detail_pane_with_selected_row",
                render_to_string(&state)
            );
        });
    }
}
