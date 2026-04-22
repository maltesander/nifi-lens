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
    // Search strip / hint added in T22.
    modal.last_viewport_rows = rows[4].height as usize;
}

fn render_header(frame: &mut Frame, area: Rect, modal: &ContentModalState) {
    let h = &modal.header;
    let at = format!(
        "at    {}   pg {}",
        short_iso(&h.event_timestamp_iso),
        h.pg_path,
    );
    let sizes = format!(
        "sizes in {} → out {}  ({})",
        size_ui(h.input_size),
        size_ui(h.output_size),
        delta_ui(h.input_size, h.output_size),
    );
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
    let lines: Vec<Line<'static>> = match modal.active_tab {
        ContentModalTab::Input => text_body_lines(
            &modal.input.decoded,
            modal.scroll_offset,
            area.height as usize,
        ),
        ContentModalTab::Output => text_body_lines(
            &modal.output.decoded,
            modal.scroll_offset,
            area.height as usize,
        ),
        ContentModalTab::Diff => diff_body_lines(modal, area.height as usize),
    };
    frame.render_widget(Paragraph::new(lines), area);
}

fn text_body_lines(
    render: &crate::client::tracer::ContentRender,
    scroll: usize,
    height: usize,
) -> Vec<Line<'static>> {
    use crate::client::tracer::ContentRender;
    match render {
        ContentRender::Empty => vec![Line::from(Span::styled("<empty>", theme::muted()))],
        ContentRender::Text { text, .. } => text
            .lines()
            .skip(scroll)
            .take(height)
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect(),
        ContentRender::Hex { first_4k } => first_4k
            .lines()
            .skip(scroll)
            .take(height)
            .map(|l| Line::from(Span::raw(l.to_string())))
            .collect(),
    }
}

fn diff_body_lines(modal: &ContentModalState, height: usize) -> Vec<Line<'static>> {
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

    for (idx, dl) in cache
        .lines
        .iter()
        .enumerate()
        .skip(modal.scroll_offset)
        .take(height)
    {
        if idx == next_hunk_line {
            let h = &hunks[hunk_idx];
            rows.push(Line::from(Span::styled(
                format!("@@ input L{} · output L{} @@", h.input_line, h.output_line),
                theme::hunk_header(),
            )));
            hunk_idx += 1;
            next_hunk_line = hunks
                .get(hunk_idx)
                .map(|h| h.line_idx as usize)
                .unwrap_or(usize::MAX);
            continue;
        }
        let (prefix, style) = match dl.tag {
            similar::ChangeTag::Insert => ("+ ", theme::diff_add()),
            similar::ChangeTag::Delete => ("- ", theme::diff_del()),
            similar::ChangeTag::Equal => ("  ", ratatui::style::Style::default()),
        };
        rows.push(Line::from(vec![
            Span::styled(prefix, style),
            Span::styled(dl.text.clone(), style),
        ]));
    }
    rows
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
            let d = o as i64 - i as i64;
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
