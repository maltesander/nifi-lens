//! Remote Process Group detail renderer.
//!
//! Layout:
//!
//! ```text
//! ┌ <name> · remote process group · <transmission> ──────────┐
//! │┌ Identity ────────────────────────────────────┐          │
//! ││ name / parent / target / secure / protocol / │          │
//! ││ transmission / validation                    │          │
//! │└──────────────────────────────────────────────┘          │
//! │┌ Validation errors  N ────────────────────────┐          │  ← optional
//! ││ error message 1                              │          │
//! ││ ...                                          │          │
//! │└──────────────────────────────────────────────┘          │
//! │┌ Input ports  N ──────────────────────────────┐          │
//! ││ NAME      STATE     TASKS  COMMENTS          │          │
//! ││ ...                                          │          │
//! │└──────────────────────────────────────────────┘          │
//! │┌ Output ports  N ─────────────────────────────┐          │
//! ││ NAME      STATE     TASKS  COMMENTS          │          │
//! ││ ...                                          │          │
//! │└──────────────────────────────────────────────┘          │
//! ```
//!
//! The Identity panel reserves the right-half sparkline strip via
//! `super::render_identity_panel_with_sparkline` — Task 12 lights it
//! up; until then the strip surfaces the standard `loading…`
//! placeholder.
//!
//! Input/Output port tables are display-only (read-only `v0.1`); the
//! `RemotePortSummary` rows are not part of the Browser arena and
//! cannot be focused or descended into.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::client::NodeKind;
use crate::client::browser::{RemotePortSummary, RemoteProcessGroupDetail};
use crate::client::status::PortStatus;
use crate::layout;
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus, DetailSection, DetailSections};
use crate::widget::panel::Panel;

/// Distinguishes the two ports panels — drives both the panel title and
/// the empty-state placeholder, replacing the prior title-string sniff.
#[derive(Debug, Clone, Copy)]
enum PortKind {
    Input,
    Output,
}

impl PortKind {
    fn title(self) -> &'static str {
        match self {
            Self::Input => " Input ports ",
            Self::Output => " Output ports ",
        }
    }

    fn empty_placeholder(self) -> &'static str {
        match self {
            Self::Input => "(no input ports)",
            Self::Output => "(no output ports)",
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &RemoteProcessGroupDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
) {
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Layout: Identity (10 rows; ≥7 lines for the full header + comments) +
    // optional Validation errors (dynamic, hidden when no errors) +
    // Input/Output ports (Min 3 each, split evenly so neither dominates).
    let has_validation = !d.validation_errors.is_empty();
    let sections = DetailSections::for_node_detail(NodeKind::RemoteProcessGroup, has_validation);

    let validation_h = if has_validation {
        (d.validation_errors
            .len()
            .min(layout::VALIDATION_ERROR_ROWS_MAX)
            + 2) as u16
    } else {
        0
    };

    let mut constraints: Vec<Constraint> = vec![Constraint::Length(10)];
    if validation_h > 0 {
        constraints.push(Constraint::Length(validation_h));
    }
    constraints.push(Constraint::Fill(1));
    constraints.push(Constraint::Fill(1));

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    render_identity(frame, rows[0], d, state);
    let mut idx = 1;
    if has_validation {
        render_validation_errors_panel(frame, rows[idx], d, &sections, detail_focus);
        idx += 1;
    }
    render_ports_panel(
        frame,
        rows[idx],
        PortKind::Input,
        &d.input_ports,
        &sections,
        detail_focus,
    );
    render_ports_panel(
        frame,
        rows[idx + 1],
        PortKind::Output,
        &d.output_ports,
        &sections,
        detail_focus,
    );
}

/// Build the outer panel title:
/// ` <name> · remote process group · <transmission_status> `.
fn build_header_title(d: &RemoteProcessGroupDetail) -> Line<'_> {
    let trans_style = if d.transmission_status == "Transmitting" {
        theme::accent()
    } else {
        theme::muted()
    };
    Line::from(vec![
        Span::raw(" "),
        Span::styled(d.name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("remote process group", theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(d.transmission_status.as_str(), trans_style),
        Span::raw(" "),
    ])
}

fn render_identity(
    frame: &mut Frame,
    area: Rect,
    d: &RemoteProcessGroupDetail,
    state: &BrowserState,
) {
    super::render_identity_panel_with_sparkline(frame, area, state.sparkline.as_ref(), |inner| {
        let parent_label = match d.parent_group_id.as_deref() {
            Some(raw) => state
                .resolve_id(raw)
                .map(|r| format!("{}  →", r.name))
                .unwrap_or_else(|| raw.to_string()),
            None => "(root)".to_string(),
        };
        let label = |k: &'static str| Span::styled(format!("{k:<20}"), theme::muted());
        let value_w = (inner.width as usize).saturating_sub(20);

        let validation_style = if d.validation_status == "VALID" {
            theme::success()
        } else {
            theme::error()
        };

        let mut lines: Vec<Line<'static>> = vec![
            Line::from(vec![label("name"), Span::raw(truncate(&d.name, value_w))]),
            Line::from(vec![label("parent group"), Span::raw(parent_label)]),
            Line::from(vec![
                label("target uri"),
                Span::raw(truncate(&d.target_uri, value_w)),
            ]),
            Line::from(vec![
                label("target secure"),
                Span::raw(if d.target_secure { "yes" } else { "no" }.to_string()),
            ]),
            Line::from(vec![
                label("transport protocol"),
                Span::raw(d.transport_protocol.clone()),
            ]),
            Line::from(vec![
                label("transmission"),
                Span::raw(d.transmission_status.clone()),
            ]),
            Line::from(vec![
                label("validation"),
                Span::styled(d.validation_status.clone(), validation_style),
            ]),
        ];
        if !d.comments.is_empty() {
            lines.push(Line::from(vec![
                label("comments"),
                Span::raw(truncate(&d.comments, value_w)),
            ]));
        }
        lines
    });
}

/// Renders the bullet list of validation errors in a dedicated bordered
/// panel below Identity. Caller must ensure `d.validation_errors` is
/// non-empty — the panel is laid out only in that case.
fn render_validation_errors_panel(
    frame: &mut Frame,
    area: Rect,
    d: &RemoteProcessGroupDetail,
    sections: &DetailSections,
    detail_focus: &DetailFocus,
) {
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ValidationErrors);
    let is_focused = my_idx
        .is_some_and(|i| matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == i));
    let x_offset = my_idx
        .and_then(|i| match detail_focus {
            DetailFocus::Section { x_offsets, .. } if is_focused => Some(x_offsets[i]),
            _ => None,
        })
        .unwrap_or(0);

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
        .take(layout::VALIDATION_ERROR_ROWS_MAX)
        .map(|e| Line::from(Span::styled(e.as_str(), theme::error())))
        .collect();
    frame.render_widget(Paragraph::new(lines).scroll((0, x_offset as u16)), inner);
}

fn render_ports_panel(
    frame: &mut Frame,
    area: Rect,
    kind: PortKind,
    ports: &[RemotePortSummary],
    sections: &DetailSections,
    detail_focus: &DetailFocus,
) {
    let target_section = match kind {
        PortKind::Input => DetailSection::InputPorts,
        PortKind::Output => DetailSection::OutputPorts,
    };
    let my_idx = sections.0.iter().position(|s| *s == target_section);
    let is_focused = my_idx
        .is_some_and(|i| matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == i));

    let total = ports.len();
    let panel = Panel::new(kind.title())
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    if ports.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                kind.empty_placeholder(),
                theme::muted(),
            ))),
            inner,
        );
        return;
    }

    let header = Row::new(vec![
        Cell::from("NAME"),
        Cell::from("STATE"),
        Cell::from("TASKS"),
        Cell::from("COMMENTS"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let rows_data: Vec<Row> = ports
        .iter()
        .map(|p| {
            Row::new(vec![
                Cell::from(p.name.clone()),
                Cell::from(Span::styled(
                    p.run_status.clone(),
                    port_run_status_style(&p.run_status),
                )),
                Cell::from(p.concurrent_tasks.to_string()),
                Cell::from(p.comments.clone()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Fill(2),
        Constraint::Length(8),
        Constraint::Length(5),
        Constraint::Fill(3),
    ];
    let table = Table::new(rows_data, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut table_state = TableState::default();
    if let (Some(i), DetailFocus::Section { rows, .. }) = (my_idx, detail_focus)
        && is_focused
    {
        table_state.select(Some(rows[i]));
    }
    frame.render_stateful_widget(table, inner, &mut table_state);
}

/// Style for a remote port `run_status` value. Defers to the typed
/// `PortStatus` helper for the standard NiFi port states (`RUNNING`,
/// `STOPPED`, `DISABLED`, `INVALID`, `VALIDATING`); RPGs additionally
/// synthesize `MISSING` (port no longer exists on the remote NiFi),
/// which falls outside `PortStatus` and is handled inline here.
fn port_run_status_style(run_status: &str) -> Style {
    if run_status.eq_ignore_ascii_case("MISSING") {
        return theme::error();
    }
    match PortStatus::from_wire(run_status) {
        PortStatus::Unknown => Style::default(),
        s => s.style(),
    }
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

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::client::browser::{RemotePortSummary, RemoteProcessGroupDetail};
    use crate::view::browser::state::BrowserState;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn seeded_detail() -> RemoteProcessGroupDetail {
        RemoteProcessGroupDetail {
            id: "rpg-1".into(),
            name: "MyRemoteSink".into(),
            parent_group_id: Some("pg-1".into()),
            target_uri: "https://nifi-east:8443/nifi".into(),
            target_secure: true,
            transport_protocol: "HTTP".into(),
            transmission_status: "Transmitting".into(),
            validation_status: "VALID".into(),
            validation_errors: vec![],
            comments: String::new(),
            input_ports: vec![RemotePortSummary {
                id: "ip-1".into(),
                name: "ingest".into(),
                run_status: "RUNNING".into(),
                concurrent_tasks: 4,
                comments: String::new(),
            }],
            output_ports: vec![],
            active_remote_input_port_count: 1,
            inactive_remote_input_port_count: 0,
            active_remote_output_port_count: 0,
            inactive_remote_output_port_count: 0,
        }
    }

    fn render_to_string(d: &RemoteProcessGroupDetail, state: &BrowserState) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), d, state, &DetailFocus::Tree);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    fn render_to_string_with_focus(
        d: &RemoteProcessGroupDetail,
        state: &BrowserState,
        detail_focus: &DetailFocus,
    ) -> String {
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), d, state, detail_focus);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    fn render_with_sparkline(
        width: u16,
        sparkline: Option<crate::view::browser::state::sparkline::SparklineState>,
    ) -> String {
        let d = seeded_detail();
        let mut state = BrowserState::new();
        state.sparkline = sparkline;
        let mut term = Terminal::new(TestBackend::new(width, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &d, &state, &DetailFocus::Tree);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    fn sample_rpg_series(buckets: usize) -> crate::client::history::StatusHistorySeries {
        use crate::client::history::{Bucket, StatusHistorySeries};
        StatusHistorySeries {
            buckets: (0..buckets)
                .map(|i| Bucket {
                    timestamp: std::time::SystemTime::now(),
                    in_count: ((i * 5) % 80) as u64,
                    out_count: ((i * 3) % 60) as u64,
                    queued_count: None,
                    task_time_ns: None,
                    bytes_per_sec: Some(((i * 2) % 8) as u64),
                })
                .collect(),
            generated_at: std::time::SystemTime::now(),
        }
    }

    #[test]
    fn rpg_sparkline_wide() {
        use crate::client::history::ComponentKind;
        use crate::view::browser::state::sparkline::SparklineState;
        let mut s = SparklineState::pending(ComponentKind::RemoteProcessGroup, "rpg-1".into());
        s.series = Some(sample_rpg_series(40));
        let out = render_with_sparkline(120, Some(s));
        assert_snapshot!("rpg_sparkline_wide", out);
    }

    #[test]
    fn rpg_identity_renders_loaded_with_one_input_port() {
        let d = seeded_detail();
        let state = BrowserState::new();
        let out = render_to_string(&d, &state);
        assert_snapshot!("rpg_identity_loaded", out);
    }

    #[test]
    fn rpg_identity_renders_invalid_with_validation_errors() {
        let mut d = seeded_detail();
        d.transmission_status = "Not Transmitting".into();
        d.validation_status = "INVALID".into();
        d.validation_errors = vec![
            "Target URI is not reachable".into(),
            "Authentication failed".into(),
        ];
        d.input_ports.clear();
        let state = BrowserState::new();
        let out = render_to_string(&d, &state);
        assert_snapshot!("rpg_identity_invalid_with_errors", out);
    }

    #[test]
    fn rpg_identity_renders_invalid_with_multiple_validation_errors() {
        let mut d = seeded_detail();
        d.transmission_status = "Not Transmitting".into();
        d.validation_status = "INVALID".into();
        d.validation_errors = vec![
            "Authentication failed".into(),
            "Target URI unreachable".into(),
            "Protocol mismatch".into(),
        ];
        d.input_ports.clear();
        let state = BrowserState::new();
        let out = render_to_string(&d, &state);
        assert_snapshot!("rpg_identity_invalid_with_multiple_errors", out);
    }

    #[test]
    fn rpg_identity_renders_with_input_and_output_ports() {
        let mut d = seeded_detail();
        d.input_ports.push(RemotePortSummary {
            id: "ip-2".into(),
            name: "events".into(),
            run_status: "STOPPED".into(),
            concurrent_tasks: 1,
            comments: String::new(),
        });
        d.output_ports.push(RemotePortSummary {
            id: "op-1".into(),
            name: "errors".into(),
            run_status: "MISSING".into(),
            concurrent_tasks: 0,
            comments: "remote port deleted".into(),
        });
        d.active_remote_input_port_count = 1;
        d.inactive_remote_input_port_count = 1;
        d.active_remote_output_port_count = 0;
        d.inactive_remote_output_port_count = 1;
        let state = BrowserState::new();
        let out = render_to_string(&d, &state);
        assert_snapshot!("rpg_identity_loaded_with_input_and_output_ports", out);
    }

    fn detail_with_two_input_two_output() -> RemoteProcessGroupDetail {
        let mut d = seeded_detail();
        d.input_ports.push(RemotePortSummary {
            id: "ip-2".into(),
            name: "events".into(),
            run_status: "STOPPED".into(),
            concurrent_tasks: 1,
            comments: String::new(),
        });
        d.output_ports.push(RemotePortSummary {
            id: "op-1".into(),
            name: "errors".into(),
            run_status: "RUNNING".into(),
            concurrent_tasks: 2,
            comments: String::new(),
        });
        d.output_ports.push(RemotePortSummary {
            id: "op-2".into(),
            name: "deadletter".into(),
            run_status: "STOPPED".into(),
            concurrent_tasks: 1,
            comments: String::new(),
        });
        d
    }

    fn focus_section(idx: usize, row: usize) -> DetailFocus {
        use crate::view::browser::state::MAX_DETAIL_SECTIONS;
        let mut rows = [0usize; MAX_DETAIL_SECTIONS];
        rows[idx] = row;
        DetailFocus::Section {
            idx,
            rows,
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        }
    }

    #[test]
    fn rpg_input_ports_focus_renders_thick_border_and_row_highlight() {
        let d = detail_with_two_input_two_output();
        let state = BrowserState::new();
        // No validation errors → idx 0 = InputPorts.
        let out = render_to_string_with_focus(&d, &state, &focus_section(0, 1));
        assert_snapshot!("rpg_input_ports_focused_row_1", out);
    }

    #[test]
    fn rpg_output_ports_focus_renders_thick_border_and_row_highlight() {
        let d = detail_with_two_input_two_output();
        let state = BrowserState::new();
        // No validation errors → idx 1 = OutputPorts.
        let out = render_to_string_with_focus(&d, &state, &focus_section(1, 0));
        assert_snapshot!("rpg_output_ports_focused_row_0", out);
    }

    #[test]
    fn rpg_validation_errors_focus_renders_thick_border() {
        let mut d = detail_with_two_input_two_output();
        d.validation_status = "INVALID".into();
        d.validation_errors = vec![
            "Target URI unreachable".into(),
            "Authentication failed".into(),
        ];
        let state = BrowserState::new();
        // With validation present → idx 0 = ValidationErrors.
        let out = render_to_string_with_focus(&d, &state, &focus_section(0, 0));
        assert_snapshot!("rpg_validation_errors_focused", out);
    }
}
