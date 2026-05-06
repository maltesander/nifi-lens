//! Renders the Watch strip below the Events filter bar:
//! predicate input + status chip + stats line.
//!
//! Layout — two visual rows inside a bordered box:
//!
//! ```text
//! +- Watch ----------------------------- (status) ------+
//! | predicate <text>                                    |
//! | <ev/s> · <fill>/<cap> buf · last <ms>               |
//! +-----------------------------------------------------+
//! ```
//!
//! Below [`COLLAPSE_BELOW`] cols, the strip degrades to a single-line
//! summary: `watch: <predicate excerpt> · <status>`.
//!
//! Five status chips:
//!
//! - tailing — accent green
//! - paused — muted
//! - narrow required — warning
//! - waiting — muted
//! - failed — error (truncates the wrapped error to 40 chars)

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use crate::theme;
use crate::view::events::state::{EventsState, WatchSession, WatchStatus};

/// Below this width, render the collapsed single-line summary.
pub const COLLAPSE_BELOW: u16 = 80;

/// Maximum chars from the predicate input shown in the collapsed
/// single-line summary.
const COLLAPSED_PREDICATE_EXCERPT: usize = 20;

/// Maximum chars from a `Failed` status' error message rendered into
/// the chip itself (the full text is preserved on the session).
const FAILED_CHIP_ERROR_MAX: usize = 40;

/// Render the watch strip into `area`. No-op when `state` is not in
/// watch mode.
pub fn render(frame: &mut Frame, area: Rect, state: &EventsState) {
    let Some(watch) = state.watch() else {
        return;
    };
    if area.width < COLLAPSE_BELOW {
        render_collapsed(frame, area, watch);
    } else {
        render_full(frame, area, watch, state.predicate_input_focused());
    }
}

fn render_full(frame: &mut Frame, area: Rect, watch: &WatchSession, focused: bool) {
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Watch", theme::bold()),
        Span::raw(" "),
    ]);
    let right_title = Line::from(vec![
        Span::raw(" "),
        status_chip_span(&watch.status),
        Span::raw(" "),
    ])
    .right_aligned();

    let block = crate::widget::panel::Panel::new(title)
        .focused(focused)
        .into_block()
        .title_top(right_title);

    let inner = block.inner(area);
    frame.render_widget(block, area);

    let pred_label = if focused {
        Span::styled("> predicate ", theme::accent().add_modifier(Modifier::BOLD))
    } else {
        Span::styled("  predicate ", theme::bold())
    };
    let pred_value = if watch.predicate_input.is_empty() {
        Span::styled("(empty)", theme::muted())
    } else {
        Span::styled(watch.predicate_input.as_str(), Style::default())
    };

    let last = watch
        .stats
        .last_poll_latency
        .map(|d| format!("{} ms", d.as_millis()))
        .unwrap_or_else(|| "-".to_string());
    let mut stats = format!(
        "{:.1} ev/s · {}/{} buf · last {}",
        watch.stats.events_per_sec_ewma,
        watch.buffer.len(),
        watch.buffer_cap,
        last,
    );
    if watch.stats.trimmed_total > 0 {
        stats.push_str(&format!(" · trimmed {}", watch.stats.trimmed_total));
    }
    if watch.stats.detail_fetch_errors > 0 {
        stats.push_str(&format!(
            " · detail-errors {}",
            watch.stats.detail_fetch_errors
        ));
    }

    // The bottom line is normally the stats. When a parse error is
    // sticky on the session (set by `commit_predicate`), replace the
    // stats line with a focused error message — investigators care
    // far more about why their predicate didn't take than the ev/s
    // counter. The error stays visible until the next successful
    // commit, so the user can iterate on the predicate and see by
    // the disappearance of the chip whether their edit worked.
    let bottom_line = match &watch.last_parse_error {
        Some(err) => Line::from(vec![
            Span::styled("✖ ", theme::error()),
            Span::styled(
                format!("col {}: {}", err.column, err.message),
                theme::error(),
            ),
        ]),
        None => Line::from(Span::styled(stats, theme::muted())),
    };

    let body = Paragraph::new(vec![Line::from(vec![pred_label, pred_value]), bottom_line]);
    frame.render_widget(body, inner);
}

fn render_collapsed(frame: &mut Frame, area: Rect, watch: &WatchSession) {
    let pred_excerpt: String = watch
        .predicate_input
        .chars()
        .take(COLLAPSED_PREDICATE_EXCERPT)
        .collect();
    let line = Line::from(vec![
        Span::styled("watch: ", theme::muted()),
        Span::raw(pred_excerpt),
        Span::raw(" · "),
        status_chip_span(&watch.status),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

fn status_chip_span(status: &WatchStatus) -> Span<'static> {
    match status {
        WatchStatus::Tailing => Span::styled("● tailing", theme::success()),
        WatchStatus::Paused => Span::styled("⏸ paused", theme::muted()),
        WatchStatus::NarrowRequired => Span::styled("⚠ narrow required", theme::warning()),
        WatchStatus::Waiting => Span::styled("⌛ waiting", theme::muted()),
        WatchStatus::Failed { error, .. } => {
            let trunc: String = error.chars().take(FAILED_CHIP_ERROR_MAX).collect();
            Span::styled(format!("✖ failed: {trunc}"), theme::error())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{Predicate, ProvenanceQuery};
    use crate::test_support::{TEST_BACKEND_WIDTH, test_backend};
    use crate::view::events::state::{EventsState, WatchSession, WatchStats, WatchStatus};
    use ratatui::Terminal;
    use std::collections::VecDeque;
    use std::time::Duration;

    fn populated_state(status: WatchStatus, predicate_focus: bool) -> EventsState {
        let mut s = EventsState::new();
        s.enter_watch_mode(WatchSession {
            narrow: ProvenanceQuery::default(),
            predicate: Predicate::default(),
            predicate_input: "filename =~ /^invoice-/".into(),
            buffer: VecDeque::new(),
            buffer_cap: 2000,
            cursor: None,
            status,
            stats: WatchStats {
                events_per_sec_ewma: 12.5,
                last_poll_latency: Some(Duration::from_millis(250)),
                trimmed_total: 0,
                detail_fetch_errors: 0,
            },
            last_parse_error: None,
        });
        if predicate_focus {
            s.focus_predicate();
        }
        s
    }

    fn snapshot_render(state: &EventsState, height: u16) -> String {
        let backend = test_backend(height);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, height);
            render(f, area, state);
        })
        .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn watch_strip_tailing_focused() {
        let state = populated_state(WatchStatus::Tailing, true);
        insta::assert_snapshot!(snapshot_render(&state, 4));
    }

    #[test]
    fn watch_strip_paused_unfocused() {
        let state = populated_state(WatchStatus::Paused, false);
        insta::assert_snapshot!(snapshot_render(&state, 4));
    }

    #[test]
    fn watch_strip_failed_with_long_error() {
        let mut state = populated_state(WatchStatus::Tailing, false);
        if let Some(w) = state.watch_mut() {
            w.status = WatchStatus::Failed {
                error: "submit returned 502 Bad Gateway: upstream timeout xxx yyy zzz".into(),
                retry_in: Duration::from_secs(10),
            };
        }
        insta::assert_snapshot!(snapshot_render(&state, 4));
    }

    #[test]
    fn watch_strip_collapsed_below_min_width() {
        let state = populated_state(WatchStatus::Tailing, false);
        let backend = ratatui::backend::TestBackend::new(60, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let area = Rect::new(0, 0, 60, 1);
            render(f, area, &state);
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn watch_strip_with_trimmed_and_detail_errors() {
        let mut state = populated_state(WatchStatus::Tailing, false);
        if let Some(w) = state.watch_mut() {
            w.stats.trimmed_total = 42;
            w.stats.detail_fetch_errors = 3;
        }
        insta::assert_snapshot!(snapshot_render(&state, 4));
    }

    #[test]
    fn render_is_noop_in_oneshot_mode() {
        let state = EventsState::new();
        // Should not panic and should leave the buffer empty/unchanged.
        let backend = test_backend(4);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, 4);
            render(f, area, &state);
        })
        .unwrap();
    }

    #[test]
    fn collapse_threshold_constant_is_80() {
        assert_eq!(COLLAPSE_BELOW, 80);
    }
}
