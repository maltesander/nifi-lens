//! Sticky footer hint bar showing context-sensitive keybindings.

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::theme;

/// A single key-action pair displayed in the hint bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HintSpan {
    /// The key or key combination (e.g. `"j/k"`, `"Enter"`).
    pub key: &'static str,
    /// The action description (e.g. `"nav"`, `"expand"`).
    pub action: &'static str,
}

/// Separator between hint spans: ` · ` (space, middle dot U+00B7, space).
const SEPARATOR: &str = " \u{00B7} ";

/// Render the hint bar into the given area.
///
/// Key portions use `theme::accent()`, action text uses `theme::muted()`,
/// separated by ` · ` in muted style. If the combined width exceeds
/// `area.width`, the bar is truncated from the right with `…` (U+2026).
/// Empty hints produce no output.
pub fn render(frame: &mut Frame, area: Rect, hints: &[HintSpan]) {
    if hints.is_empty() {
        return;
    }

    let spans = build_spans(hints);
    let total_width: usize = spans.iter().map(|s| s.content.width()).sum();
    let max_width = area.width as usize;

    let line = if total_width <= max_width {
        Line::from(spans)
    } else {
        truncate_spans(&spans, max_width)
    };

    frame.render_widget(Paragraph::new(line), area);
}

/// Build the full (untruncated) span list for the given hints.
fn build_spans(hints: &[HintSpan]) -> Vec<Span<'static>> {
    let mut spans = Vec::new();
    for (i, hint) in hints.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled(SEPARATOR, theme::muted()));
        }
        spans.push(Span::styled(hint.key, theme::accent()));
        spans.push(Span::styled(format!(" {}", hint.action), theme::muted()));
    }
    spans
}

/// Truncate spans so the total display width fits within `max_width`,
/// appending `…` (U+2026) at the end.
fn truncate_spans(spans: &[Span<'static>], max_width: usize) -> Line<'static> {
    let ellipsis = "\u{2026}";
    // The ellipsis takes 1 column in a fixed-width terminal.
    let budget = max_width.saturating_sub(1);
    let mut result: Vec<Span<'static>> = Vec::new();
    let mut used = 0;

    for span in spans {
        let w = span.content.width();
        if used + w <= budget {
            result.push(span.clone());
            used += w;
        } else {
            // Partial span: take what fits by display width.
            let remaining = budget - used;
            if remaining > 0 {
                let content = span.content.as_ref();
                let truncated = truncate_to_width(content, remaining);
                if !truncated.is_empty() {
                    result.push(Span::styled(truncated.to_owned(), span.style));
                }
            }
            break;
        }
    }

    result.push(Span::styled(ellipsis, theme::muted()));
    Line::from(result)
}

/// Truncate a string to at most `max_cols` display columns on a char boundary.
fn truncate_to_width(s: &str, max_cols: usize) -> &str {
    if s.width() <= max_cols {
        return s;
    }
    let mut cols = 0;
    for (i, c) in s.char_indices() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if cols + w > max_cols {
            return &s[..i];
        }
        cols += w;
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    use unicode_width::UnicodeWidthStr;

    /// Helper: collect span text content from a line.
    fn line_text(line: &Line<'_>) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    #[test]
    fn empty_hints_renders_blank() {
        let spans = build_spans(&[]);
        assert!(spans.is_empty());
    }

    #[test]
    fn single_hint_renders_key_and_action() {
        let hints = [HintSpan {
            key: "j/k",
            action: "nav",
        }];
        let spans = build_spans(&hints);
        let line = Line::from(spans);
        assert_eq!(line_text(&line), "j/k nav");
    }

    #[test]
    fn multiple_hints_separated_by_dot() {
        let hints = [
            HintSpan {
                key: "j/k",
                action: "nav",
            },
            HintSpan {
                key: "Enter",
                action: "expand",
            },
        ];
        let spans = build_spans(&hints);
        let line = Line::from(spans);
        assert_eq!(line_text(&line), "j/k nav \u{00B7} Enter expand");
    }

    #[test]
    fn truncation_when_too_wide() {
        let hints = [
            HintSpan {
                key: "j/k",
                action: "nav",
            },
            HintSpan {
                key: "Enter",
                action: "expand",
            },
        ];
        let spans = build_spans(&hints);
        let line = truncate_spans(&spans, 20);
        let text = line_text(&line);
        assert!(
            text.ends_with('\u{2026}'),
            "expected trailing ellipsis, got: {text:?}"
        );
        // The text (including the ellipsis) must fit in 20 display columns.
        assert!(
            text.width() <= 20,
            "expected at most 20 columns, got {}: {text:?}",
            text.width()
        );
    }
}
