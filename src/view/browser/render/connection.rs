//! Connection detail renderer.
//!
//! The Endpoints panel carries a focusable 2-row FROM/TO mini-table.
//! Each row appends a `  →` marker when the opposite component resolves
//! to a known arena node via `BrowserState::resolve_id`.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ConnectionDetail;
use crate::theme;
use crate::view::browser::state::{BrowserState, DetailFocus};
use crate::widget::panel::Panel;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ConnectionDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
    age_warning: std::time::Duration,
    show_node_column: bool,
) {
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    use ratatui::layout::{Constraint, Direction, Layout};
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(8), // endpoints
            Constraint::Length(7), // back-pressure (4 content lines + panel borders)
            Constraint::Min(8),    // queue listing
        ])
        .split(inner);

    render_endpoints_panel(frame, rows[0], d, state, detail_focus);
    render_back_pressure_panel(frame, rows[1], d);

    if let Some(listing) = state.queue_listing.as_ref() {
        let is_empty_queue = d.flow_files_queued == 0;
        let focused = state.listing_focused;
        crate::view::browser::render::queue_listing::render_queue_listing(
            frame,
            rows[2],
            listing,
            age_warning,
            is_empty_queue,
            show_node_column,
            focused,
        );
    }
}

/// Build the outer panel title: ` <name> · connection `.
fn build_header_title(d: &ConnectionDetail) -> Line<'_> {
    Line::from(vec![
        Span::raw(" "),
        Span::styled(
            d.name.as_str(),
            theme::accent().add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("connection", theme::muted()),
        Span::raw(" "),
    ])
}

fn render_endpoints_panel(
    frame: &mut Frame,
    area: Rect,
    d: &ConnectionDetail,
    state: &BrowserState,
    detail_focus: &DetailFocus,
) {
    use crate::view::browser::state::{DetailSection, DetailSections};
    use crate::widget::gauge::fill_bar;
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Cell, Row, Table, TableState};

    let sections = DetailSections::for_node(crate::client::NodeKind::Connection);
    let my_idx = sections
        .0
        .iter()
        .position(|s| *s == DetailSection::Endpoints)
        .unwrap_or(0);
    let is_focused = matches!(
        detail_focus,
        DetailFocus::Section { idx, .. } if *idx == my_idx
    );

    let panel = Panel::new(" Endpoints ").focused(is_focused).into_block();
    let inner = panel.inner(area);
    frame.render_widget(panel, area);

    // Reserve the rightmost columns for the 3-line sparkline strip
    // and give endpoints content (fill gauge + table + relations)
    // everything else. Mirrors the natural-width approach
    // `render_identity_panel_with_sparkline` uses for processor / PG
    // identity panels — the previous 50/50 split crushed the NAME
    // column on narrow terminals (visible as `GenerateFlowF` truncating
    // into the sparkline).
    //
    // ENDPOINTS_MIN_WIDTH = 5 (DIR) + 1 + 12 (KIND) + 1 + ~25 (typical
    // NAME + cross-link arrow) = ~44; round up to 48 for the fill
    // gauge to stay legible.
    const ENDPOINTS_MIN_WIDTH: u16 = 48;
    const SPARKLINE_PREFERRED_WIDTH: u16 = 28;
    let (endpoints_area, sparkline_area) = if inner.width
        >= ENDPOINTS_MIN_WIDTH + super::SPARKLINE_GAP_COLS + super::SPARKLINE_MIN_RIGHT_HALF_WIDTH
    {
        let sparkline_width = SPARKLINE_PREFERRED_WIDTH
            .min(
                inner
                    .width
                    .saturating_sub(ENDPOINTS_MIN_WIDTH + super::SPARKLINE_GAP_COLS),
            )
            .max(super::SPARKLINE_MIN_RIGHT_HALF_WIDTH);
        let chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Min(ENDPOINTS_MIN_WIDTH),
                Constraint::Length(super::SPARKLINE_GAP_COLS),
                Constraint::Length(sparkline_width),
            ])
            .split(inner);
        (chunks[0], Some(chunks[2]))
    } else {
        (inner, None)
    };
    if let Some(spark_area) = sparkline_area {
        super::render_inline_sparkline_strip(frame, spark_area, state.sparkline.as_ref());
    }

    // Fill header (line 0), mini-table (lines 1..=3), relations (line 4).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // fill
            Constraint::Length(3), // header + 2 rows
            Constraint::Length(1), // relations
        ])
        .split(endpoints_area);

    // Fill gauge.
    let gauge_width: u16 = rows[0].width.saturating_sub(12).clamp(8, 40);
    let bar = fill_bar(gauge_width, d.fill_percent);
    let gauge_style = fill_style(d.fill_percent);
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Fill ", theme::muted()),
            Span::styled(bar, gauge_style),
            Span::raw(format!(
                "  {}% ({} ff / {})",
                d.fill_percent, d.flow_files_queued, d.queued_display
            )),
        ])),
        rows[0],
    );

    // Endpoints mini-table.
    let header = Row::new(vec![
        Cell::from("DIR"),
        Cell::from("KIND"),
        Cell::from("NAME"),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let from_row = endpoint_row("FROM", &d.source_type, &d.source_name, &d.source_id, state);
    let to_row = endpoint_row(
        "TO",
        &d.destination_type,
        &d.destination_name,
        &d.destination_id,
        state,
    );

    let widths = [
        Constraint::Length(5),
        Constraint::Length(12),
        Constraint::Fill(1),
    ];
    let table = Table::new(vec![from_row, to_row], widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut ts = TableState::default();
    if let DetailFocus::Section {
        rows: focus_rows, ..
    } = detail_focus
        && is_focused
    {
        ts.select(Some(focus_rows[my_idx]));
    }
    frame.render_stateful_widget(table, rows[1], &mut ts);

    // Relations.
    let relations = if d.selected_relationships.is_empty() {
        "(none)".to_string()
    } else {
        d.selected_relationships.join(", ")
    };
    frame.render_widget(
        Paragraph::new(Line::from(vec![
            Span::styled("Relations: ", theme::muted()),
            Span::raw(relations),
        ])),
        rows[2],
    );
}

fn endpoint_row(
    dir: &str,
    kind: &str,
    name: &str,
    id: &str,
    state: &BrowserState,
) -> ratatui::widgets::Row<'static> {
    use ratatui::widgets::{Cell, Row};
    let mut name_cell = name.to_string();
    if state.resolve_id(id).is_some() {
        name_cell.push_str("  \u{2192}");
    }
    Row::new(vec![
        Cell::from(dir.to_string()),
        Cell::from(kind.to_string()),
        Cell::from(name_cell),
    ])
}

fn render_back_pressure_panel(frame: &mut Frame, area: Rect, d: &ConnectionDetail) {
    let block = Panel::new(" Back-pressure ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = vec![
        Line::from(vec![
            Span::styled("count    ", theme::muted()),
            Span::raw(d.back_pressure_object_threshold.to_string()),
        ]),
        Line::from(vec![
            Span::styled("size     ", theme::muted()),
            Span::raw(d.back_pressure_data_size_threshold.clone()),
        ]),
        Line::from(vec![
            Span::styled("expire   ", theme::muted()),
            Span::raw(if d.flow_file_expiration.is_empty() {
                "none".to_string()
            } else {
                d.flow_file_expiration.clone()
            }),
        ]),
        Line::from(vec![
            Span::styled("load-bal ", theme::muted()),
            Span::raw(d.load_balance_strategy.clone()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Fill-percent → gauge color. Mirrors the Overview repositories
/// severity mapping.
fn fill_style(percent: u32) -> ratatui::style::Style {
    if percent >= 80 {
        theme::error()
    } else if percent >= 50 {
        theme::warning()
    } else {
        theme::success()
    }
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
    use crate::view::browser::state::{MAX_DETAIL_SECTIONS, TreeNode};
    use insta::assert_snapshot;
    use ratatui::Terminal;

    // UUID-shaped IDs so resolve_id passes the is_uuid_shape pre-filter.
    const SRC_UUID: &str = "aaaaaaaa-0000-0000-0000-000000000001";
    const DST_UUID: &str = "aaaaaaaa-0000-0000-0000-000000000002";

    fn seeded() -> (ConnectionDetail, BrowserState) {
        let d = ConnectionDetail {
            id: "c1c1c1c1-0000-0000-0000-000000000001".into(),
            name: "enrich → publish".into(),
            source_id: SRC_UUID.into(),
            source_name: "EnrichAttribute".into(),
            source_type: "PROCESSOR".into(),
            source_group_id: "ingest".into(),
            destination_id: DST_UUID.into(),
            destination_name: "PublishKafka".into(),
            destination_type: "PROCESSOR".into(),
            destination_group_id: "publish".into(),
            selected_relationships: vec!["success".into()],
            available_relationships: vec!["success".into(), "failure".into()],
            back_pressure_object_threshold: 10_000,
            back_pressure_data_size_threshold: "1 GB".into(),
            flow_file_expiration: "0 sec".into(),
            load_balance_strategy: "DO_NOT_LOAD_BALANCE".into(),
            fill_percent: 55,
            flow_files_queued: 5_500,
            bytes_queued: 50 * crate::bytes::MIB,
            queued_display: "5,500 / 50 MB".into(),
        };
        // Arena must contain the source and destination so resolve_id
        // returns Some and the → markers render.
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::Processor,
            id: SRC_UUID.into(),
            group_id: "ingest".into(),
            name: "EnrichAttribute".into(),
            status_summary: NodeStatusSummary::Processor {
                run_status: "Running".into(),
            },
            parameter_context_ref: None,
        });
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::Processor,
            id: DST_UUID.into(),
            group_id: "publish".into(),
            name: "PublishKafka".into(),
            status_summary: NodeStatusSummary::Processor {
                run_status: "Running".into(),
            },
            parameter_context_ref: None,
        });
        (d, state)
    }

    #[test]
    fn connection_detail_renders() {
        let (d, state) = seeded();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &DetailFocus::Tree,
                    std::time::Duration::from_secs(5 * 60),
                    false,
                )
            })
            .unwrap();
        assert_snapshot!(
            "connection_detail_renders",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn connection_detail_endpoints_focused() {
        // `TestBackend` does not capture ANSI style codes, so the row-
        // highlight cursor is not visible in the snapshot. This test
        // verifies the panel border flips to the focused style and the
        // arrow markers render correctly; the row-to-intent mapping is
        // covered by the reducer tests in `app::state::browser`.
        let (d, state) = seeded();
        let focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &focus,
                    std::time::Duration::from_secs(5 * 60),
                    false,
                )
            })
            .unwrap();
        assert_snapshot!(
            "connection_detail_endpoints_focused",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn connection_detail_endpoints_hides_arrow_when_opposite_not_in_arena() {
        let (d, mut state) = seeded();
        state.nodes.clear(); // nothing resolves → no markers
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &DetailFocus::Tree,
                    std::time::Duration::from_secs(5 * 60),
                    false,
                )
            })
            .unwrap();
        assert_snapshot!(
            "connection_detail_endpoints_no_arrows",
            format!("{}", terminal.backend())
        );
    }

    fn sample_conn_series(buckets: usize) -> crate::client::history::StatusHistorySeries {
        use crate::client::history::{Bucket, StatusHistorySeries};
        StatusHistorySeries {
            buckets: (0..buckets)
                .map(|i| Bucket {
                    timestamp: std::time::SystemTime::now(),
                    in_count: ((i * 4) % 80) as u64,
                    out_count: ((i * 3) % 70) as u64,
                    queued_count: Some(((i * 6) % 40) as u64),
                    task_time_ns: None,
                })
                .collect(),
            generated_at: std::time::SystemTime::now(),
        }
    }

    fn render_conn_with_sparkline(
        width: u16,
        sparkline: Option<crate::view::browser::state::sparkline::SparklineState>,
    ) -> String {
        let (d, mut state) = seeded();
        state.sparkline = sparkline;
        let mut terminal = Terminal::new(ratatui::backend::TestBackend::new(width, 30)).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &d,
                    &state,
                    &DetailFocus::Tree,
                    std::time::Duration::from_secs(5 * 60),
                    false,
                )
            })
            .unwrap();
        format!("{}", terminal.backend())
    }

    const CONN_ID: &str = "c1c1c1c1-0000-0000-0000-000000000001";

    #[test]
    fn connection_detail_sparkline_wide() {
        use crate::client::history::ComponentKind;
        use crate::view::browser::state::sparkline::SparklineState;
        let mut s = SparklineState::pending(ComponentKind::Connection, CONN_ID.into());
        s.series = Some(sample_conn_series(40));
        let out = render_conn_with_sparkline(120, Some(s));
        assert_snapshot!("connection_detail_sparkline_wide", out);
    }

    #[test]
    fn connection_detail_sparkline_narrow_suppressed() {
        use crate::client::history::ComponentKind;
        use crate::view::browser::state::sparkline::SparklineState;
        let mut s = SparklineState::pending(ComponentKind::Connection, CONN_ID.into());
        s.series = Some(sample_conn_series(40));
        let out = render_conn_with_sparkline(26, Some(s));
        assert_snapshot!("connection_detail_sparkline_narrow_suppressed", out);
    }

    #[test]
    fn connection_detail_sparkline_endpoint_missing() {
        use crate::client::history::ComponentKind;
        use crate::view::browser::state::sparkline::SparklineState;
        let mut s = SparklineState::pending(ComponentKind::Connection, CONN_ID.into());
        s.endpoint_missing = true;
        let out = render_conn_with_sparkline(120, Some(s));
        assert_snapshot!("connection_detail_sparkline_endpoint_missing", out);
    }

    #[test]
    fn connection_detail_sparkline_empty_buckets_renders_placeholder() {
        use crate::client::history::ComponentKind;
        use crate::view::browser::state::sparkline::SparklineState;
        let mut s = SparklineState::pending(ComponentKind::Connection, CONN_ID.into());
        s.series = Some(sample_conn_series(0));
        let out = render_conn_with_sparkline(120, Some(s));
        assert_snapshot!("connection_detail_sparkline_empty_buckets", out);
    }
}
