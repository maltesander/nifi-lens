//! Controller Service detail renderer.
//!
//! Phase 7 layout:
//!
//! ```text
//! ┌ <name> · controller service · <state> ─┐
//! │┌ Identity ─────────────────────┐       │
//! ││ type / bundle / parent /      │       │
//! ││ comments / flags              │       │
//! │└───────────────────────────────┘       │
//! │┌ Properties  N ────────────────┐       │  ← focusable
//! ││ KEY              VALUE        │       │
//! ││ ...scrollable Table...        │       │
//! │└───────────────────────────────┘       │
//! │┌ Validation errors  N ─────────┐       │  ← optional, shown when errors present
//! ││ error message 1               │       │
//! ││ ...                           │       │
//! │└───────────────────────────────┘       │
//! │┌ Referencing components  N ────┐       │  ← focusable
//! ││ STATE  KIND  NAME  THR        │       │
//! │└───────────────────────────────┘       │
//! │┌ Recent bulletins  N ──────────┐       │  ← focusable
//! ││ TIME  SEV  MESSAGE            │       │
//! │└───────────────────────────────┘       │
//! └────────────────────────────────────────┘
//! ```
//!
//! Key hints live in the sticky footer hint bar, not inside the pane.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::client::{ControllerServiceDetail, NodeKind};
use crate::layout;
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus, DetailSection, DetailSections};
use crate::widget::panel::Panel;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    state: &BrowserState,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    detail_focus: &DetailFocus,
) {
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let has_validation = !d.validation_errors.is_empty();
    let sections = DetailSections::for_node_detail(NodeKind::ControllerService, has_validation);

    let mut constraints: Vec<Constraint> = Vec::new();
    constraints.push(Constraint::Length(7)); // Identity (5 content lines + 2 borders)
    constraints.push(Constraint::Fill(1)); // Properties
    if has_validation {
        let h = (d
            .validation_errors
            .len()
            .min(layout::VALIDATION_ERROR_ROWS_MAX)
            + 2) as u16;
        constraints.push(Constraint::Length(h));
    }
    constraints.push(Constraint::Length(6)); // Referencing components (2 borders + header + 3 rows)
    constraints.push(Constraint::Length(6)); // Recent bulletins (2 borders + 4 rows)

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    let mut idx = 0;
    render_identity_panel(frame, rows[idx], d, state);
    idx += 1;
    render_properties_panel(frame, rows[idx], d, state, detail_focus, &sections);
    idx += 1;
    if has_validation {
        render_validation_errors_panel(frame, rows[idx], d, detail_focus, &sections);
        idx += 1;
    }
    render_referencing_components_panel(frame, rows[idx], d, detail_focus, &sections);
    idx += 1;
    render_recent_bulletins_panel(frame, rows[idx], d, bulletins, detail_focus, &sections);
}

/// Build the outer panel title: ` <name> · controller service · <state> `.
fn build_header_title(d: &ControllerServiceDetail) -> Line<'_> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(d.name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("controller service", theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled(d.state.as_str(), state_style(&d.state)),
        Span::raw(" "),
    ])
}

fn state_style(state: &str) -> Style {
    match state {
        "ENABLED" => theme::success().add_modifier(Modifier::BOLD),
        "DISABLED" => theme::muted(),
        _ => theme::warning(),
    }
}

fn render_identity_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    state: &BrowserState,
) {
    let block = Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let parent = match d.parent_group_id.as_deref() {
        Some(raw) => state
            .resolve_id(raw)
            .map(|r| r.name)
            .unwrap_or_else(|| raw.to_string()),
        None => "(controller)".to_string(),
    };
    let w = inner.width.saturating_sub(9) as usize;
    let comments = if d.comments.is_empty() {
        "—".to_string()
    } else {
        truncate(&d.comments, w)
    };

    let flag = |on: bool, label: &str| {
        if on {
            Span::styled(label.to_string(), theme::accent())
        } else {
            Span::styled(label.to_string(), theme::muted())
        }
    };

    let lines = vec![
        Line::from(vec![
            Span::styled("type     ", theme::muted()),
            Span::raw(truncate(&d.type_name, w)),
        ]),
        Line::from(vec![
            Span::styled("bundle   ", theme::muted()),
            Span::raw(truncate(&d.bundle, w)),
        ]),
        Line::from(vec![
            Span::styled("parent   ", theme::muted()),
            Span::raw(truncate(&parent, w)),
        ]),
        Line::from(vec![
            Span::styled("comments ", theme::muted()),
            Span::raw(comments),
        ]),
        Line::from(vec![
            Span::styled("flags    ", theme::muted()),
            flag(d.restricted, "restricted"),
            Span::styled(" · ", theme::muted()),
            flag(d.deprecated, "deprecated"),
            Span::styled(" · ", theme::muted()),
            flag(d.persists_state, "persists state"),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_validation_errors_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ValidationErrors)
        .unwrap_or(0);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );
    let x_offset = match detail_focus {
        DetailFocus::Section { x_offsets, .. } if is_focused => x_offsets[my_idx],
        _ => 0,
    };

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
    d: &ControllerServiceDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
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
            let value_cell = match super::format_property_value(v, state) {
                Some(formatted) => Cell::from(formatted),
                None => Cell::from(char_skip(v, x_offset)),
            };
            Row::new(vec![Cell::from(k.clone()), value_cell])
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

fn render_referencing_components_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    use crate::client::ReferencingKind;
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ReferencingComponents)
        .unwrap_or(0);
    let is_focused = matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == my_idx);
    let x_offset = match detail_focus {
        DetailFocus::Section { x_offsets, .. } if is_focused => x_offsets[my_idx],
        _ => 0,
    };

    let total = d.referencing_components.len();
    let panel = Panel::new(" Referencing components ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let header = Row::new(vec![
        Cell::from("STATE"),
        Cell::from("KIND"),
        Cell::from("NAME"),
        Cell::from("THR"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let kind_abbrev = |k: &ReferencingKind| -> &'static str {
        match k {
            ReferencingKind::Processor => "PROC",
            ReferencingKind::ControllerService => "CS",
            ReferencingKind::ReportingTask => "REPT",
            ReferencingKind::FlowRegistryClient => "REG",
            ReferencingKind::ParameterProvider => "PARM",
            ReferencingKind::Other(_) => "?",
        }
    };
    let state_cell = |kind: &ReferencingKind, state: &str| -> Cell<'static> {
        match kind {
            ReferencingKind::Processor => {
                let (glyph, style) = crate::widget::run_icon::processor_run_icon(state);
                Cell::from(format!("{glyph} {state}")).style(style)
            }
            ReferencingKind::ControllerService => {
                let style = match state {
                    "ENABLED" => theme::success(),
                    "DISABLED" => theme::disabled(),
                    _ => theme::warning(),
                };
                Cell::from(state.to_string()).style(style)
            }
            _ => Cell::from(state.to_string()).style(theme::muted()),
        }
    };

    let rows_data: Vec<Row> = d
        .referencing_components
        .iter()
        .map(|r| {
            Row::new(vec![
                state_cell(&r.kind, &r.state),
                Cell::from(kind_abbrev(&r.kind)),
                Cell::from(char_skip(&r.name, x_offset)),
                Cell::from(r.active_thread_count.to_string()),
            ])
        })
        .collect();

    let widths = [
        Constraint::Length(12),
        Constraint::Length(4),
        Constraint::Fill(1),
        Constraint::Length(4),
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

fn render_recent_bulletins_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    use crate::widget::severity::{format_severity_label, severity_style};
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::RecentBulletins)
        .unwrap_or(0);
    let is_focused = matches!(detail_focus, DetailFocus::Section { idx, .. } if *idx == my_idx);
    let x_offset = match detail_focus {
        DetailFocus::Section { x_offsets, .. } if is_focused => x_offsets[my_idx],
        _ => 0,
    };
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
            let sev = format_severity_label(&b.level);
            let sev_style = severity_style(&b.level);
            let msg = crate::view::bulletins::state::strip_component_prefix(&b.message).to_string();
            Row::new(vec![
                Cell::from(short_time(&b.timestamp_iso, &b.timestamp_human)),
                Cell::from(sev).style(sev_style),
                Cell::from(char_skip(&msg, x_offset)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(8),
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

/// Extract `HH:MM:SS` from an ISO-8601 timestamp, falling back to a
/// short slice of the human-readable form when the ISO field is empty.
fn short_time(iso: &str, human: &str) -> String {
    if iso.len() >= 19 {
        let t = &iso[11..19];
        if t.as_bytes().get(2) == Some(&b':') && t.as_bytes().get(5) == Some(&b':') {
            return t.to_string();
        }
    }
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
    use crate::view::browser::state::MAX_DETAIL_SECTIONS;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn seeded_cs_detail() -> (ControllerServiceDetail, BrowserState) {
        let d = ControllerServiceDetail {
            id: "cs1".into(),
            name: "http-pool".into(),
            type_name: "org.apache.nifi.ssl.StandardRestrictedSSLContextService".into(),
            bundle: "org.apache.nifi:nifi-ssl-context-service-nar:2.8.0".into(),
            state: "ENABLED".into(),
            parent_group_id: Some("ingest".into()),
            properties: vec![
                ("Keystore Filename".into(), "/opt/nifi/keystore.jks".into()),
                ("Keystore Type".into(), "JKS".into()),
                (
                    "Truststore Filename".into(),
                    "/opt/nifi/truststore.jks".into(),
                ),
                ("SSL Protocol".into(), "TLSv1.2".into()),
                ("Key Password".into(), "********".into()),
            ],
            validation_errors: vec!["Keystore password is required".into()],
            bulletin_level: "WARN".into(),
            comments: String::new(),
            restricted: false,
            deprecated: false,
            persists_state: false,
            referencing_components: Vec::new(),
        };
        let state = BrowserState::new();
        (d, state)
    }

    fn render_snapshot(detail_focus: &DetailFocus) -> String {
        let (d, state) = seeded_cs_detail();
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &d, &state, &bulletins, detail_focus);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn controller_service_detail_tree_focused() {
        let out = render_snapshot(&DetailFocus::Tree);
        assert_snapshot!("controller_service_detail_tree_focused", out);
    }

    #[test]
    fn controller_service_detail_properties_focused() {
        let focus = DetailFocus::Section {
            idx: 0,
            rows: [1, 0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let out = render_snapshot(&focus);
        assert_snapshot!("controller_service_detail_properties_focused", out);
    }

    #[test]
    fn controller_service_detail_with_referencing_components() {
        use crate::client::{ReferencingComponent, ReferencingKind};
        let (mut d, state) = seeded_cs_detail();
        // Clear validation errors so the section layout is
        // [Properties, ReferencingComponents, RecentBulletins].
        d.validation_errors.clear();
        d.referencing_components = vec![
            ReferencingComponent {
                id: "p1".into(),
                name: "InvokeHTTP".into(),
                kind: ReferencingKind::Processor,
                state: "RUNNING".into(),
                active_thread_count: 2,
                group_id: "ingest".into(),
            },
            ReferencingComponent {
                id: "cs2".into(),
                name: "dependent-ssl".into(),
                kind: ReferencingKind::ControllerService,
                state: "ENABLED".into(),
                active_thread_count: 0,
                group_id: "ingest".into(),
            },
        ];
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        // ReferencingComponents is at idx 1 when there are no validation errors.
        let focus = DetailFocus::Section {
            idx: 1,
            rows: [0, 0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let mut term = Terminal::new(TestBackend::new(120, 32)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus))
            .unwrap();
        assert_snapshot!(
            "controller_service_detail_with_referencing_components",
            format!("{}", term.backend())
        );
    }

    fn seeded_cs_with_uuid_property() -> (ControllerServiceDetail, BrowserState) {
        use crate::client::{NodeKind, NodeStatusSummary};
        use crate::view::browser::state::TreeNode;
        let dep_uuid = "11111111-2222-3333-4444-555555555555";
        let d = ControllerServiceDetail {
            id: "cs1".into(),
            name: "http-pool".into(),
            type_name: "org.apache.nifi.ssl.StandardRestrictedSSLContextService".into(),
            bundle: "org.apache.nifi:nifi-ssl-context-service-nar:2.8.0".into(),
            state: "ENABLED".into(),
            parent_group_id: Some("ingest".into()),
            properties: vec![
                ("Keystore Filename".into(), "/opt/nifi/keystore.jks".into()),
                ("Delegate Service".into(), dep_uuid.into()),
            ],
            validation_errors: vec![],
            bulletin_level: "WARN".into(),
            comments: String::new(),
            restricted: false,
            deprecated: false,
            persists_state: false,
            referencing_components: Vec::new(),
        };
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ControllerService,
            id: dep_uuid.into(),
            group_id: "ingest".into(),
            name: "dependent-ssl".into(),
            status_summary: NodeStatusSummary::ControllerService {
                state: "ENABLED".into(),
            },
        });
        (d, state)
    }

    #[test]
    fn controller_service_detail_properties_resolvable_uuid_shows_arrow() {
        let (d, state) = seeded_cs_with_uuid_property();
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus))
            .unwrap();
        assert_snapshot!(
            "controller_service_detail_properties_resolvable_uuid_shows_arrow",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn controller_service_detail_parent_group_resolved_to_name() {
        use crate::client::{NodeKind, NodeStatusSummary};
        use crate::view::browser::state::TreeNode;
        let pg_uuid = "deadbeef-cafe-f00d-face-b001b001b001";
        let (mut d, mut state) = seeded_cs_detail();
        d.validation_errors.clear();
        d.parent_group_id = Some(pg_uuid.into());
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ProcessGroup,
            id: pg_uuid.into(),
            group_id: String::new(),
            name: "ingest".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 1,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        });
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| render(f, f.area(), &d, &state, &bulletins, &DetailFocus::Tree))
            .unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains("ingest"),
            "expected parent PG name 'ingest' in output, got: {out}"
        );
        assert!(
            !out.contains(pg_uuid),
            "expected UUID {pg_uuid} to be resolved away; got: {out}"
        );
        assert_snapshot!(
            "controller_service_detail_parent_group_resolved_to_name",
            out
        );
    }
}
