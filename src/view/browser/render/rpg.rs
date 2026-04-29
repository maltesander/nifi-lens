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
//! │┌ Input ports  N ──────────────────────────────┐          │
//! ││ NAME      STATE     TASKS  COMMENTS          │          │
//! ││ ...                                          │          │
//! │└──────────────────────────────────────────────┘          │
//! │┌ Output ports  N ─────────────────────────────┐          │
//! ││ NAME      STATE     TASKS  COMMENTS          │          │
//! ││ ...                                          │          │
//! │└──────────────────────────────────────────────┘          │
//! └──────────────────────────────────────────────────────────┘
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
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::client::browser::{RemotePortSummary, RemoteProcessGroupDetail};
use crate::theme;
use crate::view::browser::state::BrowserState;
use crate::widget::panel::Panel;

pub fn render(frame: &mut Frame, area: Rect, d: &RemoteProcessGroupDetail, state: &BrowserState) {
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Identity needs ≥7 lines for the full header; ports panels get the
    // remaining space, split evenly so neither table dominates.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(10),
            Constraint::Fill(1),
            Constraint::Fill(1),
        ])
        .split(inner);

    render_identity(frame, rows[0], d, state);
    render_ports_panel(frame, rows[1], " Input ports ", &d.input_ports);
    render_ports_panel(frame, rows[2], " Output ports ", &d.output_ports);
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
        for err in &d.validation_errors {
            lines.push(Line::from(vec![
                Span::raw("  • "),
                Span::styled(truncate(err, value_w), theme::error()),
            ]));
        }
        if !d.comments.is_empty() {
            lines.push(Line::from(vec![
                label("comments"),
                Span::raw(truncate(&d.comments, value_w)),
            ]));
        }
        lines
    });
}

fn render_ports_panel(
    frame: &mut Frame,
    area: Rect,
    title: &'static str,
    ports: &[RemotePortSummary],
) {
    let total = ports.len();
    let panel = Panel::new(title)
        .right(Line::from(format!(" {total} ")))
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    if ports.is_empty() {
        let placeholder = if title.trim() == "Input ports" {
            "(no input ports)"
        } else {
            "(no output ports)"
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(placeholder, theme::muted()))),
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
    let table = Table::new(rows_data, widths).header(header);
    frame.render_widget(table, inner);
}

/// Style for a remote port `run_status` value. Mirrors the ProcessorStatus
/// idea on a smaller domain — RPG remote ports synthesise three values:
/// `RUNNING` (transmitting), `STOPPED`, `MISSING` (port no longer exists
/// on the remote NiFi).
fn port_run_status_style(run_status: &str) -> ratatui::style::Style {
    match run_status {
        "RUNNING" => theme::success(),
        "MISSING" => theme::error(),
        _ => theme::muted(),
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
            render(f, f.area(), d, state);
        })
        .unwrap();
        format!("{}", term.backend())
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
}
