//! Renderer for the Overview tab's middle strip: a bulletins sparkline
//! on the left and a "noisy components" table on the right.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Bar, BarChart, BarGroup, Cell, Paragraph, Row, Table};

use super::super::state::{
    BulletinBucket, NoisyComponent, OverviewFocus, OverviewState, SPARKLINE_MINUTES, Severity,
};
use crate::app::navigation::compute_scroll_window;
use crate::theme;
use crate::widget::panel::Panel;

pub(super) fn render_bulletins_and_noisy(
    frame: &mut Frame,
    bulletins_area: Rect,
    noisy_area: Rect,
    state: &OverviewState,
) {
    let bulletins_block = Panel::new(" Bulletins / min ").into_block();
    let bulletins_inner = bulletins_block.inner(bulletins_area);
    frame.render_widget(bulletins_block, bulletins_area);
    render_bulletin_sparkline(frame, bulletins_inner, &state.sparkline);

    let noisy_focused = state.focus == OverviewFocus::Noisy;
    let noisy_block = Panel::new(" Noisy components ")
        .focused(noisy_focused)
        .into_block();
    let noisy_inner = noisy_block.inner(noisy_area);
    frame.render_widget(noisy_block, noisy_area);
    render_noisy_components(
        frame,
        noisy_inner,
        &state.noisy,
        noisy_focused,
        state.noisy_selected,
    );
}

fn render_bulletin_sparkline(frame: &mut Frame, area: Rect, buckets: &[BulletinBucket]) {
    // Trim leading zero-count buckets so bars start from the left edge
    // rather than appearing mid-chart while the window is still filling up.
    let first_nonempty = buckets
        .iter()
        .position(|b| b.count > 0)
        .unwrap_or(buckets.len());
    let visible = &buckets[first_nonempty..];

    // Layout: legend (1) | grouped bars (3) | time axis (1).
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .split(area);

    // ── Legend ────────────────────────────────────────────────────────────────
    let legend = if visible.is_empty() {
        Line::from(Span::styled(
            format!("{}m  no bulletins", SPARKLINE_MINUTES),
            theme::muted(),
        ))
    } else {
        Line::from(vec![
            Span::styled(format!("{}m  ", SPARKLINE_MINUTES), theme::muted()),
            Span::styled("■ Err  ", Severity::Error.style()),
            Span::styled("■ Warn  ", Severity::Warning.style()),
            Span::styled("■ Info", Severity::Info.style()),
        ])
    };
    frame.render_widget(Paragraph::new(legend), chunks[0]);

    if visible.is_empty() {
        return;
    }

    // One BarGroup per visible minute with three bars (Error, Warning, Info).
    // All bars share the same chart so they scale to a common maximum —
    // this makes them grow as a unit rather than each track independently.
    //
    // bar_width: fill available space across visible groups, 3 bars each,
    // with a 1-column gap between groups for readability.
    const GROUP_GAP: u16 = 1;
    let n = visible.len() as u16;
    let bar_width = area
        .width
        .saturating_sub(n.saturating_sub(1) * GROUP_GAP)
        .checked_div(n * 3)
        .unwrap_or(1)
        .max(1);

    let groups: Vec<BarGroup> = visible
        .iter()
        .map(|b| {
            BarGroup::default().bars(&[
                Bar::default()
                    .value(b.error_count as u64)
                    .style(Severity::Error.style())
                    .text_value(""),
                Bar::default()
                    .value(b.warning_count as u64)
                    .style(Severity::Warning.style())
                    .text_value(""),
                Bar::default()
                    .value(b.info_count as u64)
                    .style(Severity::Info.style())
                    .text_value(""),
            ])
        })
        .collect();

    frame.render_widget(
        BarChart::grouped(groups)
            .bar_width(bar_width)
            .bar_gap(0)
            .group_gap(GROUP_GAP)
            .bar_set(symbols::bar::NINE_LEVELS),
        chunks[1],
    );

    // ── Time-axis grid ────────────────────────────────────────────────────────
    let oldest_min = SPARKLINE_MINUTES - first_nonempty;
    let left_label = format!("←{}m", oldest_min);
    let right_label = "now→";
    let fill = (area.width as usize).saturating_sub(left_label.len() + right_label.len());
    let axis_line = Line::from(vec![
        Span::styled(left_label, theme::muted()),
        Span::raw(" ".repeat(fill)),
        Span::styled(right_label, theme::muted()),
    ]);
    frame.render_widget(Paragraph::new(axis_line), chunks[2]);
}

fn render_noisy_components(
    frame: &mut Frame,
    area: Rect,
    noisy: &[NoisyComponent],
    focused: bool,
    selected: usize,
) {
    let rows: Vec<Row> = if noisy.is_empty() {
        vec![Row::new(vec![
            Cell::from(""),
            Cell::from(Span::styled("no bulletins yet", theme::muted())),
            Cell::from(""),
        ])]
    } else {
        let visible_rows = area.height.saturating_sub(1) as usize;
        let window = compute_scroll_window(selected, noisy.len(), visible_rows);
        noisy
            .iter()
            .skip(window.offset)
            .take(visible_rows)
            .enumerate()
            .map(|(idx, n)| {
                let sev_style = n.max_severity.style();
                let row_style = if focused && idx == window.selected_local {
                    theme::cursor_row()
                } else {
                    Style::default()
                };
                Row::new(vec![
                    Cell::from(format!("{:>3}", n.count)).style(theme::bold()),
                    Cell::from(n.source_name.clone()),
                    Cell::from(format!("{:?}", n.max_severity)).style(sev_style),
                ])
                .style(row_style)
            })
            .collect()
    };
    let table = Table::new(
        rows,
        [
            Constraint::Length(4),
            Constraint::Fill(1),
            Constraint::Length(8),
        ],
    )
    .header(Row::new(vec!["cnt", "source", "worst"]).style(theme::bold()));
    frame.render_widget(table, area);
}
