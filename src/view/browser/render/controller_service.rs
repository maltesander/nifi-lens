//! Controller Service detail renderer.
//!
//! Phase 7 layout:
//!
//! ```text
//! ┌ <name> · controller service · <state> ─┐
//! │┌ Identity ─────────────────────┐       │
//! ││ type / bundle / parent        │       │
//! │└───────────────────────────────┘       │
//! │┌ Properties  N ────────────────┐       │  ← focusable
//! ││ KEY              VALUE        │       │
//! ││ ...scrollable Table...        │       │
//! │└───────────────────────────────┘       │
//! │┌ Validation errors  N ─────────┐       │  ← optional, shown when errors present
//! ││ error message 1               │       │
//! ││ ...                           │       │
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
    _state: &BrowserState,
    detail_focus: &DetailFocus,
) {
    // 1. Outer panel: " <name> · controller service · <state> "
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // 2. Inner vertical layout.
    //    identity: 5 rows (2 borders + 3 content lines)
    //    properties (and optional validation line): fill
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(5), Constraint::Fill(1)])
        .split(inner);

    render_identity_panel(frame, rows[0], d);
    render_properties_and_validation(frame, rows[1], d, detail_focus);
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

fn render_identity_panel(frame: &mut Frame, area: Rect, d: &ControllerServiceDetail) {
    let block = Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let parent = d.parent_group_id.as_deref().unwrap_or("(controller)");
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
        Line::from(vec![
            Span::styled("parent ", theme::muted()),
            Span::raw(truncate(parent, inner.width.saturating_sub(7) as usize)),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Renders the Properties sub-panel and, when the CS has validation
/// errors, a bordered panel listing them below Properties.
fn render_properties_and_validation(
    frame: &mut Frame,
    area: Rect,
    d: &ControllerServiceDetail,
    detail_focus: &DetailFocus,
) {
    let has_validation = !d.validation_errors.is_empty();
    let sections = DetailSections::for_node_detail(NodeKind::ControllerService, has_validation);
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
    d: &ControllerServiceDetail,
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
    d: &ControllerServiceDetail,
    detail_focus: &DetailFocus,
) {
    let sections = DetailSections::for_node_detail(
        NodeKind::ControllerService,
        !d.validation_errors.is_empty(),
    );
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
        let mut term = Terminal::new(TestBackend::new(120, 30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &d, &state, detail_focus);
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
}
