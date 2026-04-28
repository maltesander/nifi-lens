//! Browser tab renderer: two-pane tree + detail layout and modal overlays.
//!
//! Per-kind detail renderers live in sibling files (`pg.rs`,
//! `processor.rs`, `connection.rs`, `controller_service.rs`). This
//! module owns the outer layout, the tree pane, the dispatch to the
//! per-kind renderer, the loading / empty states, and the properties
//! modal overlay.

pub mod action_history_modal;
pub mod connection;
pub mod controller_service;
mod param_ref_scan;
pub mod parameter_context_modal;
pub mod pg;
pub mod port;
pub mod processor;
mod properties_modal;
pub mod queue_listing;
pub mod queue_listing_peek;
pub mod version_control_modal;
pub use param_ref_scan::{ParamRefScan, scan as scan_param_refs};
pub use properties_modal::render_properties_modal;

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::client::{FolderKind, NodeKind, NodeStatusSummary};
use crate::theme;
use crate::view::browser::state::{BrowserState, FlowIndex, NodeDetail, PgHealth};

/// Render an "Identity" Panel containing caller-built lines.
///
/// The helper handles the bordered Panel, computes the inner Rect,
/// and renders the lines as a Paragraph. The caller closure receives
/// the inner Rect (so it can size truncation against `inner.width`)
/// and returns the lines to render.
///
/// Used by Processor, ControllerService, ProcessGroup, and Port
/// detail panes; their per-pane content layouts differ so the helper
/// only owns the panel/Paragraph wrapper.
pub(super) fn render_identity_panel<F>(frame: &mut Frame, area: Rect, build_lines: F)
where
    F: FnOnce(Rect) -> Vec<Line<'static>>,
{
    let block = crate::widget::panel::Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let lines = build_lines(inner);
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Width below which the sparkline strip is suppressed.
pub(super) const SPARKLINE_MIN_RIGHT_HALF_WIDTH: u16 = 12;

/// Visible-column gap between the natural-width identity content on the
/// left and the sparkline strip on the right. Two cells reads clearly
/// while still letting the strip take meaningful space at narrow widths.
const SPARKLINE_GAP_COLS: u16 = 2;

/// Render an "Identity" Panel: caller-built lines render at their
/// **natural width** on the left; the sparkline strip is placed in
/// the remaining columns separated by `SPARKLINE_GAP_COLS`. The strip
/// is suppressed entirely when the remainder is narrower than
/// `SPARKLINE_MIN_RIGHT_HALF_WIDTH`, so left-half content never gets
/// truncated mid-word into the strip.
///
/// This used to do a fixed 50/50 horizontal split which produced visual
/// overlap (`5 iloading…`) whenever the identity text was wider than
/// half of the inner area. Sizing the left side to the actual content
/// width — not a percentage — is what makes the gap reliable at every
/// terminal size.
pub(super) fn render_identity_panel_with_sparkline<F>(
    frame: &mut Frame,
    area: Rect,
    sparkline: Option<&crate::view::browser::state::sparkline::SparklineState>,
    build_lines: F,
) where
    F: FnOnce(Rect) -> Vec<Line<'static>>,
{
    let block = crate::widget::panel::Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let lines = build_lines(inner);
    let max_left_width: u16 = lines
        .iter()
        .map(|l| u16::try_from(l.width()).unwrap_or(u16::MAX))
        .max()
        .unwrap_or(0);

    // Decide whether the strip fits next to the content. If the natural
    // identity width plus a 2-col gap plus the strip's minimum doesn't
    // fit in `inner`, render lines at full width and skip the strip.
    let needed = max_left_width
        .saturating_add(SPARKLINE_GAP_COLS)
        .saturating_add(SPARKLINE_MIN_RIGHT_HALF_WIDTH);
    if inner.width < needed {
        frame.render_widget(Paragraph::new(lines), inner);
        return;
    }

    let left_area = Rect {
        x: inner.x,
        y: inner.y,
        width: max_left_width,
        height: inner.height,
    };
    frame.render_widget(Paragraph::new(lines), left_area);

    let strip_area = Rect {
        x: inner.x + max_left_width + SPARKLINE_GAP_COLS,
        y: inner.y,
        width: inner.width - max_left_width - SPARKLINE_GAP_COLS,
        height: inner.height,
    };
    render_inline_sparkline_strip(frame, strip_area, sparkline);
}

/// Render the 3-line inline sparkline strip into `area`. When `area.width`
/// is below `SPARKLINE_MIN_RIGHT_HALF_WIDTH`, `sparkline` is `None`,
/// `endpoint_missing` is set, or the series is empty, renders a
/// single-line muted placeholder instead. Pure rendering — no allocation
/// beyond the returned `Line`s.
pub(super) fn render_inline_sparkline_strip(
    frame: &mut Frame,
    area: Rect,
    sparkline: Option<&crate::view::browser::state::sparkline::SparklineState>,
) {
    use crate::client::history::ComponentKind;
    use crate::widget::sparkline::{count_formatter, render_sparkline_row, task_time_formatter};

    if area.width < SPARKLINE_MIN_RIGHT_HALF_WIDTH {
        return;
    }

    let Some(sparkline) = sparkline else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("loading…", theme::muted()))),
            area,
        );
        return;
    };

    if sparkline.endpoint_missing {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no history yet", theme::muted()))),
            area,
        );
        return;
    }

    let Some(series) = sparkline.series.as_ref() else {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("loading…", theme::muted()))),
            area,
        );
        return;
    };

    // NiFi can return a successful status-history response with no
    // aggregate snapshots (idle / freshly-created components before its
    // compactor catches up). Falling through would render rows like
    // `in     peak 0` with no glyphs — a broken-looking strip. Surface
    // the same muted placeholder the 404 branch uses instead.
    if series.buckets.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled("no history yet", theme::muted()))),
            area,
        );
        return;
    }

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(area);

    let in_values: Vec<u64> = series.buckets.iter().map(|b| b.in_count).collect();
    let out_values: Vec<u64> = series.buckets.iter().map(|b| b.out_count).collect();

    // Use `map(unwrap_or(0))` rather than `filter_map`: when the metric
    // is missing for some buckets (or the key didn't match in the
    // reducer), we still want the row to render with the same length as
    // `in`/`out` — a row that vanishes whenever a metric is partially
    // absent is more confusing than zeros. `peak` reflects the actual
    // recorded values, so all-zeros still surface as `peak 0`.
    let (row3_label, row3_values, row3_formatter): (&str, Vec<u64>, fn(u64) -> String) =
        match sparkline.kind {
            ComponentKind::Processor => (
                "task",
                series
                    .buckets
                    .iter()
                    .map(|b| b.task_time_ns.unwrap_or(0))
                    .collect(),
                task_time_formatter as fn(u64) -> String,
            ),
            ComponentKind::ProcessGroup | ComponentKind::Connection => (
                "queue",
                series
                    .buckets
                    .iter()
                    .map(|b| b.queued_count.unwrap_or(0))
                    .collect(),
                count_formatter as fn(u64) -> String,
            ),
            ComponentKind::ControllerService | ComponentKind::Port => return,
        };

    frame.render_widget(
        Paragraph::new(render_sparkline_row(
            "in",
            5,
            &in_values,
            theme::spark_in(),
            count_formatter,
            area.width,
        )),
        rows[0],
    );
    frame.render_widget(
        Paragraph::new(render_sparkline_row(
            "out",
            5,
            &out_values,
            theme::spark_out(),
            count_formatter,
            area.width,
        )),
        rows[1],
    );
    frame.render_widget(
        Paragraph::new(render_sparkline_row(
            row3_label,
            5,
            &row3_values,
            theme::spark_queued(),
            row3_formatter,
            area.width,
        )),
        rows[2],
    );
}

/// Format a property value for Processor/CS detail tables. Returns
/// `Some(Line)` when the raw value is a UUID that resolves to a known
/// arena node (rendered as `<name> (short8…) →`), or when the raw
/// value contains a `#{name}` parameter reference and the owning PG
/// has a bound parameter context (rendered as `#{name} →` or
/// `#{…} →` for multiple refs). Returns `None` otherwise, so the
/// caller can fall back to raw-value rendering (with x-offset
/// scrolling).
///
/// UUID annotation takes precedence when both conditions apply.
pub(super) fn format_property_value(
    raw: &str,
    owning_pg_id: &str,
    state: &BrowserState,
) -> Option<Line<'static>> {
    // UUID cross-link takes priority.
    if let Some(r) = state.resolve_id(raw) {
        let short: String = raw.trim().chars().take(8).collect();
        return Some(Line::from(vec![
            Span::raw(r.name),
            Span::styled(format!(" ({short}…)"), theme::muted()),
            Span::styled(" →", theme::muted()),
        ]));
    }
    // Parameter reference annotation — only when the owning PG has a
    // bound context.
    if state.parameter_context_ref_for(owning_pg_id).is_some() {
        match scan_param_refs(raw) {
            ParamRefScan::None => {}
            ParamRefScan::Single { name } => {
                return Some(Line::from(vec![
                    Span::raw(format!("#{{{name}}}")),
                    Span::styled(" →", theme::muted()),
                ]));
            }
            ParamRefScan::Multiple => {
                return Some(Line::from(vec![
                    Span::raw("#{…}"),
                    Span::styled(" →", theme::muted()),
                ]));
            }
        }
    }
    None
}

/// Map a wire version-control state to the (label, style) shown as a
/// trailing chip on Browser tree rows. Returns `None` for `UpToDate` —
/// callers must skip rendering when this returns `None`.
pub(super) fn chip_for_state(
    state: nifi_rust_client::dynamic::types::VersionControlInformationDtoState,
) -> Option<(&'static str, ratatui::style::Style)> {
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;
    match state {
        S::UpToDate => None,
        S::Stale => Some(("STALE", crate::theme::warning())),
        S::LocallyModified => Some(("MODIFIED", crate::theme::warning())),
        S::LocallyModifiedAndStale => Some(("STALE+MOD", crate::theme::warning())),
        S::SyncFailure => Some(("SYNC-ERR", crate::theme::error())),
        _ => None,
    }
}

/// Entry point called from `app::ui`.
#[allow(clippy::too_many_arguments)]
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    _flow_index: &Option<FlowIndex>,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    cluster: &crate::cluster::snapshot::ClusterSnapshot,
    age_warning: std::time::Duration,
    show_node_column: bool,
) {
    let block = Block::default()
        .borders(Borders::ALL)
        .title(tab_title(state));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if state.nodes.is_empty() {
        let p = Paragraph::new("initial fetch…")
            .style(theme::muted())
            .alignment(Alignment::Center);
        let mid = Rect {
            x: inner.x,
            y: inner.y + inner.height / 2,
            width: inner.width,
            height: 1,
        };
        frame.render_widget(p, mid);
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(30),
            Constraint::Length(1),
            Constraint::Percentage(70),
        ])
        .split(inner);

    render_tree(frame, chunks[0], state, cluster);

    // Vertical separator between tree and detail panes.
    let sep = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme::muted());
    frame.render_widget(sep, chunks[1]);

    render_detail(
        frame,
        chunks[2],
        state,
        bulletins,
        cluster,
        age_warning,
        show_node_column,
    );
}

fn tab_title(state: &BrowserState) -> String {
    match state.last_tree_fetched_at {
        Some(t) => format!(" Browser — last {} ago ", fmt_ago(t)),
        None => " Browser ".into(),
    }
}

fn fmt_ago(when: SystemTime) -> String {
    match when.elapsed() {
        Ok(d) => {
            let secs = d.as_secs();
            if secs < 60 {
                format!("{secs}s")
            } else if secs < 3600 {
                format!("{}m", secs / 60)
            } else {
                format!("{}h", secs / 3600)
            }
        }
        Err(_) => "?".into(),
    }
}

fn render_tree(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    cluster: &crate::cluster::snapshot::ClusterSnapshot,
) {
    let mut lines: Vec<Line> = Vec::with_capacity(state.visible.len());
    let window_height = area.height as usize;
    let top = state
        .selected
        .saturating_sub(window_height.saturating_sub(1));
    for (row_idx, &arena_idx) in state
        .visible
        .iter()
        .enumerate()
        .skip(top)
        .take(window_height)
    {
        let node = &state.nodes[arena_idx];
        let depth = node_depth(state, arena_idx);
        let is_expanded_pg =
            matches!(node.kind, NodeKind::ProcessGroup) && state.expanded.contains(&arena_idx);
        let marker: &str = match (&node.kind, is_expanded_pg) {
            (NodeKind::ProcessGroup, true) => "▾ ",
            (NodeKind::ProcessGroup, false) => "▸ ",
            (NodeKind::Folder(_), _) => {
                if state.expanded.contains(&arena_idx) {
                    "▾ "
                } else {
                    "▸ "
                }
            }
            _ => "  ",
        };
        // Build the glyph span content as an owned String so Processor rows
        // can carry a per-status glyph char alongside a color style.
        let (glyph_owned, glyph_style): (String, Style) = match (&node.kind, &node.status_summary) {
            (NodeKind::Processor, NodeStatusSummary::Processor { run_status }) => {
                let (c, s) = crate::widget::run_icon::processor_run_icon(run_status);
                (c.to_string(), s)
            }
            _ => (kind_glyph(&node.kind).to_owned(), Style::default()),
        };
        let indent = "  ".repeat(depth);

        let row_style = if row_idx == state.selected {
            theme::cursor_row()
        } else {
            Style::default()
        };
        // Indent uses the neutral row style; marker uses the PG
        // rollup color (PG rows only) patched onto the row style.
        let marker_style = match node.kind {
            NodeKind::ProcessGroup => {
                let rollup_style = match state.pg_health_rollup(arena_idx) {
                    PgHealth::Green => theme::success(),
                    PgHealth::Yellow => theme::warning(),
                    PgHealth::Red => theme::error(),
                };
                rollup_style.patch(row_style)
            }
            _ => row_style,
        };
        let mut row_spans = vec![
            Span::styled(indent.clone(), row_style),
            Span::styled(marker.to_string(), marker_style),
            Span::styled(format!("{glyph_owned} "), glyph_style.patch(row_style)),
            Span::styled(node.name.clone(), row_style),
        ];
        if matches!(node.kind, crate::client::NodeKind::ProcessGroup)
            && let Some(summary) = BrowserState::version_control_for(cluster, &node.id)
            && let Some((label, style)) = chip_for_state(summary.state)
        {
            row_spans.push(Span::raw(" "));
            row_spans.push(Span::styled(format!("[{label}]"), style.patch(row_style)));
        }
        lines.push(Line::from(row_spans));
    }
    let p = Paragraph::new(lines);
    frame.render_widget(p, area);
}

fn node_depth(state: &BrowserState, idx: usize) -> usize {
    let mut depth = 0;
    let mut cursor = state.nodes[idx].parent;
    while let Some(p) = cursor {
        depth += 1;
        cursor = state.nodes[p].parent;
    }
    depth
}

fn kind_glyph(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::ProcessGroup => "",
        NodeKind::Processor => "●",
        NodeKind::Connection => "→",
        NodeKind::InputPort => "⇥",
        NodeKind::OutputPort => "⇤",
        NodeKind::ControllerService => "⚙",
        NodeKind::Folder(FolderKind::Queues) => "→",
        NodeKind::Folder(FolderKind::ControllerServices) => "⚙",
    }
}

/// Build the breadcrumb line for the current selection.
fn build_breadcrumb_line(state: &BrowserState) -> Line<'static> {
    let segments = state.breadcrumb_segments();
    if segments.is_empty() {
        return Line::from("");
    }

    let last = segments.len() - 1;
    let mut spans: Vec<Span<'static>> = Vec::new();

    for (i, seg) in segments.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(" > ", theme::muted()));
        }

        let style = if i == last {
            theme::bold() // current node
        } else {
            theme::muted() // ancestor
        };

        spans.push(Span::styled(seg.name.clone(), style));
    }

    Line::from(spans)
}

fn render_detail(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    cluster: &crate::cluster::snapshot::ClusterSnapshot,
    age_warning: std::time::Duration,
    show_node_column: bool,
) {
    let Some(&arena_idx) = state.visible.get(state.selected) else {
        return;
    };

    if matches!(state.nodes[arena_idx].kind, NodeKind::Folder(_)) {
        // Folder row: just render the breadcrumb (which skips folder
        // ancestors and in this case evaluates to ancestor-only).
        let crumb = build_breadcrumb_line(state);
        frame.render_widget(Paragraph::new(crumb), area);
        return;
    }

    // Split: 1 line for breadcrumb, rest for detail content.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // breadcrumb
            Constraint::Fill(1),   // detail content
        ])
        .split(area);

    // Render breadcrumb.
    let crumb_line = build_breadcrumb_line(state);
    frame.render_widget(Paragraph::new(crumb_line), chunks[0]);

    let detail_area = chunks[1];

    let node = &state.nodes[arena_idx];
    let header = format!(
        "{kind} — {name}",
        kind = kind_label(&node.kind),
        name = node.name
    );
    let header_line = Line::from(Span::styled(header, theme::accent()));

    match state.details.get(&arena_idx) {
        Some(NodeDetail::ProcessGroup(d)) => {
            pg::render(
                frame,
                detail_area,
                d,
                state,
                bulletins,
                &state.detail_focus,
                cluster,
            );
        }
        Some(NodeDetail::Processor(d)) => {
            processor::render(frame, detail_area, d, state, bulletins, &state.detail_focus);
        }
        Some(NodeDetail::Connection(d)) => {
            connection::render(
                frame,
                detail_area,
                d,
                state,
                &state.detail_focus,
                age_warning,
                show_node_column,
            );
        }
        Some(NodeDetail::ControllerService(d)) => {
            controller_service::render(
                frame,
                detail_area,
                d,
                state,
                bulletins,
                &state.detail_focus,
            );
        }
        Some(NodeDetail::Port(d)) => {
            port::render(frame, detail_area, d, state, bulletins, &state.detail_focus);
        }
        None => {
            let lines = vec![
                header_line,
                Line::from(""),
                Line::from(Span::styled("loading…", theme::muted())),
            ];
            frame.render_widget(Paragraph::new(lines), detail_area);
        }
    }
}

fn kind_label(kind: &NodeKind) -> &'static str {
    match kind {
        NodeKind::ProcessGroup => "Process Group",
        NodeKind::Processor => "Processor",
        NodeKind::Connection => "Connection",
        NodeKind::InputPort => "Input Port",
        NodeKind::OutputPort => "Output Port",
        NodeKind::ControllerService => "Controller Service",
        NodeKind::Folder(FolderKind::Queues) => "Queues",
        NodeKind::Folder(FolderKind::ControllerServices) => "Controller services",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::view::browser::state::{BrowserState, TreeNode, rebuild_visible};

    #[test]
    fn breadcrumb_line_shows_path_segments() {
        let mut state = BrowserState::new();
        // Build Root > Generate (2 nodes).
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![1],
            kind: NodeKind::ProcessGroup,
            id: "root-id".into(),
            group_id: String::new(),
            name: "Root".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 1,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
            parameter_context_ref: None,
        });
        state.nodes.push(TreeNode {
            parent: Some(0),
            children: vec![],
            kind: NodeKind::Processor,
            id: "proc-1".into(),
            group_id: "root-id".into(),
            name: "Generate".into(),
            status_summary: NodeStatusSummary::Processor {
                run_status: "Running".into(),
            },
            parameter_context_ref: None,
        });
        state.expanded.insert(0);
        rebuild_visible(&mut state);
        state.selected = 1;

        let line = build_breadcrumb_line(&state);
        let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        assert!(text.contains("Root"));
        assert!(text.contains("Generate"));
        assert!(text.contains(" > "));
    }
}

#[cfg(test)]
mod chip_tests {
    use super::chip_for_state;
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;

    #[test]
    fn up_to_date_renders_no_chip() {
        assert!(chip_for_state(S::UpToDate).is_none());
    }

    #[test]
    fn each_drift_state_has_a_chip() {
        assert_eq!(chip_for_state(S::Stale).unwrap().0, "STALE");
        assert_eq!(chip_for_state(S::LocallyModified).unwrap().0, "MODIFIED");
        assert_eq!(
            chip_for_state(S::LocallyModifiedAndStale).unwrap().0,
            "STALE+MOD"
        );
        assert_eq!(chip_for_state(S::SyncFailure).unwrap().0, "SYNC-ERR");
    }
}

#[cfg(test)]
mod identity_panel_tests {
    use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
    use ratatui::Terminal;
    use ratatui::layout::Rect;
    use ratatui::text::{Line, Span};

    #[test]
    fn render_identity_panel_invokes_closure_once_and_renders_panel_border() {
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        let mut closure_calls = 0u32;
        let mut observed_inner_width = 0u16;
        let mut observed_inner_height = 0u16;
        terminal
            .draw(|frame| {
                let area = Rect::new(0, 0, 40, 6);
                super::render_identity_panel(frame, area, |inner| {
                    closure_calls += 1;
                    observed_inner_width = inner.width;
                    observed_inner_height = inner.height;
                    vec![Line::from(Span::raw("hello"))]
                });
            })
            .unwrap();
        assert_eq!(closure_calls, 1, "closure must be invoked exactly once");
        // Panel borders consume 1 cell on each side, so inner is 40-2 by 6-2.
        assert_eq!(observed_inner_width, 38);
        assert_eq!(observed_inner_height, 4);
        // Verify the " Identity " title made it into the rendered buffer.
        let dump = format!("{}", terminal.backend());
        assert!(
            dump.contains("Identity"),
            "expected rendered panel to contain 'Identity' title, got:\n{dump}"
        );
        // And the closure-supplied body line should be present too.
        assert!(
            dump.contains("hello"),
            "expected rendered panel to contain closure-supplied body, got:\n{dump}"
        );
    }
}

#[cfg(test)]
mod tree_render_tests {
    use super::*;
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::cluster::snapshot::{
        ClusterSnapshot, EndpointState, FetchMeta, VersionControlMap, VersionControlSummary,
    };
    use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
    use crate::view::browser::state::TreeNode;
    use insta::assert_snapshot;
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
    use ratatui::Terminal;

    fn seeded_state_with_one_pg(pg_id: &str, pg_name: &str) -> BrowserState {
        let mut state = BrowserState::new();
        state.nodes.push(TreeNode {
            parent: None,
            children: vec![],
            kind: NodeKind::ProcessGroup,
            id: pg_id.into(),
            group_id: String::new(),
            name: pg_name.into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
            parameter_context_ref: None,
        });
        // Visible row pointing at the PG.
        state.visible.push(0);
        state
    }

    fn snapshot_with(pg_id: &str, st: VersionControlInformationDtoState) -> ClusterSnapshot {
        let mut map = VersionControlMap::default();
        map.by_pg_id.insert(
            pg_id.into(),
            VersionControlSummary {
                state: st,
                registry_name: Some("ops".into()),
                bucket_name: Some("flows".into()),
                branch: None,
                flow_id: Some("f-1".into()),
                flow_name: Some("ingest".into()),
                version: Some("3".into()),
                state_explanation: None,
            },
        );
        ClusterSnapshot {
            version_control: EndpointState::Ready {
                data: map,
                meta: FetchMeta {
                    fetched_at: std::time::Instant::now(),
                    fetch_duration: std::time::Duration::from_millis(10),
                    next_interval: std::time::Duration::from_secs(30),
                },
            },
            ..ClusterSnapshot::default()
        }
    }

    #[test]
    fn tree_row_renders_stale_chip() {
        let state = seeded_state_with_one_pg("pg-1", "ingest");
        let snap = snapshot_with("pg-1", VersionControlInformationDtoState::Stale);
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| render_tree(f, f.area(), &state, &snap))
            .unwrap();
        assert_snapshot!(
            "tree_row_renders_stale_chip",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn tree_row_renders_no_chip_when_unversioned() {
        let state = seeded_state_with_one_pg("pg-1", "ingest");
        let snap = ClusterSnapshot::default();
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| render_tree(f, f.area(), &state, &snap))
            .unwrap();
        assert_snapshot!(
            "tree_row_renders_no_chip_when_unversioned",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn tree_row_renders_stale_mod_chip_for_combined_state() {
        let state = seeded_state_with_one_pg("pg-1", "ingest");
        let snap = snapshot_with(
            "pg-1",
            VersionControlInformationDtoState::LocallyModifiedAndStale,
        );
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| render_tree(f, f.area(), &state, &snap))
            .unwrap();
        assert_snapshot!(
            "tree_row_renders_stale_mod_chip_for_combined_state",
            format!("{}", terminal.backend())
        );
    }

    #[test]
    fn tree_row_renders_sync_err_chip() {
        let state = seeded_state_with_one_pg("pg-1", "ingest");
        let snap = snapshot_with("pg-1", VersionControlInformationDtoState::SyncFailure);
        let mut terminal = Terminal::new(test_backend(TEST_BACKEND_SHORT)).unwrap();
        terminal
            .draw(|f| render_tree(f, f.area(), &state, &snap))
            .unwrap();
        assert_snapshot!(
            "tree_row_renders_sync_err_chip",
            format!("{}", terminal.backend())
        );
    }
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use crate::view::browser::state::apply_tree_snapshot;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::SystemTime;

    fn render_to_string(state: &BrowserState) -> String {
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let cluster = crate::cluster::snapshot::ClusterSnapshot::default();
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|f| {
                super::render(
                    f,
                    f.area(),
                    state,
                    &None,
                    &bulletins,
                    &cluster,
                    std::time::Duration::from_secs(5 * 60),
                    false,
                );
            })
            .unwrap();
        format!("{}", terminal.backend())
    }

    fn demo() -> BrowserState {
        let mut s = BrowserState::new();
        // Use a fixed fetched_at in the past so the "last Ns ago" text is
        // stable across test runs. Pin to now() - 3s (matching the Bulletins
        // fix) so the snapshot shows "last 3s ago".
        let fetched_at = SystemTime::now() - std::time::Duration::from_secs(3);
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 5,
                        stopped: 1,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "root".into(),
                    name: "GenerateFlowFile".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Connection,
                    id: "c1".into(),
                    group_id: "root".into(),
                    name: "gen→upd".into(),
                    status_summary: NodeStatusSummary::Connection {
                        fill_percent: 34,
                        flow_files_queued: 12,
                        queued_display: "12 / 1KB".into(),
                        source_id: String::new(),
                        source_name: String::new(),
                        destination_id: String::new(),
                        destination_name: String::new(),
                    },
                },
            ],
            fetched_at,
        };
        apply_tree_snapshot(&mut s, snap);
        // Re-pin last_fetched_at to stabilize the snapshot regardless of
        // the wall-clock delta between snap.fetched_at and now.
        s.last_tree_fetched_at = Some(SystemTime::now() - std::time::Duration::from_secs(3));
        s
    }

    #[test]
    fn browser_initial_fetch_empty() {
        let s = BrowserState::new();
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { assert_snapshot!("browser_initial_fetch_empty", render_to_string(&s)); }
        );
    }

    #[test]
    fn browser_tree_seeded_root_expanded() {
        let s = demo();
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { assert_snapshot!("browser_tree_seeded_root_expanded", render_to_string(&s)); }
        );
    }

    #[test]
    fn browser_tree_renders_queues_and_cs_folders_collapsed() {
        let mut s = BrowserState::new();
        let fetched_at = SystemTime::now() - std::time::Duration::from_secs(3);
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 1,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "p".into(),
                    group_id: "root".into(),
                    name: "Generate".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Connection,
                    id: "c".into(),
                    group_id: "root".into(),
                    name: "q1".into(),
                    status_summary: NodeStatusSummary::Connection {
                        fill_percent: 0,
                        flow_files_queued: 0,
                        queued_display: "0".into(),
                        source_id: String::new(),
                        source_name: String::new(),
                        destination_id: String::new(),
                        destination_name: String::new(),
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ControllerService,
                    id: "cs".into(),
                    group_id: "root".into(),
                    name: "pool".into(),
                    status_summary: NodeStatusSummary::ControllerService {
                        state: "ENABLED".into(),
                    },
                },
            ],
            fetched_at,
        };
        apply_tree_snapshot(&mut s, snap);
        s.last_tree_fetched_at = Some(SystemTime::now() - std::time::Duration::from_secs(3));
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { assert_snapshot!("browser_tree_queues_and_cs_folders_collapsed", render_to_string(&s)); }
        );
    }
}
