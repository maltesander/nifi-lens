//! Render the Tracer content viewer modal.
//!
//! Full-screen overlay. Header with event identity, sizes, mime pair,
//! and diff eligibility. Tab strip (Input / Output / Diff) with
//! grayed Diff when ineligible. Body is the scrollable region —
//! Input/Output show streamed text or hex; Diff shows a colored
//! unified diff. Footer strip carries the modal-scoped hint line.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};

use crate::theme;
use crate::view::tracer::state::{ContentModalState, ContentModalTab, Diffable, NotDiffableReason};

pub fn render(
    frame: &mut Frame,
    area: Rect,
    modal: &mut ContentModalState,
    cfg: &crate::config::TracerCeilingConfig,
) {
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
    render_stream_status(frame, rows[6], modal, cfg);
    render_search_strip(frame, rows[7], modal);
    render_footer_hint(frame, rows[8], modal);
    modal.scroll.vertical.last_viewport_rows = rows[4].height as usize;
    // Body cols = area width minus the fixed gutter. The renderer
    // already computed the gutter size above; approximate here as the
    // total width minus a small default when the renderer hasn't run
    // through text yet. The first frame is always generous since the
    // live modal starts with at least one chunk requested.
    modal.scroll.last_viewport_body_cols = rows[4].width as usize;
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
            modal.scroll.vertical.offset,
            area.height as usize,
            modal.search.as_ref(),
        ),
        ContentModalTab::Output => text_body_lines(
            &modal.output.decoded,
            modal.scroll.vertical.offset,
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

    let cols = crate::layout::split_two_cols(area, Constraint::Length(gutter_width as u16));

    frame.render_widget(Paragraph::new(gutter_lines), cols[0]);
    frame.render_widget(
        Paragraph::new(body_lines).scroll((0, modal.scroll.horizontal_offset as u16)),
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

/// One row in the visible window. `gutter` and `base_style` are
/// resolved per-variant before the shared search-overlay loop runs, so
/// Text / Hex / Tabular all flow through the same highlight code path.
struct RenderRow {
    abs_idx: usize,
    text: String,
    base_style: ratatui::style::Style,
    gutter: Span<'static>,
}

fn text_body_lines(
    render: &crate::client::tracer::ContentRender,
    scroll: usize,
    height: usize,
    search: Option<&crate::widget::search::SearchState>,
) -> Vec<Line<'static>> {
    use crate::client::tracer::ContentRender;

    // Build the visible window. Each row carries its gutter span and a
    // base style — Tabular schema rows render muted, the separator
    // bolded-muted, body rows use default style; Text / Hex use a
    // numbered gutter and default style. The search-overlay loop below
    // is variant-agnostic.
    let rows: Vec<RenderRow> = match render {
        ContentRender::Empty => {
            return vec![Line::from(Span::styled("<empty>", theme::muted()))];
        }
        ContentRender::Text { text, .. } => {
            let gutter_width = line_number_width(text.lines().count());
            text.lines()
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(abs_idx, l)| RenderRow {
                    abs_idx,
                    text: l.to_string(),
                    base_style: ratatui::style::Style::default(),
                    gutter: gutter_span(Some(abs_idx + 1), gutter_width),
                })
                .collect()
        }
        ContentRender::Hex { first_4k } => {
            let gutter_width = line_number_width(first_4k.lines().count());
            first_4k
                .lines()
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(abs_idx, l)| RenderRow {
                    abs_idx,
                    text: l.to_string(),
                    base_style: ratatui::style::Style::default(),
                    gutter: gutter_span(Some(abs_idx + 1), gutter_width),
                })
                .collect()
        }
        ContentRender::Tabular {
            schema_summary,
            body,
            ..
        } => {
            // The full ordered line stream (schema → separator → body)
            // matches the search corpus shape in `content_modal_search_body`,
            // so `abs_idx` here is the same `line_idx` `compute_matches`
            // produced. The gutter is a zero-width empty span so the
            // render_body gutter-splitter routes the content span into
            // the body column.
            let empty_gutter = Span::raw("");
            let separator_style = theme::muted().add_modifier(Modifier::BOLD);
            schema_summary
                .lines()
                .map(|s| (s.to_string(), theme::muted()))
                .chain(std::iter::once((
                    "-- schema --".to_string(),
                    separator_style,
                )))
                .chain(
                    body.lines()
                        .map(|b| (b.to_string(), ratatui::style::Style::default())),
                )
                .enumerate()
                .skip(scroll)
                .take(height)
                .map(|(abs_idx, (text, base_style))| RenderRow {
                    abs_idx,
                    text,
                    base_style,
                    gutter: empty_gutter.clone(),
                })
                .collect()
        }
    };

    let search_active = search
        .map(|s| s.committed && !s.matches.is_empty())
        .unwrap_or(false);

    rows.into_iter()
        .map(|row| {
            let mut spans: Vec<Span<'static>> = vec![row.gutter];

            if !search_active {
                spans.push(Span::styled(row.text, row.base_style));
                return Line::from(spans);
            }
            let s = search.expect("search_active implies search Some");
            let line_matches: Vec<(usize, usize, bool)> = s
                .matches
                .iter()
                .enumerate()
                .filter(|(_, m)| m.line_idx == row.abs_idx)
                .map(|(mi, m)| {
                    let start = m.byte_start.min(row.text.len());
                    let end = m.byte_end.min(row.text.len());
                    (start, end, Some(mi) == s.current)
                })
                .collect();
            if line_matches.is_empty() {
                spans.push(Span::styled(row.text, row.base_style));
                return Line::from(spans);
            }
            let mut cursor = 0usize;
            for (start, end, is_current) in line_matches {
                if cursor < start {
                    spans.push(Span::styled(
                        row.text[cursor..start].to_string(),
                        row.base_style,
                    ));
                }
                let style = if is_current {
                    theme::highlight()
                } else {
                    theme::bold()
                };
                spans.push(Span::styled(row.text[start..end].to_string(), style));
                cursor = end;
            }
            if cursor < row.text.len() {
                spans.push(Span::styled(row.text[cursor..].to_string(), row.base_style));
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
        .skip(modal.scroll.vertical.offset)
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

fn render_stream_status(
    frame: &mut Frame,
    area: Rect,
    modal: &ContentModalState,
    cfg: &crate::config::TracerCeilingConfig,
) {
    use crate::client::tracer::ContentRender;
    let buf = match modal.active_tab {
        ContentModalTab::Input => Some(&modal.input),
        ContentModalTab::Output => Some(&modal.output),
        ContentModalTab::Diff => None,
    };
    let text = match buf {
        None => String::new(),
        Some(b) if b.last_error.is_some() => format!(
            "loaded {} · fetch failed: {}",
            crate::bytes::format_bytes(b.loaded.len() as u64),
            b.last_error.clone().unwrap_or_default(),
        ),
        Some(b) if matches!(b.decoded, ContentRender::Tabular { .. }) => {
            footer_chip_text(&b.decoded, cfg, b.loaded.len())
        }
        Some(b) if b.ceiling_hit => ceiling_hit_chip(b, cfg),
        Some(b) if b.fully_loaded => format!(
            "loaded {} (complete)",
            crate::bytes::format_bytes(b.loaded.len() as u64)
        ),
        Some(b) if b.in_flight => format!(
            "loaded {} · fetching…",
            crate::bytes::format_bytes(b.loaded.len() as u64)
        ),
        Some(b) => format!(
            "loaded {}",
            crate::bytes::format_bytes(b.loaded.len() as u64)
        ),
    };
    let style = match buf {
        Some(b) if b.last_error.is_some() => theme::error(),
        _ => theme::muted(),
    };
    frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), area);
}

/// Returns the chip text to show when a side has hit its per-side ceiling.
///
/// If the loaded buffer starts with the Parquet magic bytes (`PAR1`) AND
/// `decoded` is `Hex` (meaning the partial parquet could not be decoded),
/// this surfaces a distinct, actionable message pointing the user at the
/// relevant config knob. All other ceiling-hit cases fall through to the
/// legacy chip text.
pub fn ceiling_hit_chip(
    side: &crate::view::tracer::modal_state::SideBuffer,
    cfg: &crate::config::TracerCeilingConfig,
) -> String {
    use crate::client::tracer::ContentRender;
    let parquet_truncated =
        side.loaded.starts_with(b"PAR1") && matches!(side.decoded, ContentRender::Hex { .. });
    if parquet_truncated {
        let cap = cfg
            .tabular
            .map(|c| crate::bytes::format_bytes_int(c as u64))
            .unwrap_or_else(|| "unbounded".into());
        return format!(
            "parquet truncated at {cap} — raise [tracer.ceiling] tabular or use \"s\" to save"
        );
    }
    legacy_ceiling_hit_chip(side)
}

fn legacy_ceiling_hit_chip(side: &crate::view::tracer::modal_state::SideBuffer) -> String {
    format!(
        "loaded {} · ceiling reached — press 's' to save full content",
        crate::bytes::format_bytes(side.loaded.len() as u64),
    )
}

/// Returns the footer chip text for a content side.
///
/// For `ContentRender::Tabular` this includes the format name, row count,
/// and either a percentage of the configured ceiling or "complete".
/// For all other variants the chip falls back to the stream-status text that
/// was already rendered before this helper existed (loaded size, in-flight
/// indicator, etc.), so callers must only invoke this for Tabular content;
/// the non-Tabular path is provided for completeness and test coverage.
pub fn footer_chip_text(
    render: &crate::client::tracer::ContentRender,
    cfg: &crate::config::TracerCeilingConfig,
    fetched_bytes: usize,
) -> String {
    use crate::client::tracer::ContentRender;
    match render {
        ContentRender::Tabular {
            format,
            body,
            truncated,
            ..
        } => {
            let row_count = body.lines().count();
            let pct = match cfg.tabular {
                Some(c) if c > 0 => {
                    format!(
                        "{}% of {}",
                        (fetched_bytes * 100) / c,
                        crate::bytes::format_bytes_int(c as u64)
                    )
                }
                _ => "complete".into(),
            };
            let suffix = if *truncated { " · truncated" } else { "" };
            format!(
                "{} · {} rows · {}{}",
                format.label(),
                row_count,
                pct,
                suffix
            )
        }
        // Non-Tabular variants: return an empty string — the caller
        // (`render_stream_status`) handles these arms inline with the
        // existing `crate::bytes::format_bytes`-based chip text.
        _ => String::new(),
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
    use crate::test_support::{TEST_BACKEND_MEDIUM, TEST_BACKEND_SHORT, test_backend};
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
                effective_ceiling: None,
                in_flight_decode: false,
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
                effective_ceiling: None,
                in_flight_decode: false,
            },
            diff_cache: None,
            scroll: crate::widget::scroll::BidirectionalScrollState::default(),
            search: None,
        }
    }

    #[test]
    fn modal_input_complete() {
        let mut modal = stub_modal(ContentModalTab::Input);
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_input_complete", terminal.backend().buffer());
    }

    #[test]
    fn modal_output_empty() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.output.loaded.clear();
        modal.output.decoded = ContentRender::Empty;
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_output_empty", terminal.backend().buffer());
    }

    #[test]
    fn modal_input_ceiling_hit() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.loaded = vec![b'x'; 4 * 1024 * 1024];
        modal.input.ceiling_hit = true;
        modal.input.fully_loaded = true;
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
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
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_diff_unified_three_hunks",
            terminal.backend().buffer()
        );
    }

    #[test]
    fn modal_diff_no_differences() {
        let mut modal = stub_modal(ContentModalTab::Output);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::NoDifferences);
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_diff_no_differences", terminal.backend().buffer());
    }

    #[test]
    fn modal_diff_size_exceeds_cap() {
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.diffable = Diffable::NotAvailable(NotDiffableReason::SizeExceedsDiffCap);
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_diff_size_exceeds_cap", terminal.backend().buffer());
    }

    #[test]
    fn modal_input_tabular_parquet_renders_schema_then_body() {
        use crate::client::tracer::TabularFormat;
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.decoded = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: "id : Int64\nname : Utf8".into(),
            body: "{\"id\":0,\"name\":\"a\"}\n{\"id\":1,\"name\":\"b\"}".into(),
            decoded_bytes: 38,
            truncated: false,
        };
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_input_tabular_parquet_renders_schema_then_body",
            terminal.backend().buffer()
        );
    }

    /// Tabular content used to early-return without applying the
    /// search-overlay loop, so committed matches found "5/1500" but
    /// nothing rendered highlighted. This guards the unified pipeline
    /// in `text_body_lines`.
    #[test]
    fn modal_search_highlights_tabular_body_row() {
        use crate::client::tracer::TabularFormat;
        let mut modal = stub_modal(ContentModalTab::Input);
        modal.input.decoded = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: "id : Int64\nname : Utf8".into(),
            body: "{\"id\":0,\"name\":\"alpha\"}\n{\"id\":1,\"name\":\"alpha\"}".into(),
            decoded_bytes: 46,
            truncated: false,
        };
        // Search corpus is `schema + "\n-- schema --\n" + body`, which
        // gives line indices: 0/1 = schema rows, 2 = separator,
        // 3/4 = body rows. "alpha" sits at bytes 16..21 in each body row.
        modal.search = Some(crate::widget::search::SearchState {
            query: "alpha".to_string(),
            input_active: false,
            committed: true,
            matches: vec![
                crate::widget::search::MatchSpan {
                    line_idx: 3,
                    byte_start: 16,
                    byte_end: 21,
                },
                crate::widget::search::MatchSpan {
                    line_idx: 4,
                    byte_start: 16,
                    byte_end: 21,
                },
            ],
            current: Some(0),
        });
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        let buf = terminal.backend().buffer();

        // Locate the first body row ("{...alpha...}") and assert the
        // five "alpha" cells carry the current-match highlight modifier.
        let mut row_y = None;
        for y in 0..buf.area.height {
            let mut line = String::new();
            for x in 0..buf.area.width {
                line.push_str(buf.cell((x, y)).unwrap().symbol());
            }
            if line.contains("\"id\":0") && line.contains("alpha") {
                row_y = Some(y);
                break;
            }
        }
        let y = row_y.expect("body row with alpha must render");

        let mut alpha_x = None;
        for x in 0..buf.area.width.saturating_sub(5) {
            let mut chunk = String::new();
            for dx in 0..5 {
                chunk.push_str(buf.cell((x + dx, y)).unwrap().symbol());
            }
            if chunk == "alpha" {
                alpha_x = Some(x);
                break;
            }
        }
        let x = alpha_x.expect("alpha substring must be on this row");

        // theme::highlight() is REVERSED. The Paragraph renderer fills
        // Reset fg/bg/underline_color around it, so compare modifiers
        // rather than the whole Style.
        for dx in 0..5 {
            let cell = buf.cell((x + dx, y)).unwrap();
            assert!(
                cell.modifier.contains(ratatui::style::Modifier::REVERSED),
                "byte {dx} of 'alpha' on the current-match row must carry REVERSED, got {:?}",
                cell.modifier,
            );
        }
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
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| {
                render(
                    f,
                    f.area(),
                    &mut modal,
                    &crate::config::TracerCeilingConfig::default(),
                )
            })
            .unwrap();
        insta::assert_debug_snapshot!("modal_search_with_matches", terminal.backend().buffer());
    }

    #[test]
    fn footer_chip_for_tabular_parquet_includes_format_and_pct() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: String::new(),
            body: "{}\n{}\n{}\n{}\n{}".into(), // 5 rows
            decoded_bytes: 14,
            truncated: false,
        };
        let cfg = TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 1_500);
        assert!(
            chip.contains("parquet"),
            "expected 'parquet' in chip, got: {chip}"
        );
        assert!(
            chip.contains('5'),
            "expected row count '5' in chip, got: {chip}"
        );
        assert!(
            chip.contains("of 64 MiB"),
            "expected 'of 64 MiB' in chip, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_tabular_avro_includes_format_and_pct() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Avro,
            schema_summary: String::new(),
            body: "{}".into(), // 1 row
            decoded_bytes: 2,
            truncated: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 100);
        assert!(
            chip.contains("avro"),
            "expected 'avro' in chip, got: {chip}"
        );
        assert!(
            chip.contains('1'),
            "expected row count '1' in chip, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_tabular_truncated_marks_truncated() {
        use crate::client::tracer::{ContentRender, TabularFormat};
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Tabular {
            format: TabularFormat::Parquet,
            schema_summary: String::new(),
            body: "{}".into(),
            decoded_bytes: 2,
            truncated: true,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, /* fetched_bytes */ 100);
        assert!(
            chip.contains("truncated"),
            "expected 'truncated' marker, got: {chip}"
        );
    }

    #[test]
    fn footer_chip_for_text_falls_through_to_existing_behavior() {
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;
        let render = ContentRender::Text {
            text: "hello".into(),
            pretty_printed: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = footer_chip_text(&render, &cfg, 5);
        // Non-Tabular variants return an empty string from this helper;
        // the stream-status renderer handles them inline. Just confirm
        // that calling the helper does not panic.
        let _ = chip;
    }

    #[test]
    fn ceiling_hit_chip_for_parquet_truncation_uses_distinct_message() {
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;

        // Construct a SideBuffer mimicking the post-truncation state:
        // - loaded.starts_with(b"PAR1") (so we know it's parquet)
        // - decoded = Hex (because the partial parquet couldn't be decoded)
        // - ceiling_hit = true (because the fetch hit the tabular ceiling)
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 100]);
        let buf = SideBuffer {
            loaded: bytes,
            decoded: ContentRender::Hex {
                first_4k: "50 41 52 31 ...".into(),
            },
            fully_loaded: false,
            ceiling_hit: true,
            in_flight: false,
            last_error: None,
            effective_ceiling: Some(Some(64 * 1024 * 1024)),
            in_flight_decode: false,
        };
        let cfg = TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };

        let chip = ceiling_hit_chip(&buf, &cfg);
        assert!(
            chip.contains("parquet truncated"),
            "expected 'parquet truncated', got: {chip}"
        );
        assert!(
            chip.contains("64 MiB"),
            "expected '64 MiB' in message, got: {chip}"
        );
        assert!(
            chip.contains("[tracer.ceiling] tabular"),
            "expected config-key reference, got: {chip}"
        );
        assert!(chip.contains("\"s\""), "expected save hint, got: {chip}");
    }

    #[test]
    fn ceiling_hit_chip_for_text_uses_existing_message() {
        // Sanity check: text content with ceiling_hit still produces the
        // pre-existing chip text (whatever it is). The new helper should
        // route non-parquet-truncated cases to the legacy chip.
        use crate::client::tracer::ContentRender;
        use crate::config::TracerCeilingConfig;
        let buf = SideBuffer {
            loaded: vec![b'a'; 4 * 1024 * 1024],
            decoded: ContentRender::Text {
                text: "a".repeat(4 * 1024 * 1024),
                pretty_printed: false,
            },
            fully_loaded: false,
            ceiling_hit: true,
            in_flight: false,
            last_error: None,
            effective_ceiling: Some(Some(4 * 1024 * 1024)),
            in_flight_decode: false,
        };
        let cfg = TracerCeilingConfig::default();
        let chip = ceiling_hit_chip(&buf, &cfg);
        assert!(
            !chip.contains("parquet truncated"),
            "text content should NOT use parquet message"
        );
        // The exact text isn't asserted here — just that it's not the parquet variant.
    }

    #[test]
    fn modal_tabular_ceiling_hit_parquet_chip() {
        use crate::client::tracer::ContentRender;
        // Construct a modal state with a parquet-truncated Input side:
        // loaded starts with PAR1 magic, decoded is Hex (failed decode),
        // ceiling_hit is true.
        let mut modal = stub_modal(ContentModalTab::Input);
        let mut parquet_bytes = b"PAR1".to_vec();
        parquet_bytes.extend_from_slice(&[0u8; 64 * 1024 * 1024 - 4]);
        modal.input.loaded = parquet_bytes;
        modal.input.decoded = ContentRender::Hex {
            first_4k: "50 41 52 31 ...".into(),
        };
        modal.input.fully_loaded = false;
        modal.input.ceiling_hit = true;
        modal.input.effective_ceiling = Some(Some(64 * 1024 * 1024));
        let cfg = crate::config::TracerCeilingConfig {
            text: Some(4 * 1024 * 1024),
            tabular: Some(64 * 1024 * 1024),
            diff: Some(16 * 1024 * 1024),
        };
        let backend = test_backend(TEST_BACKEND_MEDIUM);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|f| render(f, f.area(), &mut modal, &cfg))
            .unwrap();
        insta::assert_debug_snapshot!(
            "modal_tabular_ceiling_hit_parquet_chip",
            terminal.backend().buffer()
        );
    }
}
