//! Horizontal chip row shared by the Events and Bulletins filter
//! bars. Tab-specific second rows (status, hints, text-input
//! prompt) stay in their respective render modules — this widget
//! renders the chip row only.

use ratatui::prelude::*;

/// One chip in the filter row. Each chip is a single styled span.
/// The caller formats the full chip text (e.g., "D time" or "E 12")
/// and provides the style. Active-vs-inactive weighting is decided
/// by the caller; this struct just carries the finished text + style.
#[derive(Debug, Clone)]
pub struct FilterChip<'a> {
    pub text: &'a str,
    pub style: Style,
}

/// Assemble the chips into a single ratatui `Line` with `separator`
/// inserted between each pair. Callers render the line inside
/// whatever layout row they prefer.
pub fn build_chip_line<'a>(chips: &'a [FilterChip<'a>], separator: &'a str) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::with_capacity(chips.len().saturating_mul(2));
    for (i, chip) in chips.iter().enumerate() {
        if i > 0 {
            spans.push(Span::raw(separator));
        }
        spans.push(Span::styled(chip.text, chip.style));
    }
    Line::from(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_chip_line_empty() {
        let chips: Vec<FilterChip> = vec![];
        let line = build_chip_line(&chips, "   ");
        assert_eq!(line.spans.len(), 0);
    }

    #[test]
    fn build_chip_line_single_chip_has_no_separator() {
        let chips = vec![FilterChip {
            text: "E 12",
            style: Style::default(),
        }];
        let line = build_chip_line(&chips, "   ");
        assert_eq!(line.spans.len(), 1);
        assert_eq!(line.spans[0].content, "E 12");
    }

    #[test]
    fn build_chip_line_two_chips_separated_by_3_spaces() {
        let chips = vec![
            FilterChip {
                text: "D time",
                style: Style::default(),
            },
            FilterChip {
                text: "T type",
                style: Style::default(),
            },
        ];
        let line = build_chip_line(&chips, "   ");
        assert_eq!(line.spans.len(), 3);
        assert_eq!(line.spans[0].content, "D time");
        assert_eq!(line.spans[1].content, "   ");
        assert_eq!(line.spans[2].content, "T type");
    }

    #[test]
    fn build_chip_line_preserves_per_chip_styles() {
        let red = Style::default().fg(Color::Red);
        let green = Style::default().fg(Color::Green);
        let chips = vec![
            FilterChip {
                text: "E",
                style: red,
            },
            FilterChip {
                text: "W",
                style: green,
            },
        ];
        let line = build_chip_line(&chips, " ");
        assert_eq!(line.spans[0].style, red);
        assert_eq!(line.spans[2].style, green);
    }
}
