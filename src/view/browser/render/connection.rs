//! Connection detail renderer.
//!
//! Phase 7 wraps the existing fill-gauge + endpoints + back-pressure
//! content in a bordered outer `Panel`. Connection detail has no
//! focusable sub-sections, so the `_detail_focus` parameter is
//! threaded for signature consistency with the other per-kind
//! renderers and currently ignored.

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
    _state: &BrowserState,
    _detail_focus: &DetailFocus,
) {
    // Outer panel: " <name> · connection "
    let outer = Panel::new(build_header_title(d)).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    render_content(frame, inner, d);
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

fn render_content(frame: &mut Frame, area: Rect, d: &ConnectionDetail) {
    use ratatui::layout::{Constraint, Direction, Layout};

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(6), Constraint::Fill(1)])
        .split(area);

    render_endpoints_panel(frame, rows[0], d);
    render_back_pressure_panel(frame, rows[1], d);
}

fn render_endpoints_panel(frame: &mut Frame, area: Rect, d: &ConnectionDetail) {
    use crate::widget::gauge::fill_bar;

    let block = Panel::new(" Endpoints ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Prominent fill gauge on the first content line.
    let gauge_width: u16 = inner.width.saturating_sub(12).clamp(8, 40);
    let bar = fill_bar(gauge_width, d.fill_percent);
    let gauge_style = fill_style(d.fill_percent);

    let lines = vec![
        Line::from(vec![
            Span::styled("Fill     ", theme::muted()),
            Span::styled(bar, gauge_style),
            Span::raw(format!(
                "  {}% ({} ff / {})",
                d.fill_percent, d.flow_files_queued, d.queued_display
            )),
        ]),
        Line::from(vec![
            Span::styled("From     ", theme::muted()),
            Span::raw(format!("{} ({})", d.source_name, d.source_type)),
        ]),
        Line::from(vec![
            Span::styled("To       ", theme::muted()),
            Span::raw(format!("{} ({})", d.destination_name, d.destination_type)),
        ]),
        Line::from(vec![
            Span::styled("Relations", theme::muted()),
            Span::raw(if d.selected_relationships.is_empty() {
                "(none)".to_string()
            } else {
                d.selected_relationships.join(", ")
            }),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
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
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn connection_detail_renders() {
        let d = ConnectionDetail {
            id: "c1".into(),
            name: "enrich → publish".into(),
            source_id: "p1".into(),
            source_name: "EnrichAttribute".into(),
            source_type: "PROCESSOR".into(),
            source_group_id: "ingest".into(),
            destination_id: "p2".into(),
            destination_name: "PublishKafka".into(),
            destination_type: "PROCESSOR".into(),
            destination_group_id: "publish".into(),
            selected_relationships: vec!["success".into()],
            available_relationships: vec!["success".into(), "failure".into()],
            back_pressure_object_threshold: 10000,
            back_pressure_data_size_threshold: "1 GB".into(),
            flow_file_expiration: "0 sec".into(),
            load_balance_strategy: "DO_NOT_LOAD_BALANCE".into(),
            fill_percent: 55,
            flow_files_queued: 5500,
            bytes_queued: 52_428_800,
            queued_display: "5,500 / 50 MB".into(),
        };
        let state = BrowserState::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &DetailFocus::Tree))
            .unwrap();
        assert_snapshot!(
            "connection_detail_renders",
            format!("{}", terminal.backend())
        );
    }
}
