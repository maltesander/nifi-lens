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
use crate::widget::search::MatchSpan;
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

    let show_search_strip = modal
        .search
        .as_ref()
        .map(|s| s.input_active)
        .unwrap_or(false);

    let footer_rows = if show_search_strip {
        FOOTER_ROWS + 1
    } else {
        FOOTER_ROWS
    };

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(HEADER_ROWS),
            Constraint::Fill(1),
            Constraint::Length(footer_rows),
        ])
        .split(inner);

    render_header(frame, rows[0], modal);
    render_body(frame, rows[1], modal);
    render_footer(frame, rows[2], modal);

    modal.scroll.last_viewport_rows = rows[1].height as usize;
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
    let search = modal.search.clone();

    // Build styled lines in pre-wrap coordinates.
    let mut styled: Vec<Line<'static>> = Vec::new();
    for (line_idx, line) in body.split('\n').enumerate() {
        let line_owned = line.to_string();
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut cursor = 0usize;

        if let Some(s) = search.as_ref() {
            // Find match indices that fall on this pre-wrap line.
            let per_line: Vec<(usize, &MatchSpan)> = s
                .matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line_idx == line_idx)
                .collect();

            for (global_idx, m) in per_line {
                if m.byte_start > cursor {
                    spans.push(Span::raw(line_owned[cursor..m.byte_start].to_string()));
                }
                let hit = line_owned[m.byte_start..m.byte_end].to_string();
                let mut style = ratatui::style::Style::default()
                    .add_modifier(ratatui::style::Modifier::UNDERLINED);
                if s.current == Some(global_idx) {
                    style = style.add_modifier(
                        ratatui::style::Modifier::REVERSED | ratatui::style::Modifier::BOLD,
                    );
                }
                spans.push(Span::styled(hit, style));
                cursor = m.byte_end;
            }
        }

        if cursor < line_owned.len() {
            spans.push(Span::raw(line_owned[cursor..].to_string()));
        }
        if spans.is_empty() {
            spans.push(Span::raw(""));
        }
        styled.push(Line::from(spans));
    }

    // Auto-scroll so the current match's line is visible.
    if let Some(s) = search.as_ref()
        && let Some(idx) = s.current
        && let Some(m) = s.matches.get(idx)
    {
        let target = m.line_idx;
        let rows = area.height as usize;
        if target < modal.scroll.offset {
            modal.scroll.offset = target;
        } else if rows > 0 && target >= modal.scroll.offset + rows {
            modal.scroll.offset = target + 1 - rows;
        }
    }

    // Clamp scroll offset against estimated wrapped rows.
    let estimated_rows = estimate_wrapped_rows(&body, area.width as usize);
    let max_offset = estimated_rows.saturating_sub(area.height as usize);
    if modal.scroll.offset > max_offset {
        modal.scroll.offset = max_offset;
    }

    frame.render_widget(
        Paragraph::new(styled)
            .wrap(Wrap { trim: false })
            .scroll((modal.scroll.offset as u16, 0)),
        area,
    );
}

fn render_footer(frame: &mut Frame, area: Rect, modal: &DetailModalState) {
    let mut lines: Vec<Line<'static>> = Vec::new();

    // Search input strip (above the hint line) when typing.
    if let Some(s) = modal.search.as_ref()
        && s.input_active
    {
        lines.push(Line::from(vec![
            Span::styled("/ ".to_string(), theme::accent()),
            Span::raw(s.query.clone()),
            Span::styled(
                "_".to_string(),
                ratatui::style::Style::default().add_modifier(ratatui::style::Modifier::REVERSED),
            ),
        ]));
    }

    let mut hint = String::from("↑↓ scroll · PgUp/PgDn page · c copy · / find");
    if let Some(s) = modal.search.as_ref()
        && s.committed
    {
        hint.push_str(" · n next · N prev");
    }
    hint.push_str(" · Esc close");

    // Blank separator row above the hint line. When the search strip is
    // showing, the separator appears between the strip and the hint for
    // readability.
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(hint, theme::muted())));

    frame.render_widget(Paragraph::new(lines), area);
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
        state.detail_modal.as_mut().unwrap().scroll.offset = 5;

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 60, 15), &mut state);
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_long_message_scrolled", terminal.backend().buffer());
    }

    #[test]
    fn modal_renders_search_highlights() {
        use crate::client::BulletinSnapshot;
        use crate::view::bulletins::state::BulletinsState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;

        let mut state = BulletinsState::with_capacity(10);
        state.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "connection refused\nretry connection\nconnection closed".into(),
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
        state.modal_search_open();
        for c in "connection".chars() {
            state.modal_search_push(c);
        }
        state.modal_search_commit();
        state.modal_search_cycle_next(); // current = 1

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 60, 15), &mut state);
            })
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_search_highlights_current_is_1",
            terminal.backend().buffer()
        );
    }

    #[test]
    fn modal_renders_search_input_strip() {
        use crate::client::BulletinSnapshot;
        use crate::view::bulletins::state::BulletinsState;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::layout::Rect;

        let mut state = BulletinsState::with_capacity(10);
        state.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "INFO".into(),
            message: "something happens".into(),
            source_id: "s".into(),
            source_name: "S".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-20T10:00:00Z".into(),
            timestamp_human: String::new(),
        });
        state.selected = 0;
        state.auto_scroll = false;
        state.open_detail_modal();
        state.modal_search_open();
        for c in "some".chars() {
            state.modal_search_push(c);
        }

        let backend = TestBackend::new(60, 15);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| {
                render(frame, Rect::new(0, 0, 60, 15), &mut state);
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_search_input_strip", terminal.backend().buffer());
    }
}
