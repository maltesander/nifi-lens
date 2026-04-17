//! Browser tab renderer: two-pane tree + detail layout and modal overlays.
//!
//! Per-kind detail renderers live in sibling files (`pg.rs`,
//! `processor.rs`, `connection.rs`, `controller_service.rs`). This
//! module owns the outer layout, the tree pane, the dispatch to the
//! per-kind renderer, the loading / empty states, and the properties
//! modal overlay.

pub mod connection;
pub mod controller_service;
pub mod pg;
pub mod port;
pub mod processor;

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};

use crate::client::{FolderKind, NodeKind, NodeStatusSummary};
use crate::layout;
use crate::theme;
use crate::view::browser::state::{BrowserState, FlowIndex, NodeDetail, PgHealth};

/// Entry point called from `app::ui`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    _flow_index: &Option<FlowIndex>,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
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

    render_tree(frame, chunks[0], state);

    // Vertical separator between tree and detail panes.
    let sep = Block::default()
        .borders(Borders::LEFT)
        .border_style(theme::muted());
    frame.render_widget(sep, chunks[1]);

    render_detail(frame, chunks[2], state, bulletins);
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

fn render_tree(frame: &mut Frame, area: Rect, state: &BrowserState) {
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
        lines.push(Line::from(vec![
            Span::styled(indent.clone(), row_style),
            Span::styled(marker.to_string(), marker_style),
            Span::styled(format!("{glyph_owned} "), glyph_style.patch(row_style)),
            Span::styled(node.name.clone(), row_style),
        ]));
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
            pg::render(frame, detail_area, d, state, bulletins, &state.detail_focus);
        }
        Some(NodeDetail::Processor(d)) => {
            processor::render(frame, detail_area, d, state, bulletins, &state.detail_focus);
        }
        Some(NodeDetail::Connection(d)) => {
            connection::render(frame, detail_area, d, state, &state.detail_focus);
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

/// Render the processor/CS properties modal overlay.
pub fn render_properties_modal(
    frame: &mut Frame,
    area: Rect,
    modal: &crate::view::browser::state::PropertiesModalState,
    state: &BrowserState,
) {
    let w = area.width.min(layout::BROWSER_DETAIL_MODAL_MAX_WIDTH);
    let h = area.height.min(layout::BROWSER_DETAIL_MODAL_MAX_HEIGHT);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    let (name, props) = match state.details.get(&modal.arena_idx) {
        Some(NodeDetail::Processor(p)) => (p.name.clone(), p.properties.clone()),
        Some(NodeDetail::ControllerService(c)) => (c.name.clone(), c.properties.clone()),
        _ => (String::new(), Vec::new()),
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(" Properties — {name} — esc to close "));
    let inner = block.inner(rect);
    frame.render_widget(ratatui::widgets::Clear, rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    for (k, v) in props.iter() {
        let key = format!("{k:30}");
        lines.push(Line::from(vec![
            Span::styled(key, theme::muted()),
            Span::raw(" "),
            Span::raw(v.clone()),
        ]));
    }
    let start = modal.scroll.min(lines.len().saturating_sub(1));
    let windowed: Vec<Line> = lines
        .into_iter()
        .skip(start)
        .take(inner.height as usize)
        .collect();
    frame.render_widget(Paragraph::new(windowed), inner);
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
        let mut terminal = Terminal::new(TestBackend::new(120, 30)).unwrap();
        terminal
            .draw(|f| {
                super::render(f, f.area(), state, &None, &bulletins);
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
                    },
                },
            ],
            fetched_at,
            cs_fetch_error: None,
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
            cs_fetch_error: None,
        };
        apply_tree_snapshot(&mut s, snap);
        s.last_tree_fetched_at = Some(SystemTime::now() - std::time::Duration::from_secs(3));
        insta::with_settings!(
            { filters => vec![(r"last [^\s]+ ago", "last <DUR> ago")] },
            { assert_snapshot!("browser_tree_queues_and_cs_folders_collapsed", render_to_string(&s)); }
        );
    }
}
