//! Connection detail renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ConnectionDetail;
use crate::theme;
use crate::view::browser::state::BrowserState;

pub fn render(frame: &mut Frame, area: Rect, d: &ConnectionDetail, _state: &BrowserState) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Connection — {}", d.name),
        theme::accent(),
    )));
    lines.push(Line::from(format!(
        "From: {} ({})   To: {} ({})",
        d.source_name, d.source_type, d.destination_name, d.destination_type
    )));
    lines.push(Line::from(format!(
        "Relationships: {}",
        if d.selected_relationships.is_empty() {
            "(none)".to_string()
        } else {
            d.selected_relationships.join(", ")
        }
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(format!(
        "Fill: {}%  ({} ff / {})",
        d.fill_percent, d.flow_files_queued, d.queued_display
    )));
    lines.push(Line::from(format!(
        "Back-pressure thresholds: count={}, size={}",
        d.back_pressure_object_threshold, d.back_pressure_data_size_threshold
    )));
    lines.push(Line::from(format!(
        "Expiration: {}",
        if d.flow_file_expiration.is_empty() {
            "none".to_string()
        } else {
            d.flow_file_expiration.clone()
        }
    )));
    lines.push(Line::from(format!(
        "Load balance: {}",
        d.load_balance_strategy
    )));
    frame.render_widget(Paragraph::new(lines), area);
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
