//! Full-screen peek modal renderer. Mirrors the layout shape of
//! `parameter_context_modal` and `version_control_modal`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table};

use crate::theme;
use crate::timestamp::format_age_secs;
use crate::view::browser::state::queue_listing::QueueListingPeekState;
use crate::widget::panel::Panel;
use crate::widget::search::SearchState;

/// Render the full-screen flowfile peek modal into `area`.
///
/// - Identity fields are shown immediately from `state.identity`.
/// - Attributes table renders once `state.attrs` is populated by the worker.
/// - Error and loading chips appear in the panel's right title until data arrives.
pub fn render_peek_modal(f: &mut Frame<'_>, area: Rect, state: &QueueListingPeekState) {
    if crate::widget::modal::render_too_small(f, area) {
        return;
    }

    // Clear the underlying buffer so the modal overlay doesn't leak the
    // tree / detail-pane content drawn by the regular browser render
    // below it. Mirrors the action-history / parameter-context modal
    // pattern.
    f.render_widget(Clear, area);

    let title = build_title_left(state);
    let chips = build_title_chips(state);
    let block = Panel::new(title).focused(true).right(chips).into_block();
    let inner = block.inner(area);
    f.render_widget(block, area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(9), // identity header
            Constraint::Length(1), // separator
            Constraint::Min(0),    // attrs body
            Constraint::Length(1), // hint strip
        ])
        .split(inner);

    render_identity(f, chunks[0], state);
    render_separator(f, chunks[1]);
    render_attrs(f, chunks[2], state);

    // The hint strip and the search prompt share the same bottom row.
    // While the user is typing a query, render only the prompt — the
    // Paragraph widget clears the row, so the hint text is fully
    // replaced rather than partially overwritten.
    let prompt_active = state
        .search
        .as_ref()
        .map(|s| s.input_active)
        .unwrap_or(false);
    if prompt_active {
        let query = state
            .search
            .as_ref()
            .map(|s| s.query.clone())
            .unwrap_or_default();
        let prompt = Line::from(vec![
            Span::styled("/", theme::accent()),
            Span::raw(query),
            Span::styled("_", theme::muted()),
        ]);
        f.render_widget(Paragraph::new(prompt), chunks[3]);
    } else {
        render_hints(f, chunks[3]);
    }
}

fn build_title_left(state: &QueueListingPeekState) -> Line<'static> {
    let short: String = state.uuid.chars().take(8).collect();
    Line::from(Span::raw(format!("Flowfile {short}…")))
}

fn build_title_chips(state: &QueueListingPeekState) -> Line<'static> {
    if state.error.is_some() {
        // The full NiFi error is rendered in the status-line banner
        // (post_error with detail). Keep the modal chip terse so it
        // doesn't bleed past the panel border on long messages.
        return Line::from(Span::styled("[error]", theme::warning()));
    }
    if state.attrs.is_none() {
        return Line::from(Span::styled("[loading…]", theme::muted()));
    }
    let n = state.attrs.as_ref().map(|a| a.len()).unwrap_or(0);
    Line::from(Span::raw(format!("[{n} attrs]")))
}

fn render_identity(f: &mut Frame<'_>, area: Rect, state: &QueueListingPeekState) {
    let id = &state.identity;
    let claim = id
        .content_claim
        .as_ref()
        .map(|c| {
            format!(
                "{} / {} / {} / offset {} / {}",
                c.container,
                c.section,
                c.identifier,
                c.offset,
                crate::bytes::format_bytes(c.file_size),
            )
        })
        .unwrap_or_else(|| "—".to_string());
    let lines = vec![
        kv_line("uuid", &id.uuid),
        kv_line("filename", id.filename.as_deref().unwrap_or("—")),
        kv_line("size", &crate::bytes::format_bytes(id.size)),
        kv_line("mime_type", id.mime_type.as_deref().unwrap_or("—")),
        kv_line("content claim", &claim),
        kv_line("cluster node", id.cluster_node_id.as_deref().unwrap_or("—")),
        kv_line(
            "lineage age",
            &format_age_secs(id.lineage_duration.as_secs()),
        ),
        kv_line("penalized", if id.penalized { "yes" } else { "no" }),
    ];
    f.render_widget(Paragraph::new(lines), area);
}

fn kv_line(label: &str, value: &str) -> Line<'static> {
    Line::from(vec![
        Span::styled(format!("  {label:<15}"), theme::muted()),
        Span::raw(value.to_string()),
    ])
}

fn render_separator(f: &mut Frame<'_>, area: Rect) {
    let s = "─".repeat(area.width as usize);
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(s, theme::border_dim()))),
        area,
    );
}

fn render_attrs(f: &mut Frame<'_>, area: Rect, state: &QueueListingPeekState) {
    let Some(attrs) = state.attrs.as_ref() else {
        if state.error.is_some() {
            let line = Line::from(Span::styled(
                "failed to fetch attributes — Esc to close",
                theme::warning(),
            ));
            f.render_widget(Paragraph::new(line), area);
            return;
        }
        let line = Line::from(Span::styled("…", theme::muted()));
        f.render_widget(Paragraph::new(line), area);
        return;
    };
    let header = Row::new(vec![Cell::from("key"), Cell::from("value")]).style(theme::muted());

    // Compute per-row highlight styles when search has committed
    // matches. The body's row coordinates align with `attrs` insertion
    // order via `searchable_body` — see that helper for the cell-width
    // contract.
    let highlight_styles: std::collections::HashMap<usize, Style> = state
        .search
        .as_ref()
        .filter(|s| s.committed && !s.matches.is_empty())
        .map(|search| compute_row_highlights(attrs, search))
        .unwrap_or_default();

    let rows = attrs.iter().enumerate().map(|(i, (k, v))| {
        let style = highlight_styles.get(&i).copied().unwrap_or_default();
        Row::new(vec![Cell::from(k.clone()), Cell::from(v.clone())]).style(style)
    });

    let table = Table::new(rows, [Constraint::Length(40), Constraint::Min(20)]).header(header);
    f.render_widget(table, area);
}

fn compute_row_highlights(
    attrs: &std::collections::BTreeMap<String, String>,
    search: &SearchState,
) -> std::collections::HashMap<usize, Style> {
    // `searchable_body` emits one logical line per attr in `attrs`
    // insertion order. compute_matches returns MatchSpan with
    // line_idx aligned to that ordering. So a MatchSpan with
    // line_idx == row_index means "row N has a match"; the current
    // match (if any) gets bold highlighting on top.
    let mut out = std::collections::HashMap::new();
    let current_line = search
        .current
        .and_then(|c| search.matches.get(c).map(|m| m.line_idx));
    for span in &search.matches {
        if span.line_idx >= attrs.len() {
            continue;
        }
        let style = if Some(span.line_idx) == current_line {
            theme::accent().add_modifier(Modifier::BOLD)
        } else {
            theme::accent()
        };
        out.insert(span.line_idx, style);
    }
    out
}

fn render_hints(f: &mut Frame<'_>, area: Rect) {
    use crate::input::BrowserPeekVerb;
    use crate::input::Verb;
    crate::widget::modal::render_verb_hint_strip(f, area, BrowserPeekVerb::all());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::view::browser::state::queue_listing::{
        ContentClaimSummary, PeekIdentity, QueueListingPeekState,
    };
    use crate::widget::scroll::VerticalScrollState;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use std::collections::BTreeMap;
    use std::time::Duration;

    fn pending(uuid: &str) -> QueueListingPeekState {
        QueueListingPeekState {
            uuid: uuid.into(),
            queue_id: "q1".into(),
            cluster_node_id: None,
            identity: PeekIdentity {
                uuid: uuid.into(),
                filename: Some("a.parquet".into()),
                size: 1024 * 1024,
                mime_type: None,
                content_claim: None,
                cluster_node_id: None,
                lineage_duration: Duration::from_secs(60),
                penalized: false,
            },
            attrs: None,
            error: None,
            scroll: VerticalScrollState::default(),
            search: None,
            fetch_handle: None,
        }
    }

    fn render_to_string(width: u16, height: u16, state: &QueueListingPeekState) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| render_peek_modal(f, f.area(), state))
            .unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn renders_identity_immediately() {
        let p = pending("ff-aaaa");
        let out = render_to_string(80, 24, &p);
        assert!(out.contains("ff-aaaa"));
        assert!(out.contains("a.parquet"));
        assert!(
            out.contains("loading"),
            "loading chip while attrs == None:\n{out}"
        );
    }

    #[test]
    fn renders_attrs_when_loaded() {
        let mut p = pending("ff-aaaa");
        let mut attrs = BTreeMap::new();
        attrs.insert("record.count".into(), "1000".into());
        p.attrs = Some(attrs);
        p.identity.mime_type = Some("application/x-parquet".into());
        p.identity.content_claim = Some(ContentClaimSummary {
            container: "default".into(),
            section: "1234".into(),
            identifier: "abc".into(),
            offset: 0,
            file_size: 1024,
        });
        let out = render_to_string(80, 24, &p);
        assert!(out.contains("application/x-parquet"));
        assert!(out.contains("record.count"));
        assert!(out.contains("1000"));
        assert!(out.contains("default"));
    }

    #[test]
    fn renders_error_chip() {
        // The full NiFi error message is now surfaced via the
        // status-line banner (`post_error` in handle_browser_payload),
        // so the modal chip stays terse — just `[error]`. The body
        // still shows "failed to fetch attributes — Esc to close"
        // for in-modal context.
        let mut p = pending("ff-aaaa");
        p.error = Some("404 The FlowFile is no longer in the active queue".into());
        let out = render_to_string(80, 24, &p);
        assert!(out.contains("[error]"), "expected terse error chip:\n{out}");
        assert!(
            out.contains("failed to fetch attributes"),
            "expected body fallback:\n{out}",
        );
    }

    #[test]
    fn renders_terminal_too_small() {
        let p = pending("ff-aaaa");
        let out = render_to_string(40, 10, &p);
        assert!(out.contains("terminal too small"));
    }

    #[test]
    fn committed_search_highlights_matched_attr_rows() {
        use crate::widget::search::{SearchState, compute_matches};

        let mut p = pending("ff-aaaa");
        let mut attrs = BTreeMap::new();
        attrs.insert("filename".into(), "sensor.parquet".into());
        attrs.insert("record.count".into(), "1000".into());
        p.attrs = Some(attrs);

        let body = p.searchable_body();
        let matches = compute_matches(&body, "sensor");
        assert!(!matches.is_empty(), "compute_matches must find 'sensor'");

        p.search = Some(SearchState {
            query: "sensor".into(),
            matches,
            current: Some(0),
            input_active: false,
            committed: true,
        });

        // Note: the buffer dump shows text, not styles. This assertion
        // confirms the renderer doesn't panic when search is active and
        // that matched rows still render their content. Style verification
        // would require inspecting cell styles via term.backend().buffer()
        // which is brittle under layout changes.
        let out = render_to_string(80, 24, &p);
        assert!(out.contains("filename"));
        assert!(out.contains("sensor.parquet"));
    }

    #[test]
    fn renders_search_prompt_when_input_active() {
        use crate::widget::search::SearchState;

        let mut p = pending("ff-aaaa");
        p.attrs = Some(BTreeMap::new());
        p.search = Some(SearchState {
            query: "abc".into(),
            matches: vec![],
            current: None,
            input_active: true,
            committed: false,
        });

        let out = render_to_string(80, 24, &p);
        assert!(out.contains("/abc"), "expected prompt overlay:\n{out}");
    }
}
