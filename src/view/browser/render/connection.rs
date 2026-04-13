//! Connection detail renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ConnectionDetail;
use crate::theme;
use crate::view::browser::state::BrowserState;

pub fn render(frame: &mut Frame, area: Rect, d: &ConnectionDetail, _state: &BrowserState) {
    use crate::widget::gauge::fill_bar;
    let mut lines: Vec<Line> = Vec::new();

    // Header: "<name>  connection"
    lines.push(Line::from(vec![
        Span::styled(
            d.name.clone(),
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("connection".to_string(), theme::muted()),
    ]));
    lines.push(Line::from(""));

    // Fill gauge: prominent visual block.
    let gauge_width: u16 = area.width.saturating_sub(12).clamp(8, 40);
    let bar = fill_bar(gauge_width, d.fill_percent);
    let gauge_style = fill_style(d.fill_percent);
    lines.push(Line::from(vec![
        Span::styled("Fill        ".to_string(), theme::muted()),
        Span::styled(bar, gauge_style),
        Span::raw(format!(
            "  {}% ({} ff / {})",
            d.fill_percent, d.flow_files_queued, d.queued_display
        )),
    ]));

    // Source / Destination block.
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("From        ".to_string(), theme::muted()),
        Span::raw(format!("{} ({})", d.source_name, d.source_type)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("To          ".to_string(), theme::muted()),
        Span::raw(format!("{} ({})", d.destination_name, d.destination_type)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("Relations   ".to_string(), theme::muted()),
        Span::raw(if d.selected_relationships.is_empty() {
            "(none)".to_string()
        } else {
            d.selected_relationships.join(", ")
        }),
    ]));

    // Back-pressure block.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "Back-pressure".to_string(),
        theme::accent(),
    )));
    lines.push(Line::from(vec![
        Span::styled("  count     ".to_string(), theme::muted()),
        Span::raw(format!("{}", d.back_pressure_object_threshold)),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  size      ".to_string(), theme::muted()),
        Span::raw(d.back_pressure_data_size_threshold.clone()),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  expire    ".to_string(), theme::muted()),
        Span::raw(if d.flow_file_expiration.is_empty() {
            "none".to_string()
        } else {
            d.flow_file_expiration.clone()
        }),
    ]));
    lines.push(Line::from(vec![
        Span::styled("  load-bal  ".to_string(), theme::muted()),
        Span::raw(d.load_balance_strategy.clone()),
    ]));

    // Action hints.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "c copy id".to_string(),
        theme::muted(),
    )));

    frame.render_widget(Paragraph::new(lines), area);
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
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal.draw(|f| render(f, f.area(), &d, &state)).unwrap();
        assert_snapshot!(
            "connection_detail_renders",
            format!("{}", terminal.backend())
        );
    }
}
