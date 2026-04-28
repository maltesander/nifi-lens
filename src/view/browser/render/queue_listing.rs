//! Renderer for the connection-detail flowfile listing panel. Pure
//! function over `&QueueListingState`. Composed by
//! `render::connection::render` (Task 18) when the selection is a
//! Connection node.

use std::time::Duration;

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Paragraph, Row, Table};

use crate::bytes::format_bytes;
use crate::theme;
use crate::timestamp::format_age_secs;
use crate::view::browser::state::queue_listing::QueueListingState;
use crate::widget::panel::Panel;

const COL_POS: u16 = 4;
const COL_SIZE: u16 = 9;
const COL_QUEUED: u16 = 7;
const COL_PEN: u16 = 4;
const COL_NODE: u16 = 14;
const COL_UUID: u16 = 9;

/// Render the flowfile queue listing panel into `area`.
///
/// - `age_warning`: rows with `queued_duration > age_warning` render in
///   `theme::warning()`. Pass `Duration::ZERO` to disable age highlighting.
/// - `is_empty_queue`: when the parent connection reports `flow_files_queued == 0`,
///   the renderer shows a muted "queue empty" line instead of launching a fetch.
/// - `show_node_column`: add the `Node` column only for clustered NiFi (more
///   than one node in the roster).
/// - `focused`: drives `Panel::focused(true)` for the thick-border accent.
pub fn render_queue_listing(
    f: &mut Frame<'_>,
    area: Rect,
    state: &QueueListingState,
    age_warning: Duration,
    is_empty_queue: bool,
    show_node_column: bool,
    focused: bool,
) {
    let title = Line::from(Span::raw("Flowfiles"));
    let chips = build_right_chips(state, is_empty_queue);
    let block = Panel::new(title).focused(focused).right(chips).into_block();
    let inner = block.inner(area);
    f.render_widget(block, area);

    if is_empty_queue {
        let line = Line::from(Span::styled(
            "queue empty — nothing to list",
            theme::muted(),
        ));
        f.render_widget(Paragraph::new(line), inner);
        return;
    }

    if state.timed_out {
        let line = Line::from(Span::styled(
            "listing timeout — press r to retry",
            theme::warning(),
        ));
        f.render_widget(Paragraph::new(line), inner);
        return;
    }

    if let Some(err) = &state.error {
        let line = Line::from(vec![
            Span::styled("error: ", theme::warning()),
            Span::raw(err.clone()),
            Span::raw("  (r to retry)"),
        ]);
        f.render_widget(Paragraph::new(line), inner);
        return;
    }

    if state.rows.is_empty() {
        // In-flight: the loading chip in the title carries the percent.
        let line = Line::from(Span::styled("…", theme::muted()));
        f.render_widget(Paragraph::new(line), inner);
        return;
    }

    let visible = state.visible_indices();
    let header = build_header(show_node_column);
    let rows: Vec<Row> = visible
        .iter()
        .enumerate()
        .map(|(disp_idx, &row_idx)| {
            let r = &state.rows[row_idx];
            let aged = !age_warning.is_zero() && r.queued_duration > age_warning;
            let mut row_style = if aged {
                theme::warning()
            } else {
                Style::default()
            };
            if disp_idx == state.selected {
                // Selection style wins over age-warning for visual contrast.
                row_style = theme::accent().add_modifier(Modifier::REVERSED);
            }

            let mut cells: Vec<Cell> = vec![
                Cell::from(format!("{}", r.position)),
                Cell::from(r.filename.clone().unwrap_or_else(|| "—".to_string())),
                Cell::from(format_bytes(r.size)),
                Cell::from(format_age_secs(r.queued_duration.as_secs())),
                Cell::from(if r.penalized {
                    Span::styled("PEN", theme::warning().add_modifier(Modifier::BOLD))
                } else {
                    Span::raw("")
                }),
            ];
            if show_node_column {
                cells.push(Cell::from(
                    r.cluster_node_id.clone().unwrap_or_else(|| "—".to_string()),
                ));
            }
            cells.push(Cell::from(uuid_short(&r.uuid)));
            Row::new(cells).style(row_style)
        })
        .collect();

    let mut widths = vec![
        Constraint::Length(COL_POS),
        Constraint::Min(20),
        Constraint::Length(COL_SIZE),
        Constraint::Length(COL_QUEUED),
        Constraint::Length(COL_PEN),
    ];
    if show_node_column {
        widths.push(Constraint::Length(COL_NODE));
    }
    widths.push(Constraint::Length(COL_UUID));

    let table = Table::new(rows, widths).header(header);
    f.render_widget(table, inner);
}

fn build_right_chips(state: &QueueListingState, is_empty: bool) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    if is_empty {
        return Line::from(spans);
    }
    let visible = state.visible_indices();
    spans.push(Span::raw(format!("[{} rows]", visible.len())));
    if state.truncated {
        spans.push(Span::raw(format!(
            "  [{} / {}]",
            state.rows.len(),
            state.total
        )));
    }
    if state.percent < 100 && state.error.is_none() && !state.timed_out && state.rows.is_empty() {
        spans.push(Span::styled(
            format!("  [loading… {}%]", state.percent),
            theme::muted(),
        ));
    }
    if let Some(prompt) = &state.filter_prompt {
        spans.push(Span::styled(
            format!("  /{}_", prompt.draft),
            theme::accent(),
        ));
    } else if let Some(filter) = &state.filter {
        spans.push(Span::raw(format!("  filter:{filter}")));
    }
    Line::from(spans)
}

fn build_header(show_node_column: bool) -> Row<'static> {
    let mut cells: Vec<Cell> = vec![
        Cell::from("Pos"),
        Cell::from("Filename"),
        Cell::from("Size"),
        Cell::from("Queued"),
        Cell::from("Pen"),
    ];
    if show_node_column {
        cells.push(Cell::from("Node"));
    }
    cells.push(Cell::from("UUID"));
    Row::new(cells).style(theme::muted())
}

fn uuid_short(uuid: &str) -> String {
    uuid.chars().take(8).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::browser::state::queue_listing::{QueueListingRow, QueueListingState};
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::layout::Rect;
    use std::time::Duration;

    fn row(uuid: &str, fname: &str, queued_ms: u64, penalized: bool) -> QueueListingRow {
        QueueListingRow {
            uuid: uuid.into(),
            filename: Some(fname.into()),
            size: 1024 * 1024,
            queued_duration: Duration::from_millis(queued_ms),
            position: 1,
            penalized,
            cluster_node_id: None,
            lineage_duration: Duration::from_millis(queued_ms * 2),
        }
    }

    fn render_to_string(
        width: u16,
        height: u16,
        render: impl FnOnce(&mut ratatui::Frame<'_>, Rect),
    ) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| render(f, Rect::new(0, 0, width, height)))
            .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn renders_loading_chip_during_in_flight() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.percent = 50;
        let out = render_to_string(80, 20, |f, area| {
            render_queue_listing(
                f,
                area,
                &s,
                Duration::from_secs(5 * 60),
                false,
                false,
                false,
            );
        });
        assert!(out.contains("loading"), "expected loading chip in:\n{out}");
        assert!(out.contains("50"), "expected percent in:\n{out}");
    }

    #[test]
    fn renders_empty_queue_muted_line() {
        let s = QueueListingState::pending("q1".into(), "Q1".into());
        let out = render_to_string(80, 20, |f, area| {
            render_queue_listing(f, area, &s, Duration::from_secs(5 * 60), true, false, false);
        });
        assert!(
            out.contains("queue empty"),
            "expected empty muted line in:\n{out}"
        );
    }

    #[test]
    fn renders_truncation_chip_when_over_100() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = (0..100)
            .map(|i| row(&format!("ff-{i}"), "a.txt", 1000, false))
            .collect();
        s.total = 4827;
        s.truncated = true;
        s.percent = 100;
        let out = render_to_string(100, 40, |f, area| {
            render_queue_listing(
                f,
                area,
                &s,
                Duration::from_secs(5 * 60),
                false,
                false,
                false,
            );
        });
        assert!(out.contains("100"), "expected limit numerator:\n{out}");
        assert!(out.contains("4827"), "expected total denominator:\n{out}");
    }

    #[test]
    fn renders_pen_chip_for_penalized_rows() {
        let mut s = QueueListingState::pending("q1".into(), "Q1".into());
        s.rows = vec![row("ff-1", "a.txt", 1000, true)];
        s.total = 1;
        s.percent = 100;
        let out = render_to_string(80, 20, |f, area| {
            render_queue_listing(
                f,
                area,
                &s,
                Duration::from_secs(5 * 60),
                false,
                false,
                false,
            );
        });
        assert!(out.contains("PEN"), "expected PEN chip:\n{out}");
    }
}
