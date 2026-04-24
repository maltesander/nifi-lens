//! Renderer for the Overview tab's bottom "Unhealthy queues" panel.

use ratatui::Frame;
use ratatui::layout::{Constraint, Rect};
use ratatui::style::Style;
use ratatui::text::Span;
use ratatui::widgets::{Cell, Row, Table};

use super::super::state::UnhealthyQueue;
use super::fill_style;
use crate::app::navigation::compute_scroll_window;
use crate::theme;

pub(super) fn render_unhealthy_queues(
    frame: &mut Frame,
    area: Rect,
    queues: &[UnhealthyQueue],
    focused: bool,
    selected: usize,
) {
    let rows: Vec<Row> = if queues.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(""),
            Cell::from(Span::styled("no queues reported yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        let visible_rows = area.height.saturating_sub(1) as usize;
        let window = compute_scroll_window(selected, queues.len(), visible_rows);
        queues
            .iter()
            .skip(window.offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, q)| {
                let style = fill_style(q.fill_percent);
                let row_style = if focused && idx == window.selected_local {
                    theme::cursor_row()
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(format!("{:>3}%", q.fill_percent)).style(style),
                    Cell::from(q.name.clone()),
                    Cell::from(format!("{} → {}", q.source_name, q.destination_name)),
                    Cell::from(q.flow_files_queued.to_string()),
                ])
                .style(row_style)
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(5),
            Constraint::Fill(2),
            Constraint::Fill(3),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["fill", "queue", "src → dst", "ffiles"]).style(theme::bold()));
    frame.render_widget(table, area);
}
