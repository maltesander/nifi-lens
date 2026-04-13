//! Processor detail renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ProcessorDetail;
use crate::theme;
use crate::view::browser::state::BrowserState;
use crate::widget::severity::{format_severity_label, severity_style};

const INLINE_PROPERTY_ROWS: usize = 10;
const INLINE_VALIDATION_ROWS: usize = 3;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    d: &ProcessorDetail,
    _state: &BrowserState,
    bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
) {
    let mut lines: Vec<Line> = Vec::new();

    // Header: "<name>  processor"
    lines.push(Line::from(vec![
        Span::styled(
            d.name.clone(),
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("processor".to_string(), theme::muted()),
    ]));
    lines.push(Line::from(format!("Type:   {}", d.type_name)));
    lines.push(Line::from(format!("Bundle: {}", d.bundle)));
    lines.push(Line::from(format!(
        "State: {}   Concurrent: {}",
        d.run_status, d.concurrent_tasks
    )));
    lines.push(Line::from(format!(
        "Schedule: {} every {}",
        d.scheduling_strategy, d.scheduling_period
    )));
    lines.push(Line::from(format!(
        "Run duration: {} ms    Penalty: {}    Yield: {}",
        d.run_duration_ms, d.penalty_duration, d.yield_duration
    )));
    lines.push(Line::from(""));

    let m = d.properties.len();
    let n = INLINE_PROPERTY_ROWS.min(m);
    lines.push(Line::from(Span::styled(
        format!("Properties (showing {n} of {m})"),
        theme::accent(),
    )));
    for (k, v) in d.properties.iter().take(n) {
        let key = format!("  {:28}", truncate(k, 28));
        let val = truncate(v, 60);
        lines.push(Line::from(format!("{key} {val}")));
    }
    if m > INLINE_PROPERTY_ROWS {
        lines.push(Line::from(Span::styled(
            format!("  …{} more, press e to expand", m - INLINE_PROPERTY_ROWS),
            theme::muted(),
        )));
    }

    let ve = d.validation_errors.len();
    if ve == 0 {
        lines.push(Line::from("Validation errors: none"));
    } else {
        lines.push(Line::from(Span::styled(
            format!("Validation errors: {ve}"),
            theme::error(),
        )));
        let max_err_width = (area.width as usize).saturating_sub(4);
        for err in d.validation_errors.iter().take(INLINE_VALIDATION_ROWS) {
            lines.push(Line::from(format!("  {}", truncate(err, max_err_width))));
        }
        if ve > INLINE_VALIDATION_ROWS {
            lines.push(Line::from(Span::styled(
                format!("  …{} more", ve - INLINE_VALIDATION_ROWS),
                theme::muted(),
            )));
        }
    }

    // Recent bulletins section.
    let recent = crate::view::bulletins::state::recent_for_source_id(bulletins, &d.id, 3);
    let total = bulletins.iter().filter(|b| b.source_id == d.id).count();
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        format!("Recent bulletins ({total} for this processor)"),
        theme::accent(),
    )));
    for b in &recent {
        let sev = format_severity_label(&b.level);
        let sev_style = severity_style(&b.level);
        let stripped = crate::view::bulletins::state::strip_component_prefix(&b.message);
        lines.push(Line::from(vec![
            Span::raw("  "),
            Span::styled(sev, sev_style),
            Span::raw("  "),
            Span::raw(stripped.to_string()),
        ]));
    }

    // Action hints.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        "e properties · c copy id · t trace lineage".to_string(),
        theme::muted(),
    )));

    frame.render_widget(Paragraph::new(lines), area);
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let head: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{head}…")
    }
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn processor_detail_with_many_properties() {
        let d = ProcessorDetail {
            id: "put-kafka-1".into(),
            name: "PutKafka".into(),
            type_name: "org.apache.nifi.processors.kafka.pubsub.PublishKafka_2_6".into(),
            bundle: "org.apache.nifi:nifi-kafka-2-6-nar:2.8.0".into(),
            run_status: "RUNNING".into(),
            scheduling_strategy: "TIMER_DRIVEN".into(),
            scheduling_period: "1 sec".into(),
            concurrent_tasks: 2,
            run_duration_ms: 25,
            penalty_duration: "30 sec".into(),
            yield_duration: "1 sec".into(),
            bulletin_level: "WARN".into(),
            properties: (0..13)
                .map(|i| (format!("Property-{i}"), format!("value-{i}")))
                .collect(),
            validation_errors: vec!["'Kafka Key' invalid".into()],
        };
        let state = BrowserState::new();
        let bulletins: std::collections::VecDeque<crate::client::BulletinSnapshot> =
            std::collections::VecDeque::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 24)).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &d, &state, &bulletins))
            .unwrap();
        assert_snapshot!(
            "processor_detail_with_many_properties",
            format!("{}", terminal.backend())
        );
    }
}
