//! Browser Properties modal — two-column selectable table with a
//! detail strip that wraps the selected row's full value.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Wrap};

use crate::app::navigation::compute_scroll_window;
use crate::layout;
use crate::theme;
use crate::view::browser::state::{BrowserState, NodeDetail, PropertiesModalState, property_rows};

/// Property column width. Matches the padding the old modal used
/// (`{k:30}`) so downstream screenshots stay comparable.
const KEY_COL_WIDTH: u16 = 30;
/// Fixed height of the detail strip (including its top separator).
const DETAIL_STRIP_HEIGHT: u16 = 3;

/// Render the processor/CS properties modal overlay.
pub fn render_properties_modal(
    frame: &mut Frame,
    area: Rect,
    modal: &PropertiesModalState,
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

    let total = props.len();
    let selected = modal.selected.min(total.saturating_sub(1));

    let scroll_label = if total == 0 {
        String::new()
    } else {
        format!(" \u{2195}{}/{} ", selected + 1, total)
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!(
            " Properties \u{2014} {name} \u{2014} esc to close "
        ))
        .title_bottom(scroll_label);
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    // Split body: table (flex) + detail strip (fixed 3 lines).
    let [table_area, detail_area] =
        Layout::vertical([Constraint::Min(1), Constraint::Length(DETAIL_STRIP_HEIGHT)])
            .areas(inner);

    render_table(frame, table_area, state, &props, selected);
    render_detail_strip(frame, detail_area, &props, selected);
}

fn render_table(
    frame: &mut Frame,
    area: Rect,
    state: &BrowserState,
    props: &[(String, String)],
    selected: usize,
) {
    let rows_data = property_rows(state, props);

    // Reserve 1 row for the header.
    let visible_rows = area.height.saturating_sub(1) as usize;
    let window = compute_scroll_window(selected, rows_data.len(), visible_rows);

    // Value column width = total inner width − key column − 1 gap.
    let value_col_width = area.width.saturating_sub(KEY_COL_WIDTH + 1);

    let header = Row::new(vec![
        Cell::from(Span::styled("Property", theme::muted())),
        Cell::from(Span::styled("Value", theme::muted())),
    ]);

    let rows: Vec<Row> = rows_data
        .iter()
        .enumerate()
        .skip(window.offset)
        .take(visible_rows)
        .map(|(i, pr)| {
            let key_cell = Cell::from(pr.key.to_string());
            let value_cell = value_cell(pr, value_col_width);
            let row = Row::new(vec![key_cell, value_cell]);
            if i == selected {
                row.style(Style::default().add_modifier(Modifier::REVERSED))
            } else {
                row
            }
        })
        .collect();

    let table = Table::new(
        rows,
        [Constraint::Length(KEY_COL_WIDTH), Constraint::Min(1)],
    )
    .header(header)
    .column_spacing(1);

    frame.render_widget(table, area);
}

/// Build a value cell. Truncates with ellipsis and appends a
/// cross-link arrow when the value resolves to a known arena node.
fn value_cell<'a>(
    row: &'a crate::view::browser::state::PropertyRow<'a>,
    col_width: u16,
) -> Cell<'a> {
    let arrow_reserve: u16 = if row.resolves_to.is_some() { 2 } else { 0 };
    let max_value_chars = col_width.saturating_sub(arrow_reserve) as usize;
    let truncated = truncate_ellipsis(row.value, max_value_chars);
    let mut spans: Vec<Span> = vec![Span::raw(truncated)];
    if row.resolves_to.is_some() {
        spans.push(Span::raw(" "));
        spans.push(Span::styled("\u{2192}", theme::accent()));
    }
    Cell::from(Line::from(spans))
}

fn truncate_ellipsis(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max_chars.saturating_sub(1)).collect();
    out.push('\u{2026}');
    out
}

fn render_detail_strip(frame: &mut Frame, area: Rect, props: &[(String, String)], selected: usize) {
    // Top separator line + wrapped full value.
    let [sep_area, value_area] =
        Layout::vertical([Constraint::Length(1), Constraint::Min(1)]).areas(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "\u{2500}".repeat(area.width as usize),
            theme::muted(),
        ))),
        sep_area,
    );

    let value = props
        .get(selected)
        .map(|(_, v)| v.clone())
        .unwrap_or_default();
    let paragraph = Paragraph::new(value).wrap(Wrap { trim: false });
    frame.render_widget(paragraph, value_area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::ProcessorDetail;
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::view::browser::state::{BrowserState, NodeDetail, PropertiesModalState, TreeNode};
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn seeded_state_with_modal() -> (BrowserState, PropertiesModalState) {
        let mut s = BrowserState::new();
        // Root PG (arena 0).
        s.nodes.push(TreeNode {
            parent: None,
            children: vec![1, 2],
            kind: NodeKind::ProcessGroup,
            id: "root".into(),
            group_id: String::new(),
            name: "root".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        });
        // Processor (arena 1).
        s.nodes.push(TreeNode {
            parent: Some(0),
            children: vec![],
            kind: NodeKind::Processor,
            id: "gen".into(),
            group_id: "root".into(),
            name: "Gen".into(),
            status_summary: NodeStatusSummary::Processor {
                run_status: "Running".into(),
            },
        });
        // CS (arena 2) — id is a UUID so properties can reference it.
        let cs_uuid = "7f3e1c22-1111-4444-8888-abcdef012345".to_string();
        s.nodes.push(TreeNode {
            parent: Some(0),
            children: vec![],
            kind: NodeKind::ControllerService,
            id: cs_uuid.clone(),
            group_id: "root".into(),
            name: "fixture-json-reader".into(),
            status_summary: NodeStatusSummary::ControllerService {
                state: "ENABLED".into(),
            },
        });

        s.details.insert(
            1,
            NodeDetail::Processor(ProcessorDetail {
                id: "gen".into(),
                name: "Gen".into(),
                type_name: "x".into(),
                bundle: String::new(),
                run_status: "Running".into(),
                scheduling_strategy: String::new(),
                scheduling_period: String::new(),
                concurrent_tasks: 1,
                run_duration_ms: 0,
                penalty_duration: String::new(),
                yield_duration: String::new(),
                bulletin_level: String::new(),
                properties: vec![
                    ("Log Level".into(), "info".into()),
                    ("Record Reader".into(), cs_uuid.clone()),
                    (
                        "Expression".into(),
                        "${filename:substringBefore('.')}-processed".into(),
                    ),
                ],
                validation_errors: vec![],
            }),
        );

        let mut ps = PropertiesModalState::new(1);
        ps.selected = 1; // select the UUID row so the → marker shows
        (s, ps)
    }

    #[test]
    fn properties_modal_renders_table_with_arrow_and_detail_strip() {
        let (state, modal) = seeded_state_with_modal();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|f| render_properties_modal(f, f.area(), &modal, &state))
            .unwrap();
        let rendered = format!("{}", terminal.backend());
        assert_snapshot!(rendered);
    }
}
