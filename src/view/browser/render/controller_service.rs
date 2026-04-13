//! Controller Service detail renderer.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::client::ControllerServiceDetail;
use crate::theme;
use crate::view::browser::state::BrowserState;

const INLINE_PROPERTY_ROWS: usize = 10;

pub fn render(frame: &mut Frame, area: Rect, d: &ControllerServiceDetail, _state: &BrowserState) {
    let mut lines: Vec<Line> = Vec::new();

    // Header: "<name>  controller service" with state chip.
    lines.push(Line::from(vec![
        Span::styled(
            d.name.clone(),
            theme::accent().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        Span::raw("  "),
        Span::styled("controller service".to_string(), theme::muted()),
        Span::raw("  "),
        state_chip(&d.state),
    ]));
    lines.push(Line::from(format!("Type:   {}", d.type_name)));
    lines.push(Line::from(format!("Bundle: {}", d.bundle)));
    lines.push(Line::from(format!(
        "Parent: {}",
        d.parent_group_id.as_deref().unwrap_or("(controller)")
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
        for err in d.validation_errors.iter().take(3) {
            lines.push(Line::from(format!("  {}", truncate(err, max_err_width))));
        }
    }

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

fn state_chip(state: &str) -> Span<'static> {
    let label = format!("[{state}]");
    match state {
        "ENABLED" => Span::styled(
            label,
            theme::success().add_modifier(ratatui::style::Modifier::BOLD),
        ),
        "DISABLED" => Span::styled(label, theme::muted()),
        _ => Span::styled(label, theme::warning()),
    }
}

#[cfg(test)]
mod snapshots {
    use super::*;
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn cs_detail_renders() {
        let d = ControllerServiceDetail {
            id: "cs1".into(),
            name: "http-pool".into(),
            type_name: "org.apache.nifi.ssl.StandardRestrictedSSLContextService".into(),
            bundle: "org.apache.nifi:nifi-ssl-context-service-nar:2.8.0".into(),
            state: "ENABLED".into(),
            parent_group_id: Some("ingest".into()),
            properties: vec![
                ("Keystore Filename".into(), "/opt/nifi/keystore.jks".into()),
                ("Keystore Type".into(), "JKS".into()),
                (
                    "Truststore Filename".into(),
                    "/opt/nifi/truststore.jks".into(),
                ),
                ("SSL Protocol".into(), "TLSv1.2".into()),
            ],
            validation_errors: vec![],
            bulletin_level: "WARN".into(),
        };
        let state = BrowserState::new();
        let mut terminal = Terminal::new(TestBackend::new(100, 18)).unwrap();
        terminal.draw(|f| render(f, f.area(), &d, &state)).unwrap();
        assert_snapshot!("cs_detail_renders", format!("{}", terminal.backend()));
    }
}
