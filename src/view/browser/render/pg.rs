//! Process Group detail renderer.
//!
//! Phase 7 wraps the existing labeled-sections content in a bordered
//! outer `Panel`. PG detail has no focusable sub-sections, so the
//! `_detail_focus` parameter is threaded for signature consistency
//! with processor / controller_service renderers and currently
//! ignored.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ProcessGroupDetail;
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus};
use crate::widget::panel::Panel;
use crate::widget::severity::{format_severity_label, severity_style};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    state: &BrowserState,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    _detail_focus: &DetailFocus,
) {
    // Outer panel: " <name> · process group "
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Split: content (fill) + hints strip (1 line).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Fill(1), Constraint::Length(1)])
        .split(inner);

    render_content(frame, rows[0], d, state, bulletins);
    render_hints_strip(frame, rows[1]);
}

/// Build the outer panel title: ` <name> · process group `.
fn build_header_title(d: &ProcessGroupDetail) -> Line<'_> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            d.name.as_str(),
            theme::accent().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("process group", theme::muted()),
        Span::raw(" "),
    ])
}

fn render_content(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    state: &BrowserState,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Processors line: counts by state.
    lines.push(Line::from(vec![
        Span::styled("Processors  ".to_string(), theme::muted()),
        Span::raw(format!("{} running", d.running)),
        Span::raw("  "),
        Span::raw(format!("{} stopped", d.stopped)),
        Span::raw("  "),
        Span::raw(format!("{} invalid", d.invalid)),
        Span::raw("  "),
        Span::raw(format!("{} disabled", d.disabled)),
    ]));

    // Threads line.
    lines.push(Line::from(vec![
        Span::styled("Threads     ".to_string(), theme::muted()),
        Span::raw(format!("{} active", d.active_threads)),
    ]));

    // Queued line.
    lines.push(Line::from(vec![
        Span::styled("Queued      ".to_string(), theme::muted()),
        Span::raw(format!(
            "{} ffiles · {}",
            d.flow_files_queued, d.queued_display
        )),
    ]));

    // Controller services section.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Controller services ({})", d.controller_services.len()),
        theme::accent(),
    )));
    for cs in d.controller_services.iter().take(8) {
        lines.push(Line::from(format!(
            "  {state}  {name}   {type_}",
            state = cs.state,
            name = cs.name,
            type_ = cs.type_short
        )));
    }
    if d.controller_services.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  …{} more", d.controller_services.len() - 8),
            theme::muted(),
        )));
    }

    // Child groups section.
    let kids = state.child_process_groups(&d.id);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Child groups ({})", kids.len()),
        theme::accent(),
    )));
    for kid in kids.iter().take(8) {
        lines.push(Line::from(format!(
            "  {name}   {running} run · {stopped} stop · {invalid} invalid",
            name = kid.name,
            running = kid.running,
            stopped = kid.stopped,
            invalid = kid.invalid,
        )));
    }
    if kids.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  …{} more", kids.len() - 8),
            theme::muted(),
        )));
    }

    // Recent bulletins section.
    let recent = crate::view::bulletins::state::recent_for_group_id(bulletins, &d.id, 3);
    // Total count for the header (walks the ring once more — small ring, cheap).
    let total_in_pg = bulletins.iter().filter(|b| b.group_id == d.id).count();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Recent bulletins ({total_in_pg} in this PG)"),
        theme::accent(),
    )));
    for b in &recent {
        let sev = format_severity_label(&b.level);
        let sev_style = severity_style(&b.level);
        let stripped = crate::view::bulletins::state::strip_component_prefix(&b.message);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(sev, sev_style),
            Span::raw("  "),
            Span::raw(b.source_name.clone()),
            Span::raw("  "),
            Span::raw(stripped.to_string()),
        ]));
    }

    frame.render_widget(Paragraph::new(lines), area);
}

fn render_hints_strip(frame: &mut Frame, area: Rect) {
    let text = "↑/↓ nav · Enter drill in · e properties · c copy id";
    frame.render_widget(Paragraph::new(text).style(theme::muted()), area);
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn pg_detail_with_cs_list() {
        let d = ProcessGroupDetail {
            id: "ingest".into(),
            name: "ingest".into(),
            parent_group_id: Some("root".into()),
            running: 3,
            stopped: 1,
            invalid: 0,
            disabled: 0,
            active_threads: 1,
            flow_files_queued: 4,
            bytes_queued: 2048,
            queued_display: "4 / 2 KB".into(),
            controller_services: vec![
                crate::client::ControllerServiceSummary {
                    id: "cs1".into(),
                    name: "http-pool".into(),
                    type_short: "StandardRestrictedSSLContextService".into(),
                    state: "ENABLED".into(),
                },
                crate::client::ControllerServiceSummary {
                    id: "cs2".into(),
                    name: "kafka-brokers".into(),
                    type_short: "Kafka3ConnectionService".into(),
                    state: "DISABLED".into(),
                },
            ],
        };
        let state = BrowserState::new();
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &DetailFocus::Tree))
            .unwrap();
        assert_snapshot!("pg_detail_with_cs_list", format!("{}", terminal.backend()));
    }
}
