//! Render the Tracer content viewer modal.
//!
//! Full-screen overlay. Header with event identity, sizes, mime pair,
//! and diff eligibility. Tab strip (Input / Output / Diff) with
//! grayed Diff when ineligible. Body is the scrollable region —
//! Input/Output show streamed text or hex; Diff shows a colored
//! unified diff. Footer strip carries the modal-scoped hint line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::theme;
use crate::view::tracer::state::{ContentModalState, ContentModalTab, Diffable, NotDiffableReason};

pub fn render(frame: &mut Frame, area: Rect, modal: &mut ContentModalState) {
    frame.render_widget(Clear, area);

    let title = format!(
        " Content · {} · event {} · {} ",
        modal.header.component_name, modal.event_id, modal.header.event_type,
    );
    let block = Block::bordered()
        .border_type(BorderType::Thick)
        .title(title.as_str());
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // header
            Constraint::Length(1), // blank
            Constraint::Length(1), // tab strip
            Constraint::Length(1), // divider
            Constraint::Fill(1),   // body
            Constraint::Length(1), // divider
            Constraint::Length(1), // stream status
            Constraint::Length(1), // search strip (reserved; may be empty)
            Constraint::Length(1), // footer hint
        ])
        .split(inner);

    render_header(frame, rows[0], modal);
    render_tab_strip(frame, rows[2], modal);
    render_body(frame, rows[4], modal);
    render_stream_status(frame, rows[6], modal);
    render_search_strip(frame, rows[7], modal);
    render_footer_hint(frame, rows[8], modal);
    modal.last_viewport_rows = rows[4].height as usize;
    // Body cols = area width minus the fixed gutter. The renderer
    // already computed the gutter size above; approximate here as the
    // total width minus a small default when the renderer hasn't run
    // through text yet. The first frame is always generous since the
    // live modal starts with at least one chunk requested.
    modal.last_viewport_body_cols = rows[4].width as usize;
}

fn render_header(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    let h = &modal.header;
    let at = format!(
        "at    {}   pg {}",
        short_iso(&h.event_timestamp_iso),
        h.pg_path,
    );
    let mut sizes = format!(
        "sizes in {} → out {}  ({})",
        size_ui(h.input_size),
        size_ui(h.output_size),
        delta_ui(h.input_size, h.output_size),
    );
    if let Some(cache) = modal.diff_cache.as_ref() {
        let n = cache.change_stops.len();
        if n > 0 {
            sizes.push_str(&format!(
                "  ·  {n} {}",
                if n == 1 { "change" } else { "changes" },
            ));
        }
    }
    let mime = format!(
        "mime  in {} · out {}  {}",
        h.input_mime.clone().unwrap_or_else(|| "—".into()),
        h.output_mime.clone().unwrap_or_else(|| "—".into()),
        diffable_verdict(&modal.diffable),
    );
    let lines = vec![
        Line::from(Span::styled(at, theme::muted())),
        Line::from(Span::styled(sizes, theme::muted())),
        Line::from(Span::styled(mime, theme::muted())),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn render_tab_strip(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    let diff_enabled = matches!(modal.diffable, Diffable::Ok);
    let diff_label = match &modal.diffable {
        Diffable::Ok | Diffable::Pending => "Diff".to_string(),
        Diffable::NotAvailable(reason) => format!("Diff · {}", reason_chip(*reason)),
    };
    let spans: Vec<Span<'static>> = vec![
        Span::raw("  "),
        tab_span("Input", modal.active_tab == ContentModalTab::Input, true),
        Span::raw("   "),
        tab_span("Output", modal.active_tab == ContentModalTab::Output, true),
        Span::raw("   "),
        tab_span(
            &diff_label,
            modal.active_tab == ContentModalTab::Diff,
            diff_enabled,
        ),
    ];
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn render_body(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    // Build the rendered lines once (gutter spans first, then body
    // spans). For horizontal scroll we need the gutter anchored and
    // only the body shifted sideways — so split each line's spans
    // into two Lines, then render gutter and body as two paragraphs
    // in two adjacent areas. The body uses `Paragraph::scroll((0, x))`
    // which clips the left `x` display columns.
    let lines: Vec<Line<'static>> = match modal.active_tab {
        ContentModalTab::Input => text_body_lines(
            &modal.input.decoded,
            modal.scroll_offset,
            area.height as usize,
            modal.search.as_ref(),
        ),
        ContentModalTab::Output => text_body_lines(
            &modal.output.decoded,
            modal.scroll_offset,
            area.height as usize,
            modal.search.as_ref(),
        ),
        ContentModalTab::Diff => {
            diff_body_lines(modal, area.height as usize, modal.search.as_ref())
        }
    };

    let gutter_columns = match modal.active_tab {
        ContentModalTab::Diff => 2,
        _ => 1,
    };
    let gutter_width = total_gutter_width(&lines, gutter_columns);

    let mut gutter_lines: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    let mut body_lines: Vec<Line<'static>> = Vec::with_capacity(lines.len());
    for line in lines {
        let mut spans = line.spans;
        let take = gutter_columns.min(spans.len());
        let body_spans: Vec<Span<'static>> = spans.drain(take..).collect();
        gutter_lines.push(Line::from(spans));
        body_lines.push(Line::from(body_spans));
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(gutter_width as u16), Constraint::Fill(1)])
        .split(area);

    frame.render_widget(Paragraph::new(gutter_lines), cols[0]);
    frame.render_widget(
        Paragraph::new(body_lines).scroll((0, modal.horizontal_scroll_offset as u16)),
        cols[1],
    );
}

/// Sum of display widths of the first `gutter_columns` spans across
/// the rendered lines. Gutter spans are identical per row (all use
/// the same width), so the first line whose gutter is fully present
/// is authoritative.
fn total_gutter_width(lines: &[Line<'static>], gutter_columns: usize) -> usize {
    for line in lines {
        if line.spans.len() >= gutter_columns {
            return line
                .spans
                .iter()
                .take(gutter_columns)
                .map(|s| s.content.chars().count())
                .sum();
        }
    }
    0
}

fn text_body_lines(
    render: &crate::client::tracer::ContentRender,
    scroll: usize,
    height: usize,
    search: Option<&crate::widget::search::SearchState>,
) -> Vec<Line<'static>> {
    use crate::client::tracer::ContentRender;

    // Collect the visible window + the TOTAL line count so we can size
    // the line-number gutter.
    let (raw, total_lines): (Vec<(usize, String)>, usize) = match render {
        ContentRender::Empty => {
            return vec![Line::from(Span::styled("<empty>", theme::muted()))];
        }
        ContentRender::Text { text, .. } => {
            let total = text.lines().count();
            let window: Vec<(usize, String)> = text
                .lines()
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(abs_idx, l)| (abs_idx, l.to_string()))
                .collect();
            (window, total)
        }
        ContentRender::Hex { first_4k } => {
            let total = first_4k.lines().count();
            let window: Vec<(usize, String)> = first_4k
                .lines()
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(abs_idx, l)| (abs_idx, l.to_string()))
                .collect();
            (window, total)
        }
    };

    let gutter_width = line_number_width(total_lines);

    let search_active = search
        .map(|s| s.committed && !s.matches.is_empty())
        .unwrap_or(false);

    raw.into_iter()
        .map(|(abs_idx, line_text)| {
            let mut spans: Vec<Span<'static>> = vec![gutter_span(Some(abs_idx + 1), gutter_width)];

            if !search_active {
                spans.push(Span::raw(line_text));
                return Line::from(spans);
            }
            let s = search.expect("search_active implies search Some");
            let line_matches: Vec<(usize, usize, bool)> = s
                .matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line_idx == abs_idx)
                .map(|(mi, m)| {
                    let start = m.byte_start.min(line_text.len());
                    let end = m.byte_end.min(line_text.len());
                    (start, end, Some(mi) == s.current)
                })
                .collect();
            if line_matches.is_empty() {
                spans.push(Span::raw(line_text));
                return Line::from(spans);
            }
            let mut cursor = 0usize;
            for (start, end, is_current) in line_matches {
                if cursor < start {
                    spans.push(Span::raw(line_text[cursor..start].to_string()));
                }
                let style = if is_current {
                    theme::highlight()
                } else {
                    theme::bold()
                };
                spans.push(Span::styled(line_text[start..end].to_string(), style));
                cursor = end;
            }
            if cursor < line_text.len() {
                spans.push(Span::raw(line_text[cursor..].to_string()));
            }
            Line::from(spans)
        })
        .collect()
}

/// Width (in characters) of a right-aligned line-number column big
/// enough to hold `total_lines`. Minimum 3 so short content still
/// gets a tidy gutter.
fn line_number_width(total_lines: usize) -> usize {
    let digits = if total_lines == 0 {
        1
    } else {
        let mut n = total_lines;
        let mut d = 0usize;
        while n > 0 {
            n /= 10;
            d += 1;
        }
        d
    };
    digits.max(3)
}

/// Single-column line-number gutter: right-aligned number, separator
/// `│`, and surrounding spaces. `None` produces a blank gutter of the
/// same width for diff rows where one side doesn't apply. Always
/// rendered in `theme::muted()`.
fn gutter_span(n: Option<usize>, width: usize) -> Span<'static> {
    let text = match n {
        Some(v) => format!(" {:>width$} │ ", v, width = width),
        None => format!(" {:>width$} │ ", "", width = width),
    };
    Span::styled(text, theme::muted())
}

fn diff_body_lines(
    modal: &ContentModalState,
    height: usize,
    search: Option<&crate::widget::search::SearchState>,
) -> Vec<Line<'static>> {
    let Some(cache) = modal.diff_cache.as_ref() else {
        return vec![Line::from(Span::styled(
            match &modal.diffable {
                Diffable::Pending => "computing diff…".to_string(),
                Diffable::NotAvailable(r) => format!("diff unavailable: {}", reason_chip(*r)),
                Diffable::Ok => "no diff cached".to_string(),
            },
            theme::muted(),
        ))];
    };

    let mut rows: Vec<Line<'static>> = Vec::with_capacity(height);
    let hunks = &cache.hunks;
    let mut hunk_idx = 0usize;
    let mut next_hunk_line = hunks
        .first()
        .map(|h| h.line_idx as usize)
        .unwrap_or(usize::MAX);

    // Size the two-column gutter to the widest line number appearing
    // on either side. Walk the whole cache once so every rendered row
    // lines up regardless of scroll position.
    let max_input_line = cache
        .lines
        .iter()
        .filter_map(|dl| dl.input_line)
        .max()
        .unwrap_or(0) as usize;
    let max_output_line = cache
        .lines
        .iter()
        .filter_map(|dl| dl.output_line)
        .max()
        .unwrap_or(0) as usize;
    let in_width = line_number_width(max_input_line);
    let out_width = line_number_width(max_output_line);

    // Search corpus for the Diff tab is `cache.lines.join("\n")` (see
    // `content_modal_search_body`), so committed match `line_idx`
    // values map 1:1 to the iteration index into `cache.lines` below.
    let search_active = search
        .map(|s| s.committed && !s.matches.is_empty())
        .unwrap_or(false);

    for (idx, dl) in cache
        .lines
        .iter()
        .enumerate()
        .skip(modal.scroll_offset)
        .take(height)
    {
        if idx == next_hunk_line {
            let h = &hunks[hunk_idx];
            // Hunk header rows get a blank two-column gutter so the
            // `@@ … @@` text aligns with the body columns below.
            let header_spans: Vec<Span<'static>> = vec![
                gutter_span(None, in_width),
                gutter_span(None, out_width),
                Span::styled(
                    format!("@@ input L{} · output L{} @@", h.input_line, h.output_line),
                    theme::hunk_header(),
                ),
            ];
            rows.push(Line::from(header_spans));
            hunk_idx += 1;
            next_hunk_line = hunks
                .get(hunk_idx)
                .map(|h| h.line_idx as usize)
                .unwrap_or(usize::MAX);
            continue;
        }
        let (prefix, line_color) = match dl.tag {
            similar::ChangeTag::Insert => ("+ ", theme::diff_add()),
            similar::ChangeTag::Delete => ("- ", theme::diff_del()),
            similar::ChangeTag::Equal => ("  ", ratatui::style::Style::default()),
        };

        // Per-byte base styles for `dl.text`: if `inline_diff` segments
        // are present, unchanged bytes render muted and changed bytes
        // render in the row's `-`/`+` color; otherwise every byte gets
        // the row color (old behavior, appropriate for unpaired
        // Delete/Insert rows and Equal context).
        let base_styles: Vec<ratatui::style::Style> = match &dl.inline_diff {
            Some(segments) if !segments.is_empty() => {
                let mut v = vec![line_color; dl.text.len()];
                for seg in segments {
                    let end = seg.range.end.min(v.len());
                    let start = seg.range.start.min(end);
                    let style = if seg.differs {
                        line_color
                    } else {
                        theme::muted()
                    };
                    for byte_style in v.iter_mut().take(end).skip(start) {
                        *byte_style = style;
                    }
                }
                v
            }
            _ => vec![line_color; dl.text.len()],
        };

        // Collect committed search matches on this visible diff row.
        let line_matches: Vec<(usize, usize, bool)> = if search_active {
            let s = search.expect("search_active implies search Some");
            s.matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line_idx == idx)
                .map(|(mi, m)| {
                    let start = m.byte_start.min(dl.text.len());
                    let end = m.byte_end.min(dl.text.len());
                    (start, end, Some(mi) == s.current)
                })
                .collect()
        } else {
            Vec::new()
        };

        // Build the body spans (prefix + text), then prepend the two
        // line-number gutters so each row has `[in] [out] <body>`.
        let body_spans =
            build_diff_line_spans(prefix, line_color, &dl.text, &base_styles, &line_matches);
        let mut row_spans: Vec<Span<'static>> = Vec::with_capacity(body_spans.len() + 2);
        row_spans.push(gutter_span(dl.input_line.map(|n| n as usize), in_width));
        row_spans.push(gutter_span(dl.output_line.map(|n| n as usize), out_width));
        row_spans.extend(body_spans);
        rows.push(Line::from(row_spans));
    }
    rows
}

/// Build the ratatui spans for one diff body row. Walks `text` byte
/// by byte, emitting a new span whenever the effective style changes
/// (i.e. at inline-diff segment boundaries or search-match boundaries).
/// This centralizes the inline-diff + search-overlay interaction so
/// neither feature has to special-case the other.
fn build_diff_line_spans(
    prefix: &'static str,
    prefix_style: ratatui::style::Style,
    text: &str,
    base_styles: &[ratatui::style::Style],
    matches: &[(usize, usize, bool)],
) -> Vec<Span<'static>> {
    // Fast path: single color across the whole text and no matches →
    // one body span (two total including the prefix).
    let uniform_base = base_styles.is_empty() || base_styles.iter().all(|s| *s == base_styles[0]);
    if matches.is_empty() && uniform_base {
        let body_style = base_styles.first().copied().unwrap_or(prefix_style);
        return vec![
            Span::styled(prefix, prefix_style),
            Span::styled(text.to_string(), body_style),
        ];
    }

    // Walk the text, emitting a new span at every byte boundary where
    // the effective style changes. `effective_style(i)` composes the
    // per-byte base style with any search-match overlay at byte `i`.
    let effective_style = |i: usize| -> ratatui::style::Style {
        let base = base_styles.get(i).copied().unwrap_or(prefix_style);
        // Highest-priority overlay wins: current match > other match
        // > base.
        for (start, end, is_current) in matches {
            if *start <= i && i < *end {
                return if *is_current {
                    base.patch(theme::highlight())
                } else {
                    base.patch(theme::bold())
                };
            }
        }
        base
    };

    let mut spans: Vec<Span<'static>> = vec![Span::styled(prefix, prefix_style)];
    if text.is_empty() {
        return spans;
    }

    let mut run_start = 0usize;
    let mut run_style = effective_style(0);
    for i in 1..text.len() {
        // Only consider char boundaries so UTF-8 chars never split
        // mid-byte. For ASCII content (the common case) every index
        // is a boundary.
        if !text.is_char_boundary(i) {
            continue;
        }
        let style = effective_style(i);
        if style != run_style {
            spans.push(Span::styled(text[run_start..i].to_string(), run_style));
            run_start = i;
            run_style = style;
        }
    }
    if run_start < text.len() {
        spans.push(Span::styled(text[run_start..].to_string(), run_style));
    }
    spans
}

fn render_stream_status(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    let buf = match modal.active_tab {
        ContentModalTab::Input => Some(&modal.input),
        ContentModalTab::Output => Some(&modal.output),
        ContentModalTab::Diff => None,
    };
    let text = match buf {
        None => String::new(),
        Some(b) if b.last_error.is_some() => format!(
            "loaded {} · fetch failed: {}",
            bytes_ui(b.loaded.len()),
            b.last_error.clone().unwrap_or_default(),
        ),
        Some(b) if b.ceiling_hit => format!(
            "loaded {} · ceiling reached — press 's' to save full content",
            bytes_ui(b.loaded.len()),
        ),
        Some(b) if b.fully_loaded => format!("loaded {} (complete)", bytes_ui(b.loaded.len())),
        Some(b) if b.in_flight => format!("loaded {} · fetching…", bytes_ui(b.loaded.len())),
        Some(b) => format!("loaded {}", bytes_ui(b.loaded.len())),
    };
    let style = match buf {
        Some(b) if b.last_error.is_some() => theme::error(),
        _ => theme::muted(),
    };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}

fn bytes_ui(n: usize) -> String {
    if n >= 1024 * 1024 {
        format!("{:.1} MiB", n as f64 / 1_048_576.0)
    } else if n >= 1024 {
        format!("{:.1} KiB", n as f64 / 1024.0)
    } else {
        format!("{} B", n)
    }
}

fn tab_span(label: &str, active: bool, enabled: bool) -> Span<'static> {
    let style = if !enabled {
        theme::muted()
    } else if active {
        theme::accent()
    } else {
        theme::bold()
    };
    Span::styled(format!(" {} ", label), style)
}

fn reason_chip(r: NotDiffableReason) -> &'static str {
    match r {
        NotDiffableReason::InputUnavailable => "input unavailable",
        NotDiffableReason::OutputUnavailable => "output unavailable",
        NotDiffableReason::MimeMismatch => "mime mismatch",
        NotDiffableReason::SizeExceedsDiffCap => "size > 512 KiB",
        NotDiffableReason::NoDifferences => "no differences",
    }
}

fn diffable_verdict(d: &Diffable) -> String {
    match d {
        Diffable::Ok => "✓ diffable".to_string(),
        Diffable::Pending => "⋯".to_string(),
        Diffable::NotAvailable(r) => format!("⊘ {}", reason_chip(*r)),
    }
}

fn short_iso(iso: &str) -> String {
    iso.trim_end_matches('Z').to_string()
}

fn size_ui(n: Option<u64>) -> String {
    match n {
        Some(n) if n >= 1024 * 1024 => format!("{:.1} MiB", n as f64 / 1_048_576.0),
        Some(n) if n >= 1024 => format!("{:.1} KiB", n as f64 / 1024.0),
        Some(n) => format!("{} B", n),
        None => "—".into(),
    }
}

fn delta_ui(i: Option<u64>, o: Option<u64>) -> String {
    match (i, o) {
        (Some(i), Some(o)) => {
            let d = o as i128 - i as i128;
            let sign = if d >= 0 { "+" } else { "-" };
            let abs = d.unsigned_abs();
            if abs >= 1024 {
                format!("{}{:.1} KiB", sign, abs as f64 / 1024.0)
            } else {
                format!("{}{} B", sign, abs)
            }
        }
        _ => "—".into(),
    }
}

fn render_search_strip(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    let Some(s) = modal.search.as_ref() else {
        return;
    };
    if !s.input_active && !s.committed {
        return;
    }
    let match_info = match s.matches.len() {
        0 if s.committed => " (0 matches)".to_string(),
        0 => String::new(),
        n => format!(" ({}/{})", s.current.map(|i| i + 1).unwrap_or(0), n),
    };
    let line = Line::from(vec![
        Span::styled("/", theme::accent()),
        Span::raw(" "),
        Span::raw(s.query.clone()),
        Span::styled(if s.input_active { "_" } else { "" }, theme::accent()),
        Span::styled(match_info, theme::muted()),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Build the footer hint by iterating `ContentModalVerb::all()`, filtering to
/// verbs that are both `show_in_hint_bar()` and enabled given the current modal
/// state. This mirrors what the top-level hint_bar does for outer tabs, but uses
/// a local enabled predicate instead of `HintContext` (which requires `&AppState`
/// and cannot be constructed here due to split borrow constraints).
fn render_footer_hint(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    use crate::input::Verb;
    use crate::input::verb::ContentModalVerb;

    // Inline enabled check mirroring ContentModalVerb::enabled() but
    // operating on the modal reference directly (no AppState needed).
    let enabled = |v: ContentModalVerb| -> bool {
        match v {
            ContentModalVerb::JumpDiff => {
                matches!(modal.diffable, crate::view::tracer::state::Diffable::Ok)
            }
            ContentModalVerb::SearchNext | ContentModalVerb::SearchPrev => {
                modal.search.as_ref().map(|s| s.committed).unwrap_or(false)
            }
            ContentModalVerb::HunkNext | ContentModalVerb::HunkPrev => {
                modal.active_tab == crate::view::tracer::state::ContentModalTab::Diff
                    && modal
                        .diff_cache
                        .as_ref()
                        .map(|d| !d.hunks.is_empty())
                        .unwrap_or(false)
            }
            _ => true,
        }
    };

    let parts: Vec<String> = ContentModalVerb::all()
        .iter()
        .copied()
        .filter(|v| v.show_in_hint_bar() && !v.hint().is_empty() && enabled(*v))
        .map(|v| format!("[{}] {}", v.chord().display(), v.hint()))
        .collect();

    let text = parts.join(" · ");
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(text, theme::muted()))),
        area,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::tracer::ContentRender;
    use crate::view::tracer::state::{
        ContentModalHeader, ContentModalState, ContentModalTab, DiffLine, DiffRender, Diffable,
        HunkAnchor, NotDiffableReason, SideBuffer,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn stub_header() -> ContentModalHeader {
        ContentModalHeader {
            event_type: "DROP".into(),
            event_timestamp_iso: "2026-04-22T13:42:18.231".into(),
            component_name: "UpdateAttribute-enrich".into(),
            pg_path: "healthy-pipeline/enrich".into(),
            input_size: Some(2_400),
            output_size: Some(2_800),
            input_mime: Some("application/json".into()),
            output_mime: Some("application/json".into()),
            input_available: true,
            output_available: true,
        }
    }

    fn stub_modal(active: ContentModalTab) -> ContentModalState {
        ContentModalState {
            event_id: 8_471_293,
            header: stub_header(),
            active_tab: active,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Ok,
            input: SideBuffer {
                loaded: br#"{"orderId":"A-1029","total":299.00}"#.to_vec(),
                decoded: ContentRender::Text {
                    text: "{\"orderId\":\"A-1029\",\n  \"total\":299.00}\n".into(),
                    pretty_printed: true,
                },
                fully_loaded: true,
                ceiling_hit: false,
                in_flight: false,
                last_error: None,
            },
            output: SideBuffer {
                loaded: br#"{"orderId":"A-1029","total":329.99,"tax":30.99}"#.to_vec(),
                decoded: ContentRender::Text {
                    text: "{\"orderId\":\"A-1029\",\n  \"total\":329.99,\n  \"tax\":30.99}\n"
                        .into(),
                    pretty_printed: true,
                },
                fully_loaded: true,
                ceiling_hit: false,
                in_flight: false,
                last_error: None,
            },
            diff_cache: None,
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            last_viewport_rows: 0,
            last_viewport_body_cols: 0,
            search: None,
        }
    }

    #[test]
    fn modal_input_complete() {
        let mut modal = stub_modal(ContentModalTab::Input);
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_input_complete", terminal.backend().buffer());
    }

    #[test]
    fn modal_output_empty() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.output.loaded.clear();
        modal.output.decoded = ContentRender::Empty;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_output_empty", terminal.backend().buffer());
    }

    #[test]
    fn modal_input_ceiling_hit() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.loaded = vec![b'x'; 4 * 1024 * 1024];
        modal.input.ceiling_hit = true;
        modal.input.fully_loaded = true;
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_input_ceiling_hit", terminal.backend().buffer());
    }

    #[test]
    fn modal_diff_unified_three_hunks() {
        let mut modal = stub_modal(ContentModalTab::Diff);
        modal.diffable = Diffable::Ok;
        modal.diff_cache = Some(DiffRender {
            lines: vec![
                DiffLine {
                    tag: similar::ChangeTag::Delete,
                    text: "\"total\":299.00".into(),
                    input_line: Some(3),
                    output_line: None,
                    inline_diff: None,
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"total\":329.99,".into(),
                    input_line: None,
                    output_line: Some(3),
                    inline_diff: None,
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"tax\":30.99".into(),
                    input_line: None,
                    output_line: Some(4),
                    inline_diff: None,
                },
            ],
            hunks: vec![HunkAnchor {
                line_idx: 0,
                input_line: 3,
                output_line: 3,
            }],
            change_stops: vec![0, 1],
        });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!(
            "modal_diff_unified_three_hunks",
            terminal.backend().buffer()
        );
    }

    #[test]
    fn modal_diff_no_differences() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::NoDifferences);
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_diff_no_differences", terminal.backend().buffer());
    }

    #[test]
    fn modal_diff_size_exceeds_cap() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap);
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_diff_size_exceeds_cap", terminal.backend().buffer());
    }

    #[test]
    fn modal_search_with_matches() {
        let mut modal = stub_modal(ContentModalTab::Input);
        // Input text line 1 is `  "total":299.00}`.
        // "total" starts at byte 3 (after `  "`).
        modal.search = Some(crate::widget::search::SearchState {
            query: "total".to_string(),
            input_active: false,
            committed: true,
            matches: vec![crate::widget::search::MatchSpan {
                line_idx: 1,
                byte_start: 3,
                byte_end: 8,
            }],
            current: Some(0),
        });
        let backend = TestBackend::new(100, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal.draw(|f| render(f, f.area(), &mut modal)).unwrap();
        insta::assert_debug_snapshot!("modal_search_with_matches", terminal.backend().buffer());
    }
}
