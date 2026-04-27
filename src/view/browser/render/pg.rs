//! Process Group detail renderer.
//!
//! Task 6 rewrites PG detail into four nested sub-panels:
//! ` Identity ` (non-focusable Paragraph) and three focusable `Table`
//! widgets — ` Controller services `, ` Child groups `, ` Recent bulletins `.

use std::collections::VecDeque;

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table, TableState};

use crate::client::status::ControllerServiceState;
use crate::client::{BulletinSnapshot, ControllerServiceSummary, ProcessGroupDetail};
use crate::cluster::snapshot::ParameterContextRef;
use crate::theme;
use crate::view::browser::state::{
    BrowserState, ChildPgSummary, DetailFocus, DetailSection, DetailSections,
};
use crate::widget::panel::Panel;
use crate::widget::severity::{format_severity_label, severity_style};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    state: &BrowserState,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
    cluster: &crate::cluster::snapshot::ClusterSnapshot,
) {
    // Outer panel: " <name> · process group [STATE] "
    let outer = Panel::new(build_header_title(d, cluster)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Resolve the parameter-context binding for this PG so the identity
    // panel and the focusable ParameterContext section can render it.
    let pc_ref = state.parameter_context_ref_for(&d.id);

    // Section list drives both focus dispatch and the render ordering.
    let sections = DetailSections::for_pg_node(pc_ref.is_some());

    // Inner vertical layout.
    //   identity:  5 rows  (2 borders + 3 content lines)
    //   param ctx: 3 rows when bound (2 borders + 1 content line), else 0
    //   cs:        Fill(1) — split with child groups
    //   kids:      Fill(1)
    //   bulletins: 5 rows  (2 borders + 1 header + 2 data rows)
    let pc_height: u16 = if pc_ref.is_some() { 3 } else { 0 };
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(5),
            Constraint::Length(pc_height),
            Constraint::Fill(1),
            Constraint::Fill(1),
            Constraint::Length(5),
        ])
        .split(inner);

    render_identity_panel(frame, rows[0], d);
    if let Some(pc) = pc_ref {
        render_parameter_context_panel(frame, rows[1], d, pc, detail_focus, &sections);
    }
    render_controller_services_panel(frame, rows[2], d, state, detail_focus, &sections);
    render_child_groups_panel(frame, rows[3], d, state, detail_focus, &sections);
    render_recent_bulletins_panel(frame, rows[4], d, bulletins, detail_focus, &sections);
}

/// Build the outer panel title: ` <name> · process group [STATE] `.
/// Appends a `[STALE]` / `[MODIFIED]` / `[STALE+MOD]` / `[SYNC-ERR]`
/// chip when the PG is versioned and not `UP_TO_DATE`. Unversioned and
/// `UP_TO_DATE` PGs render the original two-segment title.
fn build_header_title<'a>(
    d: &'a ProcessGroupDetail,
    cluster: &'a crate::cluster::snapshot::ClusterSnapshot,
) -> Line<'a> {
    let mut spans = vec![
        Span::raw(" "),
        Span::styled(d.name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("process group", theme::muted()),
    ];
    if let Some(summary) = BrowserState::version_control_for(cluster, &d.id)
        && let Some((label, style)) = super::chip_for_state(summary.state)
    {
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("[{label}]"), style));
    }
    spans.push(Span::raw(" "));
    Line::from(spans)
}

fn render_identity_panel(frame: &mut Frame, area: Rect, d: &ProcessGroupDetail) {
    super::render_identity_panel(frame, area, |_inner| {
        vec![
            Line::from(vec![
                Span::styled("Processors ", theme::muted()),
                Span::raw(format!(
                    "{} running · {} stopped · {} invalid · {} disabled",
                    d.running, d.stopped, d.invalid, d.disabled
                )),
            ]),
            Line::from(vec![
                Span::styled("Threads    ", theme::muted()),
                Span::raw(format!("{} active", d.active_threads)),
            ]),
            Line::from(vec![
                Span::styled("Queued     ", theme::muted()),
                Span::raw(format!(
                    "{} ffiles · {}",
                    d.flow_files_queued, d.queued_display
                )),
            ]),
        ]
    });
}

/// Focusable single-row mini-section showing the bound parameter context
/// for this PG. Pressing Enter (Descend) dispatches `OpenParameterContextModal`.
fn render_parameter_context_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    pc: &ParameterContextRef,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    if area.height == 0 {
        return;
    }
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ParameterContext)
        .unwrap_or(0);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );
    let panel = Panel::new(" Parameter context ")
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let _ = d; // PG id not needed here; pc.name is the display value.
    let line = Line::from(vec![
        Span::raw(pc.name.clone()),
        Span::raw("  "),
        Span::styled("\u{2192}", theme::accent()),
    ]);
    frame.render_widget(Paragraph::new(line), inner);
}

fn render_controller_services_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ControllerServices)
        .unwrap_or(0);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );

    let x_offset = if is_focused {
        if let DetailFocus::Section { x_offsets, .. } = detail_focus {
            x_offsets[my_idx]
        } else {
            0
        }
    } else {
        0
    };

    let total = d.controller_services.len();
    let panel = Panel::new(" Controller services ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let header = Row::new(vec![
        Cell::from("STATE"),
        Cell::from("NAME"),
        Cell::from("TYPE"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let rows_data: Vec<Row> = d
        .controller_services
        .iter()
        .map(|cs: &ControllerServiceSummary| {
            let name = if state.resolve_id(&cs.id).is_some() {
                format!("{}  →", cs.name)
            } else {
                cs.name.clone()
            };
            Row::new(vec![
                Cell::from(cs.state.clone()).style(cs_state_style(&cs.state)),
                Cell::from(name),
                Cell::from(char_skip(&cs.type_short, x_offset)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(10),
        Constraint::Length(24),
        Constraint::Fill(1),
    ];
    let table = Table::new(rows_data, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut state = TableState::default();
    if let DetailFocus::Section { rows, .. } = detail_focus
        && is_focused
    {
        state.select(Some(rows[my_idx]));
    }
    frame.render_stateful_widget(table, inner, &mut state);
}

fn cs_state_style(state: &str) -> Style {
    ControllerServiceState::from_wire(state).referencing_style()
}

fn render_child_groups_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::ChildGroups)
        .unwrap_or(1);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );

    let x_offset = if is_focused {
        if let DetailFocus::Section { x_offsets, .. } = detail_focus {
            x_offsets[my_idx]
        } else {
            0
        }
    } else {
        0
    };

    let kids: Vec<ChildPgSummary> = state.child_process_groups(&d.id);
    let total = kids.len();

    let panel = Panel::new(" Child groups ")
        .right(Line::from(format!(" {total} ")))
        .focused(is_focused)
        .into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    let header = Row::new(vec![
        Cell::from("NAME"),
        Cell::from("RUN"),
        Cell::from("STOP"),
        Cell::from("INVALID"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let rows_data: Vec<Row> = kids
        .iter()
        .map(|k| {
            Row::new(vec![
                Cell::from(char_skip(&k.name, x_offset)),
                Cell::from(k.running.to_string()),
                Cell::from(k.stopped.to_string()),
                Cell::from(k.invalid.to_string()),
            ])
        })
        .collect();
    let widths = [
        Constraint::Fill(1),
        Constraint::Length(6),
        Constraint::Length(6),
        Constraint::Length(8),
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
    d: &ProcessGroupDetail,
    bulletins: &VecDeque<BulletinSnapshot>,
    detail_focus: &DetailFocus,
    sections: &DetailSections,
) {
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::RecentBulletins)
        .unwrap_or(2);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );
    let x_offset = if is_focused {
        if let DetailFocus::Section { x_offsets, .. } = detail_focus {
            x_offsets[my_idx]
        } else {
            0
        }
    } else {
        0
    };

    // Newest-first, no cap.
    let matching: Vec<&BulletinSnapshot> = bulletins
        .iter()
        .rev()
        .filter(|b| b.group_id == d.id)
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
        Cell::from("SOURCE"),
        Cell::from("MESSAGE"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let rows_data: Vec<Row> = matching
        .iter()
        .map(|b| {
            let sev_label = format_severity_label(&b.level);
            let sev_style = severity_style(&b.level);
            Row::new(vec![
                Cell::from(short_time(&b.timestamp_iso, &b.timestamp_human)),
                Cell::from(sev_label).style(sev_style),
                Cell::from(b.source_name.clone()),
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
        Constraint::Length(4),
        Constraint::Length(20),
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
    // Fallback: if the human string has `HH:MM:SS` somewhere, grab it.
    for i in 0..human.len().saturating_sub(7) {
        let slice = &human[i..i + 8];
        if slice.as_bytes()[2] == b':' && slice.as_bytes()[5] == b':' {
            return slice.to_string();
        }
    }
    "--:--:--".to_string()
}

/// Skip the first `n` Unicode scalar values from `s`, returning the remainder.
fn char_skip(s: &str, n: usize) -> String {
    s.chars().skip(n).collect()
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::client::NodeKind;
    use crate::cluster::snapshot::ClusterSnapshot;
    use crate::test_support::{TEST_BACKEND_MEDIUM, TEST_BACKEND_TALL, test_backend};
    use crate::view::browser::state::MAX_DETAIL_SECTIONS;
    use insta::assert_snapshot;
    use ratatui::Terminal;

    fn seeded_pg_detail() -> ProcessGroupDetail {
        ProcessGroupDetail {
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
                ControllerServiceSummary {
                    id: "cs1".into(),
                    name: "http-pool".into(),
                    type_short: "StandardRestrictedSSLContextService".into(),
                    state: "ENABLED".into(),
                },
                ControllerServiceSummary {
                    id: "cs2".into(),
                    name: "kafka-brokers".into(),
                    type_short: "Kafka3ConnectionService".into(),
                    state: "DISABLED".into(),
                },
            ],
        }
    }

    #[test]
    fn pg_detail_with_cs_list() {
        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_MEDIUM)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &bulletins,
                    &DetailFocus::Tree,
                    &snap,
                )
            })
            .unwrap();
        assert_snapshot!("pg_detail_with_cs_list", format!("{}", terminal.backend()));
    }

    #[test]
    fn pg_detail_controller_services_focused() {
        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let focus = DetailFocus::Section {
            idx: 0, // ControllerServices
            rows: [1, 0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_TALL)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus, &snap))
            .unwrap();
        assert_snapshot!(
            "pg_detail_controller_services_focused",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn pg_detail_child_groups_focused() {
        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let focus = DetailFocus::Section {
            idx: 1, // ChildGroups
            rows: [0, 0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_TALL)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus, &snap))
            .unwrap();
        assert_snapshot!(
            "pg_detail_child_groups_focused",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn pg_detail_controller_services_show_arrow_when_resolvable() {
        use crate::client::NodeStatusSummary;
        use crate::view::browser::state::TreeNode;

        // Seed a PG whose CSes carry UUID-shaped ids (real NiFi shape),
        // then seed arena nodes at those same ids so `resolve_id` succeeds.
        let cs1_uuid = "11111111-2222-3333-4444-555555555555";
        let cs2_uuid = "66666666-7777-8888-9999-aaaaaaaaaaaa";
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
                ControllerServiceSummary {
                    id: cs1_uuid.into(),
                    name: "http-pool".into(),
                    type_short: "StandardRestrictedSSLContextService".into(),
                    state: "ENABLED".into(),
                },
                ControllerServiceSummary {
                    id: cs2_uuid.into(),
                    name: "kafka-brokers".into(),
                    type_short: "Kafka3ConnectionService".into(),
                    state: "DISABLED".into(),
                },
            ],
        };
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ControllerService,
            id: cs1_uuid.into(),
            group_id: "ingest".into(),
            name: "http-pool".into(),
            status_summary: NodeStatusSummary::ControllerService {
                state: "ENABLED".into(),
            },
            parameter_context_ref: None,
        });
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ControllerService,
            id: cs2_uuid.into(),
            group_id: "ingest".into(),
            name: "kafka-brokers".into(),
            status_summary: NodeStatusSummary::ControllerService {
                state: "DISABLED".into(),
            },
            parameter_context_ref: None,
        });
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_MEDIUM)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &bulletins,
                    &DetailFocus::Tree,
                    &snap,
                )
            })
            .unwrap();
        let out = format!("{}", terminal.backend());
        assert!(
            out.contains("http-pool  →"),
            "expected arrow on resolvable CS row, got: {out}"
        );
        assert!(
            out.contains("kafka-brokers  →"),
            "expected arrow on second resolvable CS row, got: {out}"
        );
        assert_snapshot!(
            "pg_detail_controller_services_show_arrow_when_resolvable",
            out
        );
    }

    #[test]
    fn pg_detail_recent_bulletins_focused() {
        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let mut bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        bulletins.push_back(BulletinSnapshot {
            id: 1,
            level: "WARN".into(),
            message: "hi".into(),
            source_id: "p1".into(),
            source_name: "p1".into(),
            source_type: "PROCESSOR".into(),
            group_id: "ingest".into(),
            timestamp_iso: "2026-04-14T10:14:10.000Z".into(),
            timestamp_human: "04/14/2026 10:14:10 UTC".into(),
        });
        let focus = DetailFocus::Section {
            idx: 2, // RecentBulletins
            rows: [0, 0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_TALL)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus, &snap))
            .unwrap();
        assert_snapshot!(
            "pg_detail_recent_bulletins_focused",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn pg_detail_header_shows_modified_chip() {
        use crate::cluster::snapshot::{
            EndpointState, FetchMeta, VersionControlMap, VersionControlSummary,
        };
        use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let focus = DetailFocus::Tree;

        let mut map = VersionControlMap::default();
        map.by_pg_id.insert(
            d.id.clone(),
            VersionControlSummary {
                state: VersionControlInformationDtoState::LocallyModified,
                registry_name: Some("ops".into()),
                bucket_name: Some("flows".into()),
                branch: None,
                flow_id: None,
                flow_name: Some("ingest".into()),
                version: Some("3".into()),
                state_explanation: None,
            },
        );
        let snap = ClusterSnapshot {
            version_control: EndpointState::Ready {
                data: map,
                meta: FetchMeta {
                    fetched_at: std::time::Instant::now(),
                    fetch_duration: std::time::Duration::from_millis(0),
                    next_interval: std::time::Duration::from_secs(30),
                },
            },
            ..Default::default()
        };

        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_TALL)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus, &snap))
            .unwrap();
        insta::assert_snapshot!(
            "pg_detail_header_shows_modified_chip",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn pg_detail_renders_parameter_context_row_when_bound() {
        use crate::client::NodeStatusSummary;
        use crate::cluster::snapshot::ParameterContextRef;
        use crate::view::browser::state::TreeNode;

        let pg_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let d = ProcessGroupDetail {
            id: pg_uuid.into(),
            name: "ingest".into(),
            parent_group_id: Some("root".into()),
            running: 1,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 1,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0 / 0 B".into(),
            controller_services: vec![],
        };

        // Seed the arena with a PG node that carries a parameter_context_ref.
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ProcessGroup,
            id: pg_uuid.into(),
            group_id: "root".into(),
            name: "ingest".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 1,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
            parameter_context_ref: Some(ParameterContextRef {
                id: "ctx-id-001".into(),
                name: "ctx-prod".into(),
            }),
        });

        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_MEDIUM)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &bulletins,
                    &DetailFocus::Tree,
                    &snap,
                )
            })
            .unwrap();
        let out = format!("{}", terminal.backend());
        assert!(
            out.contains("Parameter context"),
            "expected 'Parameter context' section panel, got:\n{out}"
        );
        assert!(
            out.contains("ctx-prod"),
            "expected context name 'ctx-prod' in parameter context panel, got:\n{out}"
        );
        assert!(
            out.contains('\u{2192}'),
            "expected cross-link arrow '\u{2192}' in parameter context panel, got:\n{out}"
        );
        insta::assert_snapshot!("pg_detail_renders_parameter_context_row_when_bound", out);
    }

    #[test]
    fn pg_detail_omits_parameter_context_row_when_unbound() {
        // seeded_pg_detail() has id "ingest" and BrowserState::new() has no
        // arena nodes, so parameter_context_ref_for("ingest") returns None.
        let d = seeded_pg_detail();
        let state = BrowserState::new();
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_MEDIUM)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &bulletins,
                    &DetailFocus::Tree,
                    &snap,
                )
            })
            .unwrap();
        let out = format!("{}", terminal.backend());
        assert!(
            !out.contains("Parameter context"),
            "expected no 'Parameter context' section for unbound PG, got:\n{out}"
        );
    }

    #[test]
    fn pg_detail_parameter_context_panel_is_focused_when_section_selected() {
        use crate::client::NodeStatusSummary;
        use crate::cluster::snapshot::ParameterContextRef;
        use crate::view::browser::state::{DetailFocus, TreeNode};

        let pg_uuid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
        let d = ProcessGroupDetail {
            id: pg_uuid.into(),
            name: "ingest".into(),
            parent_group_id: Some("root".into()),
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0 / 0 B".into(),
            controller_services: vec![],
        };
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ProcessGroup,
            id: pg_uuid.into(),
            group_id: "root".into(),
            name: "ingest".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
            parameter_context_ref: Some(ParameterContextRef {
                id: "ctx-id-001".into(),
                name: "ctx-prod".into(),
            }),
        });

        // Section index 0 = ParameterContext (for_pg_node(true)).
        let focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let bulletins: VecDeque<BulletinSnapshot> = VecDeque::new();
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_MEDIUM)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins, &focus, &snap))
            .unwrap();
        let out = format!("{}", terminal.backend());
        // The param context panel should appear and contain the context name.
        assert!(
            out.contains("Parameter context"),
            "expected 'Parameter context' section; got:\n{out}"
        );
        assert!(
            out.contains("ctx-prod"),
            "expected context name; got:\n{out}"
        );
    }
}
