//! Goto-menu modal — context-sensitive cross-tab goto target picker.
//!
//! Opened by `AppAction::Goto` when more than one cross-link target
//! is available for the current selection. Auto-gotos without showing
//! this modal when only one target exists.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::input::GoTarget;
use crate::theme;

/// Subject of a goto — what the jump is "bound to". Displayed in the
/// popup's title bar. For a component jump (Browser / Events targets)
/// this carries a speaking name + uuid. For a flowfile jump
/// (Events → Tracer) only the uuid exists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GotoSubject {
    Component { name: String, id: String },
    Flowfile { uuid: String },
}

impl GotoSubject {
    /// Safety net for code paths that cannot build a real subject. The
    /// `AppAction::Goto` builder never reaches this in practice — it
    /// is here so the menu can still render rather than `unwrap()`.
    pub fn unknown() -> Self {
        GotoSubject::Component {
            name: "?".into(),
            id: String::new(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct GotoMenuState {
    pub targets: Vec<GoTarget>,
    /// Aligned 1:1 with `targets` — `subjects[i]` is the subject the
    /// jump would carry if the user picks `targets[i]`.
    pub subjects: Vec<GotoSubject>,
    pub selected: usize,
}

impl GotoMenuState {
    pub fn new(targets: Vec<GoTarget>, subjects: Vec<GotoSubject>) -> Self {
        debug_assert_eq!(
            targets.len(),
            subjects.len(),
            "goto menu: targets and subjects must be aligned",
        );
        Self {
            targets,
            subjects,
            selected: 0,
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn move_down(&mut self) {
        if self.selected + 1 < self.targets.len() {
            self.selected += 1;
        }
    }

    pub fn selected_target(&self) -> Option<GoTarget> {
        self.targets.get(self.selected).copied()
    }

    pub fn selected_subject(&self) -> Option<&GotoSubject> {
        self.subjects.get(self.selected)
    }
}

fn target_label(t: GoTarget) -> &'static str {
    match t {
        GoTarget::Browser => "Browser — open in flow tree",
        GoTarget::Events => "Events  — show events for component",
        GoTarget::Tracer => "Tracer  — trace flowfile",
    }
}

/// Render the goto menu as a small centered overlay.
pub fn render(frame: &mut Frame, area: Rect, state: &GotoMenuState) {
    let width: u16 = 44;
    let height: u16 = state.targets.len() as u16 + 2; // border rows

    let x = area.x + area.width.saturating_sub(width) / 2;
    let y = area.y + area.height.saturating_sub(height) / 2;
    let popup = Rect {
        x,
        y,
        width: width.min(area.width),
        height: height.min(area.height),
    };

    frame.render_widget(Clear, popup);

    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(theme::accent())
        .title(" Go to ");

    let inner = block.inner(popup);
    frame.render_widget(block, popup);

    let lines: Vec<Line> = state
        .targets
        .iter()
        .enumerate()
        .map(|(i, &t)| {
            let label = target_label(t);
            if i == state.selected {
                Line::from(Span::styled(format!(" \u{25b6} {label}"), theme::accent()))
            } else {
                Line::from(Span::styled(format!("   {label}"), theme::muted()))
            }
        })
        .collect();

    frame.render_widget(Paragraph::new(lines), inner);
}

/// Render a `GotoSubject` as a single line of at most `max_width`
/// characters. Components render as `Name (xxxxxxxx)` (8-char id
/// prefix); missing-name components fall back to the id prefix alone.
/// Flowfiles render as `flowfile <uuid>`, middle-truncated with `…`
/// if the full uuid would exceed the budget.
// Task 3 wires this into the render path; tests cover it already.
#[allow(dead_code)]
fn format_subject(subject: &GotoSubject, max_width: usize) -> String {
    match subject {
        GotoSubject::Component { name, id } => {
            let id_short: String = id.chars().take(8).collect();
            let rendered = if name.is_empty() {
                id_short
            } else {
                format!("{name} ({id_short})")
            };
            middle_truncate(&rendered, max_width)
        }
        GotoSubject::Flowfile { uuid } => {
            let full = format!("flowfile {uuid}");
            middle_truncate(&full, max_width)
        }
    }
}

/// Shorten `s` to at most `max` chars by replacing the middle with `…`.
/// Returns `s` unchanged when already short enough. For `max == 0`
/// returns the empty string; for `max == 1` returns the ellipsis alone.
// Task 3 wires this into the render path; tests cover it already.
#[allow(dead_code)]
fn middle_truncate(s: &str, max: usize) -> String {
    let total = s.chars().count();
    if total <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    if max <= 1 {
        return "…".into();
    }
    // Keep `left` chars from the head, `right` from the tail, plus `…`.
    let keep = max - 1;
    let left = keep.div_ceil(2);
    let right = keep - left;
    let head: String = s.chars().take(left).collect();
    let tail: String = s.chars().skip(total - right).collect();
    format!("{head}…{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::input::GoTarget;

    #[test]
    fn move_down_clamps_at_last() {
        let mut s = GotoMenuState::new(
            vec![GoTarget::Browser, GoTarget::Events],
            vec![GotoSubject::unknown(), GotoSubject::unknown()],
        );
        s.move_down();
        assert_eq!(s.selected, 1);
        s.move_down();
        assert_eq!(s.selected, 1, "should not exceed last");
    }

    #[test]
    fn move_up_clamps_at_zero() {
        let mut s = GotoMenuState::new(
            vec![GoTarget::Browser, GoTarget::Tracer],
            vec![GotoSubject::unknown(), GotoSubject::unknown()],
        );
        s.move_up();
        assert_eq!(s.selected, 0, "should not underflow");
    }

    #[test]
    fn selected_target_returns_correct_variant() {
        let s = GotoMenuState::new(
            vec![GoTarget::Events, GoTarget::Browser],
            vec![GotoSubject::unknown(), GotoSubject::unknown()],
        );
        assert_eq!(s.selected_target(), Some(GoTarget::Events));
    }

    #[test]
    fn new_stores_targets_and_subjects_aligned() {
        let targets = vec![GoTarget::Browser, GoTarget::Tracer];
        let subjects = vec![
            GotoSubject::Component {
                name: "ProcA".into(),
                id: "abcd1234-0000-0000-0000-000000000000".into(),
            },
            GotoSubject::Flowfile {
                uuid: "ff-1".into(),
            },
        ];
        let s = GotoMenuState::new(targets.clone(), subjects.clone());
        assert_eq!(s.targets, targets);
        assert_eq!(s.subjects.len(), 2);
        assert_eq!(s.selected, 0);
    }

    #[test]
    fn format_subject_component_with_name() {
        let s = GotoSubject::Component {
            name: "ProcA".into(),
            id: "abcd1234-0000-0000-0000-000000000000".into(),
        };
        assert_eq!(format_subject(&s, 64), "ProcA (abcd1234)");
    }

    #[test]
    fn format_subject_component_missing_name_shows_id_prefix() {
        let s = GotoSubject::Component {
            name: String::new(),
            id: "abcd1234-0000-0000-0000-000000000000".into(),
        };
        assert_eq!(format_subject(&s, 64), "abcd1234");
    }

    #[test]
    fn format_subject_component_short_id_no_panic() {
        // Guard the id-slicing code against shorter-than-8-char ids.
        let s = GotoSubject::Component {
            name: String::new(),
            id: "abc".into(),
        };
        assert_eq!(format_subject(&s, 64), "abc");
    }

    #[test]
    fn format_subject_flowfile_short_enough_untruncated() {
        let s = GotoSubject::Flowfile {
            uuid: "ff-1".into(),
        };
        assert_eq!(format_subject(&s, 64), "flowfile ff-1");
    }

    #[test]
    fn format_subject_flowfile_middle_truncated_to_fit() {
        let s = GotoSubject::Flowfile {
            uuid: "abcd1234-0000-0000-0000-000000000000".into(),
        };
        let out = format_subject(&s, 20);
        assert!(
            out.chars().count() <= 20,
            "got {:?} (len {})",
            out,
            out.chars().count()
        );
        assert!(out.starts_with("flowfile "), "got {:?}", out);
        assert!(out.contains('…'), "got {:?}", out);
    }

    #[test]
    fn middle_truncate_noop_when_short() {
        assert_eq!(middle_truncate("abc", 10), "abc");
    }

    #[test]
    fn middle_truncate_shortens_with_ellipsis() {
        let out = middle_truncate("abcdefghij", 7);
        assert!(out.chars().count() <= 7);
        assert!(out.contains('…'));
    }
}
