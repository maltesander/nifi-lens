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
    render_footer_hint(frame, rows[8]);
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

fn render_footer_hint(frame: &mut Frame, area: Rect) {
    let text = "[Tab] switch · [/] find · [n/N] match · [Ctrl ↓↑] hunk · [c] copy · [s] save · [Esc] close";
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
            last_viewport_rows: 0,
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
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"total\":329.99,".into(),
                    input_line: None,
                    output_line: Some(3),
                },
                DiffLine {
                    tag: similar::ChangeTag::Insert,
                    text: "\"tax\":30.99".into(),
                    input_line: None,
                    output_line: Some(4),
                },
            ],
            hunks: vec![HunkAnchor {
                line_idx: 0,
                input_line: 3,
                output_line: 3,
            }],
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
        modal.search = Some(crate::widget::search::SearchState {
            query: "total".to_string(),
            input_active: false,
            committed: true,
            matches: vec![crate::widget::search::MatchSpan {
                line_idx: 2,
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
