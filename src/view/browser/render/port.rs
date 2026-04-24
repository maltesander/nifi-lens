//! Port detail renderer.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::client::{BulletinSnapshot, NodeKind, PortDetail, PortKind};
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus, DetailSection, DetailSections};
use crate::widget::panel::Panel;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &PortDetail,
    state: &BrowserState,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
) {
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(7), Constraint::Fill(1)])
        .split(inner);

    render_identity(frame, rows[0], d, state);
    render_recent_bulletins(frame, rows[1], d, bulletins, detail_focus);
}

fn build_header_title(d: &PortDetail) -> Line<'_> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(d.name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(format!("{} port", d.kind.label()), theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(d.state.as_str(), state_style(&d.state)),
        Span::raw(" "),
    ])
}

fn state_style(state: &str) -> Style {
    match state {
        "RUNNING" => theme::success().add_modifier(Modifier::BOLD),
        "STOPPED" => theme::warning(),
        "DISABLED" => theme::muted(),
        _ => theme::info(),
    }
}

fn render_identity(frame: &mut Frame, area: Rect, d: &PortDetail, state: &BrowserState) {
    let block = Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let parent = match d.parent_group_id.as_deref() {
        Some(raw) => state
            .resolve_id(raw)
            .map(|r| r.name)
            .unwrap_or_else(|| raw.to_string()),
        None => "(root)".to_string(),
    };
    let w = inner.width.saturating_sub(18) as usize;
    let comments = if d.comments.is_empty() {
        "—".to_string()
    } else {
        truncate(&d.comments, w)
    };
    let lines = vec![
        Line::from(vec![
            Span::styled("kind            ", theme::muted()),
            Span::raw(d.kind.label().to_string()),
        ]),
        Line::from(vec![
            Span::styled("state           ", theme::muted()),
            Span::styled(d.state.clone(), state_style(&d.state)),
        ]),
        Line::from(vec![
            Span::styled("parent group    ", theme::muted()),
            Span::raw(parent.to_string()),
        ]),
        Line::from(vec![
            Span::styled("comments        ", theme::muted()),
            Span::raw(comments),
        ]),
        Line::from(vec![
            Span::styled("concurrent tasks", theme::muted()),
            Span::raw(format!(" {}", d.concurrent_tasks)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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

fn render_recent_bulletins(
    frame: &mut Frame,
    area: Rect,
    d: &PortDetail,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
) {
    use crate::widget::severity::{format_severity_label, severity_style};
    let kind = match d.kind {
        PortKind::Input => NodeKind::InputPort,
        PortKind::Output => NodeKind::OutputPort,
    };
    let sections = DetailSections::for_node(kind);
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::RecentBulletins)
        .unwrap_or(0);
    let is_focused = matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == my_idx);
    let matching: Vec<_> = bulletins
        .iter()
        .rev()
        .filter(|b| b.source_id == d.id)
        .collect();
    let total = matching.len();
    let panel = Panel::new(" Recent bulletins ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let header = Row::new(vec![
        Cell::from("TIME"),
        Cell::from("SEV"),
        Cell::from("MESSAGE"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));
    let rows_data: Vec<Row> = matching
        .iter()
        .map(|b| {
            let sev_label = format_severity_label(&b.level);
            let sev_style = severity_style(&b.level);
            let msg = crate::view::bulletins::state::strip_component_prefix(&b.message).to_string();
            Row::new(vec![
                Cell::from(b.timestamp_human.clone()),
                Cell::from(sev_label).style(sev_style),
                Cell::from(msg),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(22),
        Constraint::Length(4),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows_data, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());
    let mut ts = TableState::default();
    if let DetailFocus::Section { rows, .. } = detail_focus
        && is_focused
    {
        ts.select(Some(rows[my_idx]));
    }
    frame.render_stateful_widget(table, inner, &mut ts);
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
    use insta::assert_snapshot;
    use ratatui::Terminal;

    #[test]
    fn port_detail_output_port_renders_without_comments() {
        let d = PortDetail {
            id: "out-1".into(),
            name: "downstream".into(),
            kind: PortKind::Output,
            state: "STOPPED".into(),
            comments: String::new(),
            concurrent_tasks: 1,
            parent_group_id: Some("ingest".into()),
        };
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let mut term = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &DetailFocus::Tree))
            .unwrap();
        assert_snapshot!(
            "port_detail_output_port_renders_without_comments",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn port_detail_input_port_renders() {
        let d = PortDetail {
            id: "in-1".into(),
            name: "external-ingest".into(),
            kind: PortKind::Input,
            state: "RUNNING".into(),
            comments: "accepts from edge agents".into(),
            concurrent_tasks: 3,
            parent_group_id: Some("root".into()),
        };
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let mut term = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &DetailFocus::Tree))
            .unwrap();
        assert_snapshot!(
            "port_detail_input_port_renders",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn port_detail_parent_group_resolved_to_name() {
        use crate::client::{NodeKind, NodeStatusSummary};
        use crate::view::browser::state::TreeNode;

        let pg_uuid = "deadbeef-cafe-f00d-face-b001b001b001";
        let d = PortDetail {
            id: "in-1".into(),
            name: "external-ingest".into(),
            kind: PortKind::Input,
            state: "RUNNING".into(),
            comments: "edge ingress".into(),
            concurrent_tasks: 3,
            parent_group_id: Some(pg_uuid.into()),
        };
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ProcessGroup,
            id: pg_uuid.into(),
            group_id: String::new(),
            name: "ingress-pg".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        });
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let mut term = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &DetailFocus::Tree))
            .unwrap();
        let out = format!("{}", term.backend());
        assert!(out.contains("ingress-pg"));
        assert!(!out.contains(pg_uuid));
        assert_snapshot!("port_detail_parent_group_resolved_to_name", out);
    }
}
