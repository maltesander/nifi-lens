//! Process Group detail renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ProcessGroupDetail;
use crate::theme;
use crate::view::browser::state::BrowserState;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessGroupDetail,
    _state: &BrowserState,
    _bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
) {
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!("Process Group — {}", d.name),
        theme::accent(),
    )));
    lines.push(Line::from(format!(
        "ID: {}   Parent: {}",
        d.id,
        d.parent_group_id.as_deref().unwrap_or("(root)")
    )));
    lines.push(Line::from(format!(
        "Running: {} · Stopped: {} · Invalid: {} · Disabled: {} · Active threads: {}",
        d.running, d.stopped, d.invalid, d.disabled, d.active_threads
    )));
    lines.push(Line::from(format!(
        "Flow files queued: {} · Bytes queued: {}",
        d.flow_files_queued, d.queued_display
    )));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Controller services ({})", d.controller_services.len()),
        theme::accent(),
    )));
    for cs in d.controller_services.iter().take(8) {
        lines.push(Line::from(format!(
            "  {state}  {name}   {type_}",
            state = cs.state,
            name = cs.name,
            type_ = cs.type_short
        )));
    }
    if d.controller_services.len() > 8 {
        lines.push(Line::from(Span::styled(
            format!("  …{} more", d.controller_services.len() - 8),
            theme::muted(),
        )));
    }
    lines.push(Line::from(""));
    // Rendering PG-scoped bulletins requires access to `AppState.bulletins`,
    // which is not in `BrowserState`. For Phase 3 v1 we render the block
    // header with a zero count; threading the real ring is flagged as a
    // Phase 5 polish item.
    lines.push(Line::from(Span::styled(
        "Recent bulletins (0 in this PG)",
        theme::accent(),
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
    fn pg_detail_with_cs_list() {
        let d = ProcessGroupDetail {
            id: "ingest".into(),
            name: "ingest".into(),
            parent_group_id: Some("root".into()),
            running: 3,
            stopped: 1,
            invalid: 0,
            disabled: 0,
            active_threads: 1,
            flow_files_queued: 4,
            bytes_queued: 2048,
            queued_display: "4 / 2 KB".into(),
            controller_services: vec![
                crate::client::ControllerServiceSummary {
                    id: "cs1".into(),
                    name: "http-pool".into(),
                    type_short: "StandardRestrictedSSLContextService".into(),
                    state: "ENABLED".into(),
                },
                crate::client::ControllerServiceSummary {
                    id: "cs2".into(),
                    name: "kafka-brokers".into(),
                    type_short: "Kafka3ConnectionService".into(),
                    state: "DISABLED".into(),
                },
            ],
        };
        let state = BrowserState::new();
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins))
            .unwrap();
        assert_snapshot!("pg_detail_with_cs_list", format!("{}", terminal.backend()));
    }
}
