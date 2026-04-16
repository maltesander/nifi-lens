//! Processor detail renderer.
//!
//! Phase 7 layout:
//!
//! ```text
//! ┌ <name> · processor · <state> ──────────┐
//! │┌ Identity ───────────────────────┐     │
//! ││ type / bundle / schedule        │     │
//! │└─────────────────────────────────┘     │
//! │┌ Properties  N ──────────────────┐     │  ← focusable
//! ││ KEY              VALUE          │     │
//! ││ ...scrollable Table...          │     │
//! │└─────────────────────────────────┘     │
//! │┌ Validation errors  N ───────────┐     │  ← optional, shown when errors present
//! ││ error message 1                 │     │
//! ││ ...                             │     │
//! │└─────────────────────────────────┘     │
//! │┌ Recent bulletins  N ────────────┐     │  ← focusable
//! ││ ...scrollable Table...          │     │
//! │└─────────────────────────────────┘     │
//! └────────────────────────────────────────┘
//! ```
//!
//! The Properties and Recent bulletins sub-panels flip their border to
//! thick + accent when the corresponding `DetailSection` holds focus, and
//! their Table widget selects the row from `DetailFocus::rows[idx]`.
//! Key hints live in the sticky footer hint bar, not inside the pane.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::client::{BulletinSnapshot, NodeKind, ProcessorDetail};
use crate::layout;
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus, DetailSection, DetailSections};
use crate::widget::panel::Panel;
use crate::widget::severity::{format_severity_label, severity_style};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    _state: &BrowserState,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
) {
    // 1. Outer panel: " <name> · processor · <state> "
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // 2. Inner vertical layout.
    //    identity: 4 rows (2 borders + 2 content lines)
    //    properties: fill — takes the bulk
    //    recent bulletins: 8 rows (2 borders + 6 content rows)
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),
            Constraint::Fill(1),
            Constraint::Length(8),
        ])
        .split(inner);

    render_identity_panel(frame, rows[0], d);
    render_properties_and_validation(frame, rows[1], d, detail_focus);
    render_recent_bulletins_panel(frame, rows[2], d, bulletins, detail_focus);
}

/// Build the outer panel title: ` <name> · processor · <run_status> `.
fn build_header_title(d: &ProcessorDetail) -> Line<'_> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(d.name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("processor", theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(d.run_status.as_str(), run_state_style(&d.run_status)),
        Span::raw(" "),
    ])
}

fn run_state_style(run_status: &str) -> Style {
    match run_status.to_ascii_uppercase().as_str() {
        "RUNNING" => theme::success(),
        "STOPPED" => theme::warning(),
        "INVALID" => theme::error(),
        "DISABLED" => theme::disabled(),
        "VALIDATING" => theme::info(),
        _ => Style::default(),
    }
}

fn render_identity_panel(frame: &mut Frame, area: Rect, d: &ProcessorDetail) {
    let block = Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("type   ", theme::muted()),
            Span::raw(truncate(
                &d.type_name,
                inner.width.saturating_sub(7) as usize,
            )),
        ]),
        Line::from(vec![
            Span::styled("bundle ", theme::muted()),
            Span::raw(truncate(&d.bundle, inner.width.saturating_sub(7) as usize)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the Properties sub-panel and, when the processor has validation
/// errors, a bordered panel listing them below Properties.
fn render_properties_and_validation(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    detail_focus: &DetailFocus,
) {
    let has_validation = !d.validation_errors.is_empty();
    let sections = DetailSections::for_node_detail(NodeKind::Processor, has_validation);
    let val_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ValidationErrors);
    let is_val_focused = val_idx
        .is_some_and(|i| matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == i));

    let constraints: Vec<Constraint> = if has_validation {
        let panel_height = (d
            .validation_errors
            .len()
            .min(layout::VALIDATION_ERROR_ROWS_MAX)
            + 2) as u16;
        vec![Constraint::Fill(1), Constraint::Length(panel_height)]
    } else {
        vec![Constraint::Fill(1)]
    };
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let val_x_offset = val_idx
        .and_then(|i| {
            if let DetailFocus::Section { x_offsets, .. } = detail_focus {
                Some(x_offsets[i])
            } else {
                None
            }
        })
        .unwrap_or(0);

    render_properties_panel(frame, chunks[0], d, detail_focus);
    if has_validation {
        render_validation_errors_panel(frame, chunks[1], d, is_val_focused, val_x_offset);
    }
}

fn render_validation_errors_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    is_focused: bool,
    x_offset: usize,
) {
    let count = d.validation_errors.len();
    let panel = Panel::new(" Validation errors ")
        .right(Line::from(format!(" {count} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let lines: Vec<Line> = d
        .validation_errors
        .iter()
        .map(|e| Line::from(Span::styled(e.as_str(), theme::error())))
        .collect();
    frame.render_widget(Paragraph::new(lines).scroll((0, x_offset as u16)), inner);
}

fn render_properties_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    detail_focus: &DetailFocus,
) {
    let sections =
        DetailSections::for_node_detail(NodeKind::Processor, !d.validation_errors.is_empty());
    let props_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::Properties)
        .unwrap_or(0);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == props_idx
    );
    let x_offset = if is_focused {
        if let DetailFocus::Section { x_offsets, .. } = detail_focus {
            x_offsets[props_idx]
        } else {
            0
        }
    } else {
        0
    };

    let total = d.properties.len();
    let panel = Panel::new(" Properties ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let header = Row::new(vec![Cell::from("KEY"), Cell::from("VALUE")])
        .style(theme::muted().add_modifier(Modifier::BOLD));

    let rows_data: Vec<Row> = d
        .properties
        .iter()
        .map(|(k, v)| {
            Row::new(vec![
                Cell::from(k.clone()),
                Cell::from(char_skip(v, x_offset)),
            ])
        })
        .collect();
    let widths = layout::detail_row_constraints();
    let table = Table::new(rows_data, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut state = TableState::default();
    if let DetailFocus::Section { rows, .. } = detail_focus
        && is_focused
    {
        state.select(Some(rows[props_idx]));
    }
    frame.render_stateful_widget(table, inner, &mut state);
}

fn render_recent_bulletins_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
) {
    let sections =
        DetailSections::for_node_detail(NodeKind::Processor, !d.validation_errors.is_empty());
    let bul_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::RecentBulletins)
        .unwrap_or(1);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == bul_idx
    );
    let x_offset = if is_focused {
        if let DetailFocus::Section { x_offsets, .. } = detail_focus {
            x_offsets[bul_idx]
        } else {
            0
        }
    } else {
        0
    };

    // Collect ALL matching bulletins (no cap) — the Table scrolls.
    let matching: Vec<&BulletinSnapshot> =
        bulletins.iter().filter(|b| b.source_id == d.id).collect();
    let total = matching.len();

    let panel = Panel::new(" Recent bulletins ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let rows_data: Vec<Row> = matching
        .iter()
        .map(|b| {
            let sev_label = format_severity_label(&b.level);
            let sev_style = severity_style(&b.level);
            Row::new(vec![
                Cell::from(short_time(&b.timestamp_iso, &b.timestamp_human)),
                Cell::from(sev_label).style(sev_style),
                {
                    let msg = crate::view::bulletins::state::strip_component_prefix(&b.message)
                        .to_string();
                    Cell::from(char_skip(&msg, x_offset))
                },
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
        Constraint::Length(6),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows_data, widths).row_highlight_style(theme::cursor_row());

    let mut state = TableState::default();
    if let DetailFocus::Section { rows, .. } = detail_focus
        && is_focused
    {
        state.select(Some(rows[bul_idx]));
    }
    frame.render_stateful_widget(table, inner, &mut state);
}

/// Extract `HH:MM:SS` from an ISO-8601 timestamp, falling back to a
/// short slice of the human-readable form when the ISO field is empty.
fn short_time(iso: &str, human: &str) -> String {
    if iso.len() >= 19 {
        let t = &iso[11..19];
        if t.as_bytes().get(2) == Some(&b':') && t.as_bytes().get(5) == Some(&b':') {
            return t.to_string();
        }
    }
    // Fallback: if the human string has `HH:MM:SS` somewhere, grab it.
    for i in 0..human.len().saturating_sub(7) {
        let slice = &human[i..i + 8];
        if slice.as_bytes()[2] == b':' && slice.as_bytes()[5] == b':' {
            return slice.to_string();
        }
    }
    "--:--:--".to_string()
}

fn truncate(s: &str, max: usize) -> String {
    if max == 0 {
        return String::new();
    }
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

/// Skip the first `n` Unicode scalar values from `s`, returning the remainder.
fn char_skip(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::client::BulletinSnapshot;
    use crate::view::browser::state::MAX_DETAIL_SECTIONS;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn seeded_detail() -> (ProcessorDetail, BrowserState, VecDeque<BulletinSnapshot>) {
        let d = ProcessorDetail {
            id: "put-kafka-1".into(),
            name: "PutKafka".into(),
            type_name: "org.apache.nifi.processors.kafka.pubsub.PublishKafka_2_6".into(),
            bundle: "org.apache.nifi:nifi-kafka-2-6-nar:2.8.0".into(),
            run_status: "RUNNING".into(),
            scheduling_strategy: "TIMER_DRIVEN".into(),
            scheduling_period: "1 sec".into(),
            concurrent_tasks: 2,
            run_duration_ms: 25,
            penalty_duration: "30 sec".into(),
            yield_duration: "1 sec".into(),
            bulletin_level: "WARN".into(),
            properties: (0..8)
                .map(|i| (format!("Property-{i}"), format!("value-{i}")))
                .collect(),
            validation_errors: vec![],
        };
        let state = BrowserState::new();
        let mut bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        for (i, level) in ["ERROR", "WARN", "INFO", "WARN"].iter().enumerate() {
            bulletins.push_back(BulletinSnapshot {
                id: (100 + i) as i64,
                level: (*level).into(),
                message: format!("PutKafka[id=abc] message {i} with details"),
                source_id: "put-kafka-1".into(),
                source_name: "PutKafka".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: format!("2026-04-13T10:14:{:02}.000Z", 10 + i),
                timestamp_human: format!("04/13/2026 10:14:{:02} UTC", 10 + i),
            });
        }
        (d, state, bulletins)
    }

    fn render_snapshot(detail_focus: &DetailFocus) -> String {
        let (d, state, bulletins) = seeded_detail();
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &d, &state, &bulletins, detail_focus);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn processor_detail_tree_focused() {
        let out = render_snapshot(&DetailFocus::Tree);
        assert_snapshot!("processor_detail_tree_focused", out);
    }

    #[test]
    fn processor_detail_properties_focused() {
        let focus = DetailFocus::Section {
            idx: 0,
            rows: [1, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let out = render_snapshot(&focus);
        assert_snapshot!("processor_detail_properties_focused", out);
    }

    #[test]
    fn processor_detail_recent_bulletins_focused() {
        let focus = DetailFocus::Section {
            idx: 1,
            rows: [0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let out = render_snapshot(&focus);
        assert_snapshot!("processor_detail_recent_bulletins_focused", out);
    }
}
