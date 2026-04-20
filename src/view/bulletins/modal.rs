//! Render the Bulletins detail modal.
//!
//! Full-screen overlay. The border is colored by severity (via
//! `Block::border_style`); the title carries the severity label and source
//! name. The scrollable body wraps at pane width; the footer advertises
//! modal-local shortcuts.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph, Wrap};

use crate::theme;
use crate::view::bulletins::state::{BulletinsState, DetailModalState};
use crate::widget::severity::severity_style;

const HEADER_ROWS: u16 = 3; // timing line · blank · ids line
const FOOTER_ROWS: u16 = 2; // blank · hint line

/// Render the modal. Assumes `state.detail_modal.is_some()`; no-op
/// otherwise. Writes `last_viewport_rows` back into the modal state
/// so reducers can do page-sized scrolls.
pub fn render(frame: &mut Frame, area: Rect, state: &mut BulletinsState) {
    let Some(modal) = state.detail_modal.as_mut() else {
        return;
    };

    frame.render_widget(Clear, area);

    let sev_style = severity_style(match modal.details.severity {
        crate::client::Severity::Error => "ERROR",
        crate::client::Severity::Warning => "WARN",
        crate::client::Severity::Info => "INFO",
        crate::client::Severity::Unknown => "",
    });

    let title = format!(
        " {} · {} ",
        severity_title(&modal.details.severity),
        modal.details.source_name,
    );

    // Use ratatui's Block directly so we can color the border by severity.
    // Panel doesn't expose a border_style setter, so we bypass it here.
    let block = Block::bordered()
        .border_type(BorderType::Plain)
        .border_style(sev_style)
        .title(title.as_str());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_ROWS),
            Constraint::Fill(1),
            Constraint::Length(FOOTER_ROWS),
        ])
        .split(inner);

    render_header(frame, rows[0], modal);
    render_body(frame, rows[1], modal);
    render_footer(frame, rows[2]);

    modal.last_viewport_rows = rows[1].height as usize;
}

fn render_header(frame: &mut Frame, area: Rect, modal: &DetailModalState) {
    let mut lines: Vec<Line<'static>> = Vec::new();
    if modal.details.count > 1 {
        lines.push(Line::from(Span::styled(
            format!(
                "first {} · last {} · ×{} occurrences",
                short_time(&modal.details.first_seen_iso),
                short_time(&modal.details.last_seen_iso),
                modal.details.count,
            ),
            theme::muted(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            format!("at {}", short_time(&modal.details.first_seen_iso)),
            theme::muted(),
        )));
    }
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("src-id  ", theme::muted()),
        Span::raw(modal.details.source_id.clone()),
        Span::raw("   "),
        Span::styled("pg-id  ", theme::muted()),
        Span::raw(modal.details.group_id.clone()),
    ]));
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_body(frame: &mut Frame, area: Rect, modal: &mut DetailModalState) {
    let body = modal.details.raw_message.clone();
    let estimated_rows = estimate_wrapped_rows(&body, area.width as usize);
    let max_offset = estimated_rows.saturating_sub(area.height as usize);
    if modal.scroll_offset > max_offset {
        modal.scroll_offset = max_offset;
    }
    frame.render_widget(
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .scroll((modal.scroll_offset as u16, 0)),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect) {
    let hint = Line::from(Span::styled(
        "↑↓ scroll · PgUp/PgDn page · c copy · Enter Browser · Esc close",
        theme::muted(),
    ));
    frame.render_widget(Paragraph::new(vec![Line::from(""), hint]), area);
}

/// Approximate wrap-aware row count for `body` rendered at `width`.
/// Used only for upward-clamping `scroll_offset`. Over-estimates are
/// harmless (scroll shows blanks at the bottom); under-estimates would
/// pin content off-screen, so we err on the side of generous.
pub(crate) fn estimate_wrapped_rows(body: &str, width: usize) -> usize {
    if body.is_empty() {
        return 0;
    }
    if width == 0 {
        return body.lines().count();
    }
    body.lines()
        .map(|l| {
            let chars = l.chars().count();
            if chars == 0 { 1 } else { chars.div_ceil(width) }
        })
        .sum()
}

fn severity_title(sev: &crate::client::Severity) -> &'static str {
    match sev {
        crate::client::Severity::Error => "ERROR",
        crate::client::Severity::Warning => "WARN",
        crate::client::Severity::Info => "INFO",
        crate::client::Severity::Unknown => "",
    }
}

/// Extract HH:MM:SS from an ISO-8601 / RFC-3339 timestamp. Returns
/// `"--:--:--"` on parse failure.
fn short_time(iso: &str) -> String {
    let Some(dt) = crate::timestamp::parse_nifi_timestamp(iso) else {
        return "--:--:--".to_string();
    };
    let t = dt.time();
    format!("{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_wrapped_rows_handles_empty_and_blank_lines() {
        assert_eq!(estimate_wrapped_rows("", 80), 0);
        // "\n\n".lines() yields ["", ""] → 2 rows (each empty line maps to 1)
        assert_eq!(estimate_wrapped_rows("\n\n", 80), 2);
    }

    #[test]
    fn estimate_wrapped_rows_breaks_by_width() {
        assert_eq!(estimate_wrapped_rows("abcdefghij", 5), 2);
        assert_eq!(estimate_wrapped_rows("abcdefghij", 10), 1);
        assert_eq!(estimate_wrapped_rows("abcdefghij", 3), 4);
    }

    #[test]
    fn estimate_wrapped_rows_width_zero_falls_back_to_line_count() {
        assert_eq!(estimate_wrapped_rows("a\nb\nc", 0), 3);
    }

    #[test]
    fn modal_renders_short_message() {
        use crate::client::BulletinSnapshot;
        use crate::view::bulletins::state::BulletinsState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;

        let mut state = BulletinsState::with_capacity(10);
        state.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "short error".into(),
            source_id: "src-1".into(),
            source_name: "PutDb".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-20T10:14:22Z".into(),
            timestamp_human: String::new(),
        });
        state.selected = 0;
        state.auto_scroll = false;
        state.open_detail_modal();

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 60, 15), &mut state);
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_short_message", terminal.backend().buffer());
    }

    #[test]
    fn modal_renders_scrolled_long_message() {
        use crate::client::BulletinSnapshot;
        use crate::view::bulletins::state::BulletinsState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;

        let long = (0..30)
            .map(|i| format!("line {i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut state = BulletinsState::with_capacity(10);
        state.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: long,
            source_id: "s".into(),
            source_name: "S".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-20T10:14:22Z".into(),
            timestamp_human: String::new(),
        });
        state.selected = 0;
        state.auto_scroll = false;
        state.open_detail_modal();
        state.detail_modal.as_mut().unwrap().scroll_offset = 5;

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 60, 15), &mut state);
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_long_message_scrolled", terminal.backend().buffer());
    }
}
