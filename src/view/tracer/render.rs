//! Ratatui renderer for the Tracer tab.
//!
//! Layout dispatches by `TracerMode`:
//!
//! ```text
//! ┌─ Tracer ───────────────────────────────────────┐
//! │                                                 │
//! │          (mode-specific content)                │
//! │                                                 │
//! └─────────────────────────────────────────────────┘
//! ```

use std::time::SystemTime;

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Gauge, Paragraph, Row, Table, TableState};

use crate::client::tracer::{AttributeTriple, ContentRender, ContentSide};
use crate::theme;
use crate::view::tracer::state::{
    AttributeClass, AttributeDiffMode, ContentPane, DetailTab, EntryState, EventDetail,
    LatestEventsView, LineageFocus, LineageRunningState, LineageView, TracerMode, TracerState,
};
use crate::widget::panel::Panel;

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &TracerState,
    cfg: &crate::timestamp::TimestampConfig,
) {
    let now = time::OffsetDateTime::now_utc();

    match &state.mode {
        TracerMode::Entry(entry) => {
            let block = Panel::new(" Tracer ").into_block();
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_entry(frame, inner, entry, state.last_error.as_deref());
        }
        TracerMode::LineageRunning(running) => {
            let block = Panel::new(" Tracer — Running Lineage Query ").into_block();
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_lineage_running(frame, inner, running);
        }
        TracerMode::Lineage(view) => {
            // Lineage mode has two stacked sub-panes: timeline + detail.
            // Each gets its own bordered Panel. The detail panel flips
            // its focused border-style when the user `l`s into the
            // nested attribute table.
            let rows = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(1),                    // lineage timeline
                    Constraint::Length(DETAIL_HEIGHT + 2), // detail pane (+2 for border)
                ])
                .split(area);

            let timeline_focused = matches!(
                view.focus,
                crate::view::tracer::state::LineageFocus::Timeline
            );
            let attributes_focused = matches!(
                view.focus,
                crate::view::tracer::state::LineageFocus::Attributes { .. }
            );

            let lineage_block = Panel::new(" Lineage ")
                .focused(timeline_focused)
                .into_block();
            let lineage_inner = lineage_block.inner(rows[0]);
            frame.render_widget(lineage_block, rows[0]);
            render_lineage_timeline(frame, lineage_inner, view, now, cfg);

            let detail_block = Panel::new(" Detail ")
                .focused(attributes_focused)
                .into_block();
            let detail_inner = detail_block.inner(rows[1]);
            frame.render_widget(detail_block, rows[1]);
            render_lineage_detail(frame, detail_inner, view);
        }
        TracerMode::LatestEvents(view) => {
            let block = Panel::new(" Tracer — Latest Events ").into_block();
            let inner = block.inner(area);
            frame.render_widget(block, area);
            render_latest_events(frame, inner, view, state.last_error.as_deref(), now, cfg);
        }
    }
}

// ── Entry ─────────────────────────────────────────────────────────────────────

fn render_entry(frame: &mut Frame, area: Rect, entry: &EntryState, last_error: Option<&str>) {
    // Three vertical sections: prompt, input box, footer hint.
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1), // prompt
            Constraint::Length(1), // blank
            Constraint::Length(1), // input
            Constraint::Length(1), // blank
            Constraint::Length(1), // footer
            Constraint::Fill(1),
        ])
        .split(area);

    // Prompt.
    let prompt = Paragraph::new(Span::styled(
        "Paste a flowfile UUID to trace its lineage",
        theme::muted(),
    ))
    .alignment(Alignment::Center);
    frame.render_widget(prompt, rows[1]);

    // Input box.
    let input_text = if entry.input.is_empty() {
        Line::from(vec![
            Span::styled("UUID: ", theme::muted()),
            Span::styled("_", theme::muted()),
        ])
    } else {
        Line::from(vec![
            Span::styled("UUID: ", theme::muted()),
            Span::styled(entry.input.clone(), theme::accent()),
            Span::styled("_", theme::muted()),
        ])
    };
    let input_para = Paragraph::new(input_text).alignment(Alignment::Center);
    frame.render_widget(input_para, rows[3]);

    // Footer: error message or hints.
    let footer = if let Some(err) = last_error {
        Paragraph::new(Span::styled(err.to_string(), theme::error())).alignment(Alignment::Center)
    } else {
        Paragraph::new(Span::styled(
            "Enter submit · Esc clear · ? help",
            theme::muted(),
        ))
        .alignment(Alignment::Center)
    };
    frame.render_widget(footer, rows[5]);
}

// ── LineageRunning ────────────────────────────────────────────────────────────

fn render_lineage_running(frame: &mut Frame, area: Rect, running: &LineageRunningState) {
    let elapsed_secs = SystemTime::now()
        .duration_since(running.started_at)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1), // status line
            Constraint::Length(1), // blank
            Constraint::Length(1), // gauge
            Constraint::Length(1), // blank
            Constraint::Length(1), // elapsed + cancel hint
            Constraint::Fill(1),
        ])
        .split(area);

    // Status.
    let status = Paragraph::new(Line::from(vec![
        Span::raw("Running lineage query for "),
        Span::styled(running.uuid.clone(), theme::accent()),
        Span::raw("…"),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(status, rows[1]);

    // Progress gauge.
    let gauge = Gauge::default()
        .gauge_style(crate::theme::accent())
        .percent(running.percent as u16)
        .label(format!("{}%", running.percent));
    // Centre the gauge horizontally to avoid it spanning full width.
    let gauge_area = horizontal_center(rows[3], 60);
    frame.render_widget(gauge, gauge_area);

    // Elapsed + cancel hint.
    let hint = Paragraph::new(Line::from(vec![
        Span::styled(format!("elapsed {elapsed_secs}s"), theme::muted()),
        Span::raw("   "),
        Span::styled("Esc to cancel", theme::muted()),
    ]))
    .alignment(Alignment::Center);
    frame.render_widget(hint, rows[5]);
}

// ── Lineage ───────────────────────────────────────────────────────────────────

const DETAIL_HEIGHT: u16 = 14;

fn render_lineage_timeline(
    frame: &mut Frame,
    area: Rect,
    view: &LineageView,
    now: time::OffsetDateTime,
    cfg: &crate::timestamp::TimestampConfig,
) {
    let events = &view.snapshot.events;
    let visible = area.height as usize;
    let scroll_offset = if visible == 0 || view.selected_event < visible {
        0
    } else {
        view.selected_event + 1 - visible
    };

    let window_end = events.len().min(scroll_offset + visible);
    let window = &events[scroll_offset..window_end];
    let selected_in_window = view.selected_event.saturating_sub(scroll_offset);

    let table_rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let is_selected = idx == selected_in_window;
            let base_style = if is_selected {
                theme::cursor_row()
            } else {
                Style::default()
            };

            let marker = if is_selected { ">" } else { " " };
            let time = format_tracer_time(&e.event_time_iso, now, cfg, true);
            // "ATTRIBUTES_MODIFIED" is 19 chars; pad to 19 + 1 space = 20.
            let event_type = format!("{:<19}", truncate(&e.event_type, 19));
            // component_type is populated from lineage nodes (component_name
            // is empty for lineage queries). Pad to 19 + 1 space = 20.
            let comp_type = format!("{:<19}", truncate(&e.component_type, 19));

            let is_fail = e.relationship.as_deref().is_some_and(|r| r == "failure");

            // Build enrichment spans from the detail cache.
            let enrichment = lineage_enrichment_spans(
                view.loaded_details.get(&e.event_id),
                is_fail,
                is_selected,
            );

            Row::new(vec![
                Cell::from(Span::styled(marker, base_style)),
                Cell::from(Span::styled(time, base_style)),
                Cell::from(Span::styled(event_type, base_style)),
                Cell::from(Span::styled(comp_type, base_style)),
                Cell::from(enrichment),
            ])
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),  // gutter
            Constraint::Length(20), // Mon DD HH:MM:SS.mmm (19) + 1 space
            Constraint::Length(20), // event type (19 + 1 space)
            Constraint::Length(20), // component type (19 + 1 space)
            Constraint::Fill(1),    // enrichment: attr changes, content, fail tag
        ],
    );
    frame.render_widget(table, area);
}

fn render_lineage_detail(frame: &mut Frame, area: Rect, view: &LineageView) {
    let inner = area;

    match &view.event_detail {
        EventDetail::NotLoaded => {
            let para = Paragraph::new(Span::styled(
                "Navigate to an event to load its detail",
                theme::muted(),
            ))
            .alignment(Alignment::Center);
            let mid = inner.height.saturating_sub(1) / 2;
            let spot = Rect {
                x: inner.x,
                y: inner.y + mid,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(para, spot);
        }
        EventDetail::Loading => {
            let para = Paragraph::new(Span::styled("Loading event detail\u{2026}", theme::muted()))
                .alignment(Alignment::Center);
            let mid = inner.height.saturating_sub(1) / 2;
            let spot = Rect {
                x: inner.x,
                y: inner.y + mid,
                width: inner.width,
                height: 1,
            };
            frame.render_widget(para, spot);
        }
        EventDetail::Failed(err) => {
            let para = Paragraph::new(Span::styled(
                format!("failed to load event detail: {err}"),
                theme::error(),
            ));
            frame.render_widget(para, inner);
        }
        EventDetail::Loaded { event, content } => {
            render_lineage_detail_loaded(frame, inner, event, content, view);
        }
    }
}

fn render_lineage_detail_loaded(
    frame: &mut Frame,
    area: Rect,
    event: &crate::client::tracer::ProvenanceEventDetail,
    content: &ContentPane,
    view: &LineageView,
) {
    let s = &event.summary;
    let rel = s.relationship.as_deref().unwrap_or("");
    let details = s.details.as_deref().unwrap_or("");

    // Split into: header | tab bar | body
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // event header
            Constraint::Length(1), // tab bar
            Constraint::Fill(1),   // body (attributes or content)
        ])
        .split(area);

    // ── Header ──
    let header_line = Line::from(vec![
        Span::styled(format!("Event #{} \u{2014} ", s.event_id), theme::accent()),
        Span::raw(s.component_name.clone()),
        Span::styled(
            format!("  ({} \u{00b7} {})", s.event_type, rel),
            theme::muted(),
        ),
        Span::raw(if details.is_empty() {
            String::new()
        } else {
            format!("  {details}")
        }),
    ]);
    frame.render_widget(Paragraph::new(header_line), rows[0]);

    // ── Tab bar ──
    let has_input = event.input_available;
    let has_output = event.output_available;
    render_detail_tab_bar(
        frame,
        rows[1],
        view.active_detail_tab,
        has_input,
        has_output,
    );

    // ── Body driven by active tab ──
    match view.active_detail_tab {
        DetailTab::Attributes => {
            render_attribute_table(frame, rows[2], &event.attributes, view);
        }
        DetailTab::Input | DetailTab::Output => {
            render_content_panel(frame, rows[2], content, view);
        }
    }
}

/// Renders the "Attributes | Input | Output" tab bar.
///
/// The active tab is bold + accent colour. Disabled tabs (no claim) are
/// rendered dimmed. Enabled-but-inactive tabs use plain muted style.
fn render_detail_tab_bar(
    frame: &mut Frame,
    area: Rect,
    active: DetailTab,
    has_input: bool,
    has_output: bool,
) {
    let tab_style = |tab: DetailTab, enabled: bool| {
        if !enabled {
            theme::border_dim()
        } else if tab == active {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            theme::muted()
        }
    };

    let sep = Span::styled(" | ", theme::border_dim());
    let line = Line::from(vec![
        Span::styled("Attributes", tab_style(DetailTab::Attributes, true)),
        sep.clone(),
        Span::styled("Input", tab_style(DetailTab::Input, has_input)),
        sep,
        Span::styled("Output", tab_style(DetailTab::Output, has_output)),
    ]);
    frame.render_widget(Paragraph::new(line), area);
}

/// Style helper — maps an [`AttributeClass`] onto the diff palette:
/// added→green, updated→yellow, deleted→red, unchanged→grey.
fn attribute_row_style(class: AttributeClass) -> Style {
    match class {
        AttributeClass::Added => theme::success(),
        AttributeClass::Updated => theme::warning(),
        AttributeClass::Deleted => theme::error(),
        AttributeClass::Unchanged => theme::muted(),
    }
}

fn render_attribute_table(
    frame: &mut Frame,
    area: Rect,
    attributes: &[AttributeTriple],
    view: &LineageView,
) {
    if area.height == 0 {
        return;
    }

    // First row is a header + legend.
    let changed_count = attributes.iter().filter(|a| a.is_changed()).count();
    let mode_indicator = match view.diff_mode {
        AttributeDiffMode::All => "[ \u{25ba} All | Changed ]",
        AttributeDiffMode::Changed => "[ All | \u{25ba} Changed ]",
    };
    let attr_header = Line::from(vec![
        Span::styled("Attributes  ", theme::muted()),
        Span::styled(
            format!("{mode_indicator} ({changed_count} changed)  "),
            theme::muted(),
        ),
        Span::styled("+added", theme::success()),
        Span::styled(" \u{00b7} ", theme::muted()),
        Span::styled("~updated", theme::warning()),
        Span::styled(" \u{00b7} ", theme::muted()),
        Span::styled("-deleted", theme::error()),
        Span::styled(" \u{00b7} ", theme::muted()),
        Span::styled(" unchanged", theme::muted()),
    ]);

    let visible_attrs: Vec<&AttributeTriple> = attributes
        .iter()
        .filter(|a| view.diff_mode.matches(a))
        .collect();

    // We have area.height rows total. Use row 0 for header, rest for data rows.
    let header_area = Rect {
        x: area.x,
        y: area.y,
        width: area.width,
        height: 1,
    };
    frame.render_widget(Paragraph::new(attr_header), header_area);

    if area.height <= 1 {
        return;
    }
    let table_area = Rect {
        x: area.x,
        y: area.y + 1,
        width: area.width,
        height: area.height - 1,
    };

    // Inline separator cell used between every data column. Rendered
    // dim so the column rule reads as chrome, not content.
    let sep_cell = || Cell::from(Span::styled("\u{2502}", theme::border_dim()));

    let table_rows: Vec<Row> = visible_attrs
        .iter()
        .map(|attr| {
            let prev = attr.previous.as_deref().unwrap_or("(none)");
            let curr = attr.current.as_deref().unwrap_or("(none)");
            let class = AttributeClass::of(attr);
            let gutter = match class {
                AttributeClass::Added => "+",
                AttributeClass::Updated => "~",
                AttributeClass::Deleted => "-",
                AttributeClass::Unchanged => " ",
            };
            let row_style = attribute_row_style(class);
            Row::new(vec![
                Cell::from(Span::styled(gutter.to_string(), row_style)),
                Cell::from(Span::styled(truncate(&attr.key, 22).to_string(), row_style)),
                sep_cell(),
                Cell::from(Span::styled(truncate(prev, 28).to_string(), row_style)),
                sep_cell(),
                Cell::from(Span::styled(truncate(curr, 28).to_string(), row_style)),
            ])
        })
        .collect();

    let header_row = Row::new(vec![
        Cell::from(""),
        Cell::from(Span::styled("key", theme::muted())),
        sep_cell(),
        Cell::from(Span::styled("previous", theme::muted())),
        sep_cell(),
        Cell::from(Span::styled("current", theme::muted())),
    ])
    .style(theme::muted().add_modifier(Modifier::BOLD));

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),  // gutter
            Constraint::Length(22), // key
            Constraint::Length(1),  // │
            Constraint::Length(28), // previous
            Constraint::Length(1),  // │
            Constraint::Fill(1),    // current
        ],
    )
    .header(header_row)
    .row_highlight_style(theme::cursor_row());

    let mut ts = TableState::default();
    if let LineageFocus::Attributes { row } = view.focus
        && row < visible_attrs.len()
    {
        ts.select(Some(row));
    }
    frame.render_stateful_widget(table, table_area, &mut ts);
}

/// Renders the content sub-pane inside a focus-aware [`Panel`].
///
/// When `view.focus` is [`LineageFocus::Content`], the panel gains a
/// thick accent border and displays a `scroll/total` indicator on the
/// right title, and the caller's layout gives it the majority of the
/// detail area. Otherwise the panel is a short un-focused strip.
fn render_content_panel(frame: &mut Frame, area: Rect, content: &ContentPane, view: &LineageView) {
    if area.height == 0 {
        return;
    }

    let focused = matches!(view.focus, LineageFocus::Content { .. });
    let scroll = match view.focus {
        LineageFocus::Content { scroll } => scroll,
        _ => 0,
    };

    // Compute left-title suffix, right-title (scroll indicator), and
    // the body text to render inside the panel.
    let (title_suffix, right_title, body_text, body_style) = match content {
        ContentPane::Collapsed => (
            "".to_string(),
            "".to_string(),
            "(collapsed \u{2014} press i for input or o for output)".to_string(),
            theme::muted(),
        ),
        ContentPane::LoadingInput => (
            " \u{00b7} input ".to_string(),
            "".to_string(),
            "loading input\u{2026}".to_string(),
            theme::muted(),
        ),
        ContentPane::LoadingOutput => (
            " \u{00b7} output ".to_string(),
            "".to_string(),
            "loading output\u{2026}".to_string(),
            theme::muted(),
        ),
        ContentPane::Shown {
            side,
            render,
            bytes_fetched,
            ..
        } => {
            let side_label = match side {
                ContentSide::Input => "input",
                ContentSide::Output => "output",
            };
            let body = match render {
                ContentRender::Text { text, .. } => text.clone(),
                ContentRender::Hex { first_4k } => first_4k.clone(),
                ContentRender::Empty => "(empty)".to_string(),
            };
            let total_lines = body.lines().count().max(1);
            let suffix = format!(" \u{00b7} {side_label} \u{00b7} {bytes_fetched} B ");
            let right = if total_lines > 1 {
                format!(
                    " {}/{} ",
                    (scroll as usize).min(total_lines - 1) + 1,
                    total_lines
                )
            } else {
                "".to_string()
            };
            (suffix, right, body, Style::default())
        }
        ContentPane::Failed(err) => (
            "".to_string(),
            "".to_string(),
            format!("error: {err}"),
            theme::error(),
        ),
    };

    let mut panel = Panel::new(format!(" Content{title_suffix}")).focused(focused);
    if !right_title.is_empty() {
        panel = panel.right(Line::from(Span::styled(right_title, theme::muted())));
    }
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    // Split the body into explicit Lines so `Paragraph::scroll` steps
    // line-by-line — wrapping a multi-line String in a single Span
    // would flatten embedded `\n` into spaces.
    let body_lines: Vec<Line> = body_text
        .lines()
        .map(|l| Line::from(Span::styled(l.to_string(), body_style)))
        .collect();
    let body = if body_lines.is_empty() {
        vec![Line::from("")]
    } else {
        body_lines
    };
    let para = Paragraph::new(body).scroll((scroll, 0));
    frame.render_widget(para, inner);
}

// ── LatestEvents ──────────────────────────────────────────────────────────────

fn render_latest_events(
    frame: &mut Frame,
    area: Rect,
    view: &LatestEventsView,
    last_error: Option<&str>,
    now: time::OffsetDateTime,
    cfg: &crate::timestamp::TimestampConfig,
) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // component label
            Constraint::Fill(1),   // event list
            Constraint::Length(1), // footer
        ])
        .split(area);

    // Component label header.
    let header = Paragraph::new(Line::from(vec![
        Span::styled("Component: ", theme::muted()),
        Span::styled(view.component_label.clone(), theme::accent()),
    ]));
    frame.render_widget(header, rows[0]);

    // Event list (or placeholder).
    if view.loading {
        let placeholder = Paragraph::new(Span::styled(
            "loading latest provenance events…",
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = rows[1].height.saturating_sub(1) / 2;
        let spot = Rect {
            x: rows[1].x,
            y: rows[1].y + mid,
            width: rows[1].width,
            height: 1,
        };
        frame.render_widget(placeholder, spot);
    } else if view.events.is_empty() {
        let placeholder = Paragraph::new(Span::styled(
            "no recent events cached for this component",
            theme::muted(),
        ))
        .alignment(Alignment::Center);
        let mid = rows[1].height.saturating_sub(1) / 2;
        let spot = Rect {
            x: rows[1].x,
            y: rows[1].y + mid,
            width: rows[1].width,
            height: 1,
        };
        frame.render_widget(placeholder, spot);
    } else {
        render_event_table(frame, rows[1], view, now, cfg);
    }

    // Footer.
    let footer_text = if let Some(err) = last_error {
        Paragraph::new(Span::styled(err.to_string(), theme::error()))
    } else {
        Paragraph::new(Span::styled(
            "Enter trace flowfile · Esc back · r refresh · c copy uuid · ? help",
            theme::muted(),
        ))
    };
    frame.render_widget(footer_text, rows[2]);
}

fn render_event_table(
    frame: &mut Frame,
    area: Rect,
    view: &LatestEventsView,
    now: time::OffsetDateTime,
    cfg: &crate::timestamp::TimestampConfig,
) {
    let visible_rows = area.height as usize;
    let scroll_offset = if visible_rows == 0 || view.selected < visible_rows {
        0
    } else {
        view.selected + 1 - visible_rows
    };

    let window_end = view.events.len().min(scroll_offset + visible_rows);
    let window = &view.events[scroll_offset..window_end];
    let selected_in_window = view.selected.saturating_sub(scroll_offset);

    let table_rows: Vec<Row> = window
        .iter()
        .enumerate()
        .map(|(idx, e)| {
            let style = if idx == selected_in_window {
                theme::cursor_row()
            } else {
                Style::default()
            };
            let marker = if idx == selected_in_window { ">" } else { " " };
            let time = format_tracer_time(&e.event_time_iso, now, cfg, false);
            let uuid_short = short_uuid(&e.flow_file_uuid);
            let relationship = e.relationship.as_deref().unwrap_or("-").to_string();
            let details = e.details.as_deref().unwrap_or("").to_string();
            Row::new(vec![
                Cell::from(marker),
                Cell::from(time),
                Cell::from(e.event_type.clone()),
                Cell::from(uuid_short),
                Cell::from(relationship),
                Cell::from(details),
            ])
            .style(style)
        })
        .collect();

    let table = Table::new(
        table_rows,
        [
            Constraint::Length(1),
            Constraint::Length(16), // Mon DD HH:MM:SS (15) + 1 space
            Constraint::Length(16),
            Constraint::Length(13),
            Constraint::Length(16),
            Constraint::Fill(1),
        ],
    );
    frame.render_widget(table, area);
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Format a NiFi timestamp string using the shared timestamp module.
///
/// - `with_ms = true` → millisecond precision (`HH:MM:SS.mmm` or `Mon DD HH:MM:SS.mmm`)
/// - `with_ms = false` → second precision (`HH:MM:SS` or `Mon DD HH:MM:SS`)
///
/// Falls back to `--:--:--.---` / `--:--:--` on parse failure.
fn format_tracer_time(
    ts: &str,
    now: time::OffsetDateTime,
    cfg: &crate::timestamp::TimestampConfig,
    with_ms: bool,
) -> String {
    match crate::timestamp::parse_nifi_timestamp(ts) {
        Some(dt) => crate::timestamp::format(dt, now, cfg, with_ms),
        None => {
            if with_ms {
                "--:--:--.---".to_string()
            } else {
                "--:--:--".to_string()
            }
        }
    }
}

/// Truncates `s` to at most `max_chars` Unicode scalar values.
/// Builds the enrichment `Line` for a lineage timeline row.
///
/// Shows:
/// - `← fail` (red) when the event was routed to the failure relationship.
/// - Attribute change summary (`+N added`, `~N changed`, `-N removed`) when a
///   detail has been loaded for this event.
/// - `in` / `out` when input or output content is available.
///
/// Returns an empty `Line` when no detail has been loaded yet and there is no
/// failure indicator.
fn lineage_enrichment_spans<'a>(
    detail: Option<&'a crate::client::tracer::ProvenanceEventDetail>,
    is_fail: bool,
    is_selected: bool,
) -> Line<'a> {
    let mut spans: Vec<Span<'a>> = Vec::new();

    if is_fail {
        spans.push(Span::styled("\u{2190} fail", theme::error()));
    }

    if let Some(d) = detail {
        let attrs = &d.attributes;
        let added = attrs
            .iter()
            .filter(|a| a.previous.is_none() && a.current.is_some())
            .count();
        let changed = attrs
            .iter()
            .filter(|a| a.previous.is_some() && a.current.is_some() && a.previous != a.current)
            .count();
        let removed = attrs
            .iter()
            .filter(|a| a.previous.is_some() && a.current.is_none())
            .count();

        let has_attr_info = added > 0 || changed > 0 || removed > 0;
        if has_attr_info {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            // On selected (reversed) rows use a plain style so the colour
            // contrast stays legible; use semantic colours on normal rows.
            if is_selected {
                let parts: Vec<String> = [
                    (added > 0).then(|| format!("+{added}")),
                    (changed > 0).then(|| format!("~{changed}")),
                    (removed > 0).then(|| format!("-{removed}")),
                ]
                .into_iter()
                .flatten()
                .collect();
                spans.push(Span::raw(parts.join(" ")));
                spans.push(Span::raw(" attrs"));
            } else {
                if added > 0 {
                    spans.push(Span::styled(format!("+{added}"), theme::success()));
                }
                if changed > 0 {
                    if added > 0 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(Span::styled(format!("~{changed}"), theme::warning()));
                }
                if removed > 0 {
                    if added > 0 || changed > 0 {
                        spans.push(Span::raw(" "));
                    }
                    spans.push(Span::styled(format!("-{removed}"), theme::error()));
                }
                spans.push(Span::styled(" attrs", theme::muted()));
            }
        }

        let has_content = d.input_available || d.output_available;
        if has_content {
            if !spans.is_empty() {
                spans.push(Span::raw("  "));
            }
            let content_style = if is_selected {
                Style::default()
            } else {
                theme::muted()
            };
            if d.input_available {
                spans.push(Span::styled("in", content_style));
            }
            if d.input_available && d.output_available {
                spans.push(Span::raw(" "));
            }
            if d.output_available {
                spans.push(Span::styled("out", content_style));
            }
        }
    }

    Line::from(spans)
}

fn truncate(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &s[..byte_idx],
        None => s,
    }
}

fn short_uuid(uuid: &str) -> String {
    // Show first 8 chars of UUID (the first segment).
    uuid.chars().take(8).collect()
}

fn horizontal_center(area: Rect, pct: u16) -> Rect {
    let margin = (100u16.saturating_sub(pct)) / 2;
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage(margin),
            Constraint::Percentage(pct),
            Constraint::Percentage(margin),
        ])
        .split(area)[1]
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::render;
    use crate::view::tracer::state::{
        LineageRunningState, TracerMode, TracerState, start_latest_events,
    };
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::time::{Duration, SystemTime};

    fn snap(state: &TracerState) -> String {
        let cfg = crate::timestamp::TimestampConfig::default();
        let backend = TestBackend::new(120, 30);
        let mut terminal = Terminal::new(backend).unwrap();
        terminal
            .draw(|frame| render(frame, frame.area(), state, &cfg))
            .unwrap();
        format!("{}", terminal.backend())
    }

    #[test]
    fn snapshot_entry_empty() {
        let state = TracerState::new();
        insta::assert_snapshot!("entry_empty", snap(&state));
    }

    #[test]
    fn snapshot_entry_with_input() {
        let mut state = TracerState::new();
        if let TracerMode::Entry(ref mut e) = state.mode {
            e.input = "550e8400-e29b-41d4-a716-446655440000".to_string();
        }
        insta::assert_snapshot!("entry_with_input", snap(&state));
    }

    #[test]
    fn snapshot_entry_invalid_uuid_banner() {
        let mut state = TracerState::new();
        if let TracerMode::Entry(ref mut e) = state.mode {
            e.input = "not-a-uuid".to_string();
        }
        state.last_error = Some("invalid UUID: not-a-uuid".to_string());
        insta::assert_snapshot!("entry_invalid_uuid_banner", snap(&state));
    }

    #[test]
    fn snapshot_latest_events_loading() {
        let mut state = TracerState::new();
        start_latest_events(&mut state, "abc-component-id-123".to_string());
        insta::assert_snapshot!("latest_events_loading", snap(&state));
    }

    #[test]
    fn snapshot_lineage_running_low_percent() {
        let mut state = TracerState::new();
        state.mode = TracerMode::LineageRunning(LineageRunningState {
            uuid: "550e8400-e29b-41d4-a716-446655440000".to_string(),
            query_id: "qry-001".to_string(),
            cluster_node_id: None,
            percent: 12,
            started_at: SystemTime::now() - Duration::from_secs(2),
            abort: None,
        });
        insta::with_settings!(
            { filters => vec![(r"elapsed \d+s", "elapsed <N>s")] },
            { insta::assert_snapshot!("lineage_running_low_percent", snap(&state)); }
        );
    }

    // ── Lineage mode snapshot helpers ─────────────────────────────────────────

    fn make_lineage_summary(
        id: i64,
        event_type: &str,
        rel: Option<&str>,
    ) -> crate::client::tracer::ProvenanceEventSummary {
        crate::client::tracer::ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-04-12T10:30:45.123Z".to_string(),
            event_type: event_type.to_string(),
            component_id: "proc-1111-2222-3333-4444".to_string(),
            component_name: "LogAttribute".to_string(),
            component_type: "LogAttribute".to_string(),
            group_id: "pg-root-aaaa-bbbb".to_string(),
            flow_file_uuid: "ff000001-0000-0000-0000-000000000001".to_string(),
            relationship: rel.map(|s| s.to_string()),
            details: None,
        }
    }

    fn make_lineage_detail(event_id: i64) -> crate::client::tracer::ProvenanceEventDetail {
        crate::client::tracer::ProvenanceEventDetail {
            summary: make_lineage_summary(event_id, "CONTENT_MODIFIED", None),
            attributes: vec![
                crate::client::tracer::AttributeTriple {
                    key: "filename".to_string(),
                    previous: Some("old_file.csv".to_string()),
                    current: Some("new_file.csv".to_string()),
                },
                crate::client::tracer::AttributeTriple {
                    key: "mime.type".to_string(),
                    previous: Some("text/plain".to_string()),
                    current: Some("text/plain".to_string()),
                },
            ],
            transit_uri: None,
            input_available: true,
            output_available: true,
            input_size: None,
            output_size: None,
        }
    }

    fn seed_lineage_state(state: &mut TracerState) {
        use crate::client::tracer::LineageSnapshot;
        use crate::view::tracer::state::{
            AttributeDiffMode, DetailTab, EventDetail, LineageFocus, LineageView,
        };

        state.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff000001-0000-0000-0000-000000000001".to_string(),
            snapshot: LineageSnapshot {
                events: vec![
                    make_lineage_summary(1, "RECEIVE", None),
                    make_lineage_summary(2, "CONTENT_MODIFIED", None),
                    make_lineage_summary(3, "SEND", Some("failure")),
                ],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 1,
            event_detail: EventDetail::NotLoaded,
            loaded_details: std::collections::HashMap::new(),
            diff_mode: AttributeDiffMode::All,
            fetched_at: SystemTime::now(),
            focus: LineageFocus::default(),
            active_detail_tab: DetailTab::default(),
        }));
    }

    #[test]
    fn snapshot_lineage_view_loading_detail() {
        use crate::view::tracer::state::EventDetail;

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.event_detail = EventDetail::Loading;
        }
        insta::assert_snapshot!("lineage_view_loading_detail", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_collapsed_content() {
        use crate::view::tracer::state::{ContentPane, EventDetail};

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Collapsed,
            };
        }
        insta::assert_snapshot!("lineage_view_collapsed_content", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_expanded_text_content() {
        use crate::client::tracer::{ContentRender, ContentSide};
        use crate::view::tracer::state::{ContentPane, DetailTab, EventDetail, LineageFocus};

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.active_detail_tab = DetailTab::Output;
            view.focus = LineageFocus::Content { scroll: 0 };
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Shown {
                    side: ContentSide::Output,
                    render: ContentRender::Text {
                        text: "{\n  \"key\": \"value\",\n  \"count\": 42\n}".to_string(),
                        pretty_printed: false,
                    },
                    bytes_fetched: 36,
                    truncated: false,
                },
            };
        }
        insta::assert_snapshot!("lineage_view_expanded_text_content", snap(&state));
    }

    #[test]
    fn snapshot_lineage_view_diff_mode_changed() {
        use crate::view::tracer::state::{AttributeDiffMode, ContentPane, EventDetail};

        let mut state = TracerState::new();
        seed_lineage_state(&mut state);
        if let TracerMode::Lineage(ref mut view) = state.mode {
            view.diff_mode = AttributeDiffMode::Changed;
            view.event_detail = EventDetail::Loaded {
                event: Box::new(make_lineage_detail(2)),
                content: ContentPane::Collapsed,
            };
        }
        insta::assert_snapshot!("lineage_view_diff_mode_changed", snap(&state));
    }
}
