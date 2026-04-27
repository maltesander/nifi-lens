//! Bar-and-gauge string builders shared across health, overview, and
//! future views. Pure render-time helpers: they return a ready-to-render
//! `String` and pull in no ratatui types.

/// A width-character filled bar using full/empty block glyphs.
///
/// `percent` is an integer percentage clamped to `[0, 100]`. Output
/// consists of `█` for filled cells and `░` for empty cells, exactly
/// `width` cells wide.
pub fn fill_bar(width: u16, percent: u32) -> String {
    let filled = (width as u32 * percent / 100).min(width as u32) as usize;
    let empty = width as usize - filled;
    format!("{}{}", "\u{2588}".repeat(filled), "\u{2591}".repeat(empty))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_bar_zero_percent() {
        assert_eq!(fill_bar(4, 0), "░░░░");
    }

    #[test]
    fn fill_bar_half() {
        assert_eq!(fill_bar(4, 50), "██░░");
    }

    #[test]
    fn fill_bar_full() {
        assert_eq!(fill_bar(4, 100), "████");
    }

    #[test]
    fn fill_bar_clamps_over_100() {
        assert_eq!(fill_bar(4, 150), "████");
    }
}
