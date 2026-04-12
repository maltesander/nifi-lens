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

/// Left-to-right fill gauge using block-eighths (`▁▂▃▄▅▆▇█`).
///
/// Each of the `width` cells represents `1/width` of the `[0, max]`
/// range. Cells left of the current fill point render as `█`, the
/// rightmost partial cell picks one of the eight `▁..█` levels, and
/// cells beyond the fill render as `░`. Values are clamped to
/// `[0, max]`. A zero `width` returns an empty string.
///
/// Example at `width = 4`, `max = 1.0`:
/// - `0.00`  -> `░░░░`
/// - `0.25`  -> `█░░░`
/// - `0.50`  -> `██░░`
/// - `0.60`  -> `██▄░`
/// - `1.00`  -> `████`
/// - `1.50`  -> `████` (clamped)
pub fn spark_bar(value: f32, max: f32, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if max <= 0.0 || !max.is_finite() || !value.is_finite() {
        return "\u{2591}".repeat(width);
    }

    let ratio = (value / max).clamp(0.0, 1.0);
    // Total eighths available across `width` cells.
    let total_eighths = (ratio * (width as f32) * 8.0).round() as usize;

    let full_cells = total_eighths / 8;
    let remainder = total_eighths % 8;

    let mut out = String::with_capacity(width * 3);
    for _ in 0..full_cells {
        out.push('\u{2588}');
    }
    let mut used = full_cells;
    if remainder > 0 && used < width {
        // Block-eighths map, index 1..=7 -> ▁..▇
        let partial = match remainder {
            1 => '\u{2581}',
            2 => '\u{2582}',
            3 => '\u{2583}',
            4 => '\u{2584}',
            5 => '\u{2585}',
            6 => '\u{2586}',
            7 => '\u{2587}',
            _ => unreachable!(),
        };
        out.push(partial);
        used += 1;
    }
    while used < width {
        out.push('\u{2591}');
        used += 1;
    }
    out
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

    #[test]
    fn spark_bar_zero_value() {
        assert_eq!(spark_bar(0.0, 1.0, 4), "░░░░");
    }

    #[test]
    fn spark_bar_max_value() {
        assert_eq!(spark_bar(1.0, 1.0, 4), "████");
    }

    #[test]
    fn spark_bar_half() {
        assert_eq!(spark_bar(0.5, 1.0, 4), "██░░");
    }

    #[test]
    fn spark_bar_quarter() {
        assert_eq!(spark_bar(0.25, 1.0, 4), "█░░░");
    }

    #[test]
    fn spark_bar_clamps_over_max() {
        assert_eq!(spark_bar(1.5, 1.0, 4), "████");
    }

    #[test]
    fn spark_bar_zero_width() {
        assert_eq!(spark_bar(0.5, 1.0, 0), "");
    }

    #[test]
    fn spark_bar_handles_partial_cell() {
        // 0.3 of 1.0 across width 4 = 1.2 cells = 1 full + 2 eighths (▂)
        assert_eq!(spark_bar(0.3, 1.0, 4), "█▂░░");
    }

    #[test]
    fn spark_bar_zero_max_is_empty() {
        assert_eq!(spark_bar(5.0, 0.0, 4), "░░░░");
    }
}
