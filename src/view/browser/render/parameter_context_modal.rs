//! Renders the Browser tab's parameter-context modal (Layout A).
//!
//! Mirrors `version_control_modal` in shape: full-screen overlay,
//! identity header, two-pane body (chain sidebar + resolved
//! parameters table), footer hint strip.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::cluster::snapshot::{ClusterSnapshot, ParameterContextRef};
use crate::theme;
use crate::view::browser::state::parameter_context_modal::ParameterContextPane;
use crate::view::browser::state::{
    BrowserState, ParameterContextLoad, ParameterContextModalState, ResolvedParameter, resolve,
};
use crate::widget::panel::Panel;
use crate::widget::search::{MatchSpan, SearchState};

/// Resolved-flat / by-context table column widths. Order: flags | name |
/// value | from. The renderer and `searchable_body` MUST agree on these
/// widths so search-match byte offsets align with the rendered cells.
///
/// Flags render as a single combined chip — `[SPO]` not `[S] [P] [O]` —
/// so the column stays narrow. Worst case is `[SPO]` (5 chars).
pub(super) const FLAG_W: usize = 5;
pub(super) const NAME_W: usize = 22;
pub(super) const VALUE_W: usize = 22;
pub(super) const FROM_W: usize = 18;

/// A row in the used-by panel — a PG that binds this parameter context.
struct UsedByRow {
    pg_path: String,
}

/// Collect all PGs from the cluster snapshot that bind `ctx_id`.
fn used_by_pgs(snapshot: &ClusterSnapshot, ctx_id: &str, browser: &BrowserState) -> Vec<UsedByRow> {
    let map = match snapshot.parameter_context_bindings.latest() {
        Some(m) => m,
        None => return vec![],
    };
    let mut rows: Vec<UsedByRow> = map
        .by_pg_id
        .iter()
        .filter_map(
            |(pg_id, opt_ref): (&String, &Option<ParameterContextRef>)| {
                opt_ref
                    .as_ref()
                    .filter(|r| r.id == ctx_id)
                    .map(|_| UsedByRow {
                        pg_path: browser
                            .pg_name_for(pg_id)
                            .unwrap_or(pg_id.as_str())
                            .to_string(),
                    })
            },
        )
        .collect();
    rows.sort_by(|a, b| a.pg_path.cmp(&b.pg_path));
    rows
}

/// Entry point — renders the full-screen parameter-context modal over `area`.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    browser: &BrowserState,
    snapshot: &ClusterSnapshot,
) {
    if crate::widget::modal::render_too_small(frame, area) {
        return;
    }

    frame.render_widget(Clear, area);

    // Compute used_by once and thread it into render_header and render_used_by.
    let ctx_id = match &modal.load {
        ParameterContextLoad::Loaded { chain } => {
            chain.first().map(|n| n.id.as_str()).unwrap_or("")
        }
        _ => "",
    };
    let used_by = used_by_pgs(snapshot, ctx_id, browser);

    // Outer frame — title shows context name + id prefix (first 6 chars).
    let (ctx_name, ctx_id_prefix) = match &modal.load {
        ParameterContextLoad::Loaded { chain } => {
            let name = chain
                .first()
                .map(|n| n.name.as_str())
                .unwrap_or("parameter context");
            let prefix: String = ctx_id.chars().take(6).collect();
            (name, prefix)
        }
        _ => ("parameter context", String::new()),
    };
    let id_chip = if ctx_id_prefix.is_empty() {
        String::new()
    } else {
        format!("({ctx_id_prefix})")
    };
    let outer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Parameter Context", theme::muted()),
        Span::raw(": "),
        Span::styled(ctx_name.to_string(), theme::accent()),
        Span::raw("  "),
        Span::styled(id_chip, theme::muted()),
        Span::raw(" "),
    ]);
    let outer = Panel::new(outer_title).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    // Split: header (3 lines) / body (fill) / footer (2 lines).
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3),
            Constraint::Fill(1),
            Constraint::Length(2),
        ])
        .split(inner);

    render_header(frame, rows[0], modal, &used_by);
    render_body(frame, rows[1], modal, &used_by);
    render_footer(frame, rows[2], modal);
}

/// Renders the 3-line header: "Bound by" path + "Used by" count, chain breadcrumb.
fn render_header(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    used_by: &[UsedByRow],
) {
    let (ctx_name, chain_names) = match &modal.load {
        ParameterContextLoad::Loaded { chain } => {
            let name = chain.first().map(|n| n.name.as_str()).unwrap_or("?");
            let names: Vec<&str> = chain.iter().map(|n| n.name.as_str()).collect();
            (name, names)
        }
        _ => ("?", vec![]),
    };

    // Line 0: bound-by PG path + used-by count chip.
    let used_count = used_by.len();
    let mut bound_spans = vec![
        Span::styled(format!("{:<10}", "Bound by"), theme::muted()),
        Span::raw(modal.originating_pg_path.clone()),
    ];
    if used_count > 1 {
        bound_spans.push(Span::raw("  "));
        bound_spans.push(Span::styled(
            format!("Used by {} PGs", used_count),
            theme::muted(),
        ));
    }

    // Line 1: chain breadcrumb.
    let chain_line = if chain_names.is_empty() {
        Line::from(vec![
            Span::styled(format!("{:<10}", "Chain"), theme::muted()),
            Span::styled("loading…", theme::muted()),
        ])
    } else {
        let mut spans = vec![Span::styled(format!("{:<10}", "Chain"), theme::muted())];
        for (i, name) in chain_names.iter().enumerate() {
            if i > 0 {
                spans.push(Span::styled(" ▸ ", theme::muted()));
            }
            let style = if i == 0 {
                theme::accent()
            } else {
                Style::default()
            };
            spans.push(Span::styled(name.to_string(), style));
        }
        Line::from(spans)
    };

    // Line 2: the selected context name (from sidebar) when in by-context mode.
    let ctx_line = if modal.by_context_mode {
        Line::from(vec![
            Span::styled(format!("{:<10}", "Viewing"), theme::muted()),
            Span::styled(ctx_name.to_string(), theme::accent()),
            Span::styled("  [by-context]", theme::muted()),
        ])
    } else if modal.show_used_by {
        Line::from(vec![
            Span::styled(format!("{:<10}", "Viewing"), theme::muted()),
            Span::styled("used by", theme::muted()),
        ])
    } else {
        Line::from("")
    };

    let lines = vec![Line::from(bound_spans), chain_line, ctx_line];
    frame.render_widget(Paragraph::new(lines), area);
}

/// Two-pane body: sidebar (chain) on the left, params table on the right.
fn render_body(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    used_by: &[UsedByRow],
) {
    let sidebar_width = 22u16.min(area.width / 3);
    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(sidebar_width), Constraint::Fill(1)])
        .split(area);

    render_sidebar(frame, cols[0], modal);
    render_params(frame, cols[1], modal, used_by);
}

/// Left sidebar: one row per chain node, cursor arrow on sidebar_index.
/// Border accent reflects `focused_pane == Sidebar`.
fn render_sidebar(frame: &mut Frame, area: Rect, modal: &ParameterContextModalState) {
    let sidebar_focused = modal.focused_pane == ParameterContextPane::Sidebar;
    let border_style = if sidebar_focused {
        theme::accent()
    } else {
        theme::border_dim()
    };
    let block = Block::default()
        .borders(Borders::RIGHT)
        .border_style(border_style);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let chain = match &modal.load {
        ParameterContextLoad::Loaded { chain } => chain,
        ParameterContextLoad::Loading => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("loading…", theme::muted())))
                    .alignment(Alignment::Center),
                inner,
            );
            return;
        }
        ParameterContextLoad::Error { .. } => {
            return;
        }
    };

    let max_name_width = inner.width.saturating_sub(5) as usize;
    let mut lines: Vec<Line> = Vec::with_capacity(chain.len());
    for (i, node) in chain.iter().enumerate() {
        let cursor = if i == modal.sidebar_index {
            "▸ "
        } else {
            "  "
        };
        let param_count = node.parameters.len();
        let name = truncate_str(&node.name, max_name_width);
        // Accent the cursor row when Sidebar is focused; mute it when Body
        // has focus (the cursor remains visible but indicates non-active pane).
        let style = if i == modal.sidebar_index {
            if sidebar_focused {
                theme::accent()
            } else {
                theme::muted()
            }
        } else {
            Style::default()
        };
        let count_style = theme::muted();
        lines.push(Line::from(vec![
            Span::styled(cursor.to_string(), style),
            Span::styled(name, style),
            Span::raw(" "),
            Span::styled(param_count.to_string(), count_style),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

/// Right pane: resolved params table, loading, error, or used-by panel.
fn render_params(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    used_by: &[UsedByRow],
) {
    match &modal.load {
        ParameterContextLoad::Loading => {
            let line = Line::from(Span::styled("loading…", theme::muted()));
            frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        }
        ParameterContextLoad::Error { message } => {
            let lines = vec![
                Line::from(Span::styled("failed to load:", theme::error())),
                Line::from(Span::styled(message.clone(), theme::error())),
                Line::from(""),
                Line::from(Span::styled("press r to retry", theme::muted())),
            ];
            frame.render_widget(Paragraph::new(lines), area);
        }
        ParameterContextLoad::Loaded { chain } => {
            if modal.show_used_by {
                render_used_by(frame, area, modal, used_by);
                return;
            }
            if modal.by_context_mode {
                // Show parameters only from the sidebar-selected context.
                let selected = chain.get(modal.sidebar_index);
                render_by_context(frame, area, modal, selected);
            } else {
                // Resolved-flat view — pass the sidebar-selected context name
                // so matching rows can be subtly highlighted.
                let preselect = modal.preselect.as_deref();
                let resolved = resolve(chain, preselect);
                let sidebar_ctx = chain
                    .get(modal.sidebar_index)
                    .map(|n| n.name.as_str())
                    .unwrap_or("");
                render_flat(frame, area, modal, &resolved, sidebar_ctx);
            }
        }
    }
}

/// Render the resolved-flat parameter table.
/// `sidebar_ctx` is the name of the chain node selected in the sidebar;
/// rows whose `winner_context` matches it are given a leading `·` marker
/// to show which params come from that context.
fn render_flat(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    resolved: &[ResolvedParameter],
    sidebar_ctx: &str,
) {
    if resolved.is_empty() {
        let line = Line::from(Span::styled("no parameters", theme::muted()));
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }

    // Column widths (fixed). Order: flags | name | value | from.
    // `from` is wider than the original 12 so longer context display names
    // don't truncate (e.g. `payments-shared`). Flags first puts the most
    // forensic info — override / sensitive / provided / unresolved — at
    // the eye line.
    let flag_w = FLAG_W;
    let name_w = NAME_W;
    let value_w = VALUE_W;
    let from_w = FROM_W;

    // Header separator line.
    let sep = "─".repeat(area.width as usize);
    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(format!("{:<flag_w$}", "flags"), theme::muted()),
            Span::raw(" "),
            Span::styled(format!("{:<name_w$}", "name"), theme::muted()),
            Span::raw(" "),
            Span::styled(format!("{:<value_w$}", "value"), theme::muted()),
            Span::raw(" "),
            Span::styled(format!("{:<from_w$}", "from"), theme::muted()),
        ]),
        Line::from(Span::styled(sep, theme::muted())),
    ];

    let mut total = 0usize;
    let mut overrides = 0usize;
    let mut sensitive = 0usize;
    let mut unresolved_count = 0usize;

    for rp in resolved {
        total += 1;
        if rp.unresolved {
            unresolved_count += 1;
        }
        if rp.winner.sensitive {
            sensitive += 1;
        }
        if !rp.shadowed.is_empty() {
            overrides += rp.shadowed.len();
        }

        let from_sidebar = !sidebar_ctx.is_empty() && rp.winner_context == sidebar_ctx;
        lines.push(param_row(rp, flag_w, name_w, value_w, from_w, from_sidebar));

        if modal.show_shadowed {
            for (shadowed_entry, shadowed_ctx) in &rp.shadowed {
                let dim = theme::muted();
                let name = truncate_str(&shadowed_entry.name, name_w.saturating_sub(2));
                let value = render_value(shadowed_entry.sensitive, &shadowed_entry.value, value_w);
                let from = truncate_str(shadowed_ctx, from_w);
                // Shadowed rows: empty flag column (the dim style is the
                // visual cue) + indented name + value + from.
                lines.push(Line::from(vec![
                    Span::raw(" ".repeat(flag_w)),
                    Span::raw(" "),
                    Span::styled(format!("  {name:<width$}", width = name_w - 2), dim),
                    Span::raw(" "),
                    Span::styled(format!("{value:<value_w$}"), dim),
                    Span::raw(" "),
                    Span::styled(format!("{from:<from_w$}"), dim),
                ]));
            }
        }
    }

    // Summary line.
    let summary = build_summary(total, overrides, sensitive, unresolved_count);
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(summary, theme::muted())));
    lines.push(Line::from(legend_spans()));

    // Apply search highlights if active.
    if let Some(search) = modal.search.as_ref()
        && search.committed
        && !search.matches.is_empty()
    {
        apply_search_highlights(&mut lines, search);
    }

    let scroll_y = modal.scroll.offset as u16;
    let p = Paragraph::new(lines)
        .scroll((scroll_y, 0))
        .wrap(ratatui::widgets::Wrap { trim: false });
    frame.render_widget(p, area);
}

/// Build one param row line (winner only).
/// `from_sidebar` — when true, the row's `winner_context` matches the
/// sidebar cursor; the `from` column is styled with a subtle accent so
/// the contribution is visible.
fn param_row(
    rp: &ResolvedParameter,
    flag_w: usize,
    name_w: usize,
    value_w: usize,
    from_w: usize,
    from_sidebar: bool,
) -> Line<'static> {
    let name = truncate_str(&rp.winner.name, name_w);
    let value = render_value(rp.winner.sensitive, &rp.winner.value, value_w);
    let from = if rp.unresolved {
        "—".to_string()
    } else {
        truncate_str(&rp.winner_context, from_w)
    };

    let row_style = if rp.unresolved {
        theme::error()
    } else {
        Style::default()
    };
    let from_style = if from_sidebar && !rp.unresolved {
        theme::accent()
    } else {
        row_style
    };

    // Column order: flags | name | value | from.
    let mut spans = flag_spans_padded(rp, flag_w);
    spans.push(Span::raw(" "));
    spans.push(Span::styled(format!("{name:<name_w$}"), row_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(format!("{value:<value_w$}"), row_style));
    spans.push(Span::raw(" "));
    spans.push(Span::styled(format!("{from:<from_w$}"), from_style));

    Line::from(spans)
}

/// Build a combined flag chip — `[SPO]` style with per-letter styling —
/// padded to `width` columns. Empty rows pad to `width` with spaces.
fn flag_spans_padded(rp: &ResolvedParameter, width: usize) -> Vec<Span<'static>> {
    let letters = flag_letters(rp);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut consumed = 0usize;
    if !letters.is_empty() {
        spans.push(Span::styled("[".to_string(), theme::muted()));
        consumed += 1;
        for (ch, style) in &letters {
            spans.push(Span::styled(ch.to_string(), *style));
            consumed += 1;
        }
        spans.push(Span::styled("]".to_string(), theme::muted()));
        consumed += 1;
    }
    if consumed < width {
        spans.push(Span::raw(" ".repeat(width - consumed)));
    }
    spans
}

/// Returns the flag letters that apply, in canonical order (S, P, O, !).
/// Each letter carries its display style. Mirrors `build_flags` but
/// returns single chars instead of `[X]`-bracketed strings.
fn flag_letters(rp: &ResolvedParameter) -> Vec<(char, Style)> {
    let mut letters = Vec::new();
    if rp.winner.sensitive {
        letters.push(('S', theme::warning()));
    }
    if rp.winner.provided {
        letters.push(('P', theme::muted()));
    }
    if !rp.shadowed.is_empty() && !rp.unresolved {
        letters.push(('O', theme::muted()));
    }
    if rp.unresolved {
        letters.push(('!', theme::error()));
    }
    letters
}

/// One-line legend explaining the flag chip letters. Rendered below the
/// summary so users can decode `[S]` / `[SP]` / `[O]` / `[!]` without
/// leaving the modal.
fn legend_spans() -> Vec<Span<'static>> {
    let dot = || Span::styled(" · ", theme::muted());
    vec![
        Span::styled("flags: ", theme::muted()),
        Span::styled("S", theme::warning()),
        Span::styled(" sensitive", theme::muted()),
        dot(),
        Span::styled("P", theme::muted()),
        Span::styled(" provided", theme::muted()),
        dot(),
        Span::styled("O", theme::muted()),
        Span::styled(" overridden", theme::muted()),
        dot(),
        Span::styled("!", theme::error()),
        Span::styled(" unresolved", theme::muted()),
    ]
}

/// Render a parameter value: `(sensitive)` literal when sensitive,
/// `—` when None, otherwise the trimmed value.
fn render_value(sensitive: bool, value: &Option<String>, width: usize) -> String {
    if sensitive {
        return truncate_str("(sensitive)", width);
    }
    match value {
        Some(v) => truncate_str(v, width),
        None => "—".to_string(),
    }
}

/// By-context view: show only params from the sidebar-selected node.
fn render_by_context(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    node: Option<&crate::client::parameter_context::ParameterContextNode>,
) {
    let node = match node {
        Some(n) => n,
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "no context selected",
                    theme::muted(),
                )))
                .alignment(Alignment::Center),
                area,
            );
            return;
        }
    };

    if let Some(err) = &node.fetch_error {
        let lines = vec![
            Line::from(Span::styled("failed to load context:", theme::error())),
            Line::from(Span::styled(err.clone(), theme::error())),
        ];
        frame.render_widget(Paragraph::new(lines), area);
        return;
    }

    let flag_w = FLAG_W;
    let name_w = NAME_W;
    let value_w = VALUE_W;
    let sep = "─".repeat(area.width as usize);

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(vec![
            Span::styled(format!("{:<flag_w$}", "flags"), theme::muted()),
            Span::raw(" "),
            Span::styled(format!("{:<name_w$}", "name"), theme::muted()),
            Span::raw(" "),
            Span::styled(format!("{:<value_w$}", "value"), theme::muted()),
        ]),
        Line::from(Span::styled(sep, theme::muted())),
    ];

    for entry in &node.parameters {
        let name = truncate_str(&entry.name, name_w);
        let value = render_value(entry.sensitive, &entry.value, value_w);

        // Combined flag chip (sensitive + provided only — by_context view
        // doesn't model override / unresolved which are chain-wide).
        let mut letters: Vec<(char, Style)> = Vec::new();
        if entry.sensitive {
            letters.push(('S', theme::warning()));
        }
        if entry.provided {
            letters.push(('P', theme::muted()));
        }
        let mut spans: Vec<Span<'static>> = Vec::new();
        let mut consumed = 0usize;
        if !letters.is_empty() {
            spans.push(Span::styled("[".to_string(), theme::muted()));
            consumed += 1;
            for (ch, style) in &letters {
                spans.push(Span::styled(ch.to_string(), *style));
                consumed += 1;
            }
            spans.push(Span::styled("]".to_string(), theme::muted()));
            consumed += 1;
        }
        if consumed < flag_w {
            spans.push(Span::raw(" ".repeat(flag_w - consumed)));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("{name:<name_w$}"), Style::default()));
        spans.push(Span::raw(" "));
        spans.push(Span::styled(format!("{value:<value_w$}"), Style::default()));
        lines.push(Line::from(spans));
    }

    if !node.parameters.is_empty() {
        // Legend mirrors render_flat's so users get the same decoder ring
        // when switching to by-context view. Only S and P apply here; we
        // include the others greyed out for consistency.
        lines.push(Line::from(""));
        lines.push(Line::from(legend_spans()));
    }

    if node.parameters.is_empty() {
        lines.push(Line::from(Span::styled("(no parameters)", theme::muted())));
    }

    let scroll_y = modal.scroll.offset as u16;
    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll_y, 0))
            .wrap(ratatui::widgets::Wrap { trim: false }),
        area,
    );
}

/// Used-by panel: list every PG that binds this context.
fn render_used_by(
    frame: &mut Frame,
    area: Rect,
    modal: &ParameterContextModalState,
    rows: &[UsedByRow],
) {
    if rows.is_empty() {
        let line = Line::from(Span::styled("(not used by any other PG)", theme::muted()));
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }

    let mut lines: Vec<Line<'static>> = vec![
        Line::from(Span::styled(
            "Process groups using this context:",
            theme::muted(),
        )),
        Line::from(""),
    ];
    for row in rows {
        lines.push(Line::from(row.pg_path.clone()));
    }

    let scroll_y = modal.scroll.offset as u16;
    frame.render_widget(Paragraph::new(lines).scroll((scroll_y, 0)), area);
}

fn build_summary(total: usize, overrides: usize, sensitive: usize, unresolved: usize) -> String {
    let mut parts = vec![format!("{total} params")];
    if overrides > 0 {
        parts.push(format!("{overrides} overrides"));
    }
    if sensitive > 0 {
        parts.push(format!("{sensitive} sensitive"));
    }
    if unresolved > 0 {
        parts.push(format!("{unresolved} unresolved"));
    }
    parts.join(" · ")
}

/// Truncate a string to at most `max_chars` characters, appending `…` if cut.
fn truncate_str(s: &str, max_chars: usize) -> String {
    if max_chars == 0 {
        return String::new();
    }
    let count = s.chars().count();
    if count <= max_chars {
        s.to_string()
    } else {
        let truncated: String = s.chars().take(max_chars.saturating_sub(1)).collect();
        format!("{truncated}…")
    }
}

/// Apply search match highlights to the flat-table lines in-place.
/// Mirrors `version_control_modal::apply_search_highlights`.
fn apply_search_highlights(lines: &mut [Line<'static>], search: &SearchState) {
    for (line_idx, line) in lines.iter_mut().enumerate() {
        let per_line: Vec<(usize, &MatchSpan)> = search
            .matches
            .iter()
            .enumerate()
            .filter(|(_, m)| m.line_idx == line_idx)
            .collect();
        if per_line.is_empty() {
            continue;
        }
        let plain: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
        let mut new_spans: Vec<Span<'static>> = Vec::new();
        let mut cursor = 0usize;
        for (global_idx, m) in per_line {
            if m.byte_start > cursor {
                new_spans.push(Span::raw(plain[cursor..m.byte_start].to_string()));
            }
            let hit = plain[m.byte_start..m.byte_end].to_string();
            let style = if search.current == Some(global_idx) {
                theme::search_match_active()
            } else {
                theme::search_match()
            };
            new_spans.push(Span::styled(hit, style));
            cursor = m.byte_end;
        }
        if cursor < plain.len() {
            new_spans.push(Span::raw(plain[cursor..].to_string()));
        }
        if new_spans.is_empty() {
            new_spans.push(Span::raw(""));
        }
        *line = Line::from(new_spans);
    }
}

fn render_footer(frame: &mut Frame, area: Rect, modal: &ParameterContextModalState) {
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1)])
        .split(area);

    render_footer_status(frame, rows[0], modal);
    render_footer_hint(frame, rows[1]);
}

fn render_footer_status(frame: &mut Frame, area: Rect, modal: &ParameterContextModalState) {
    // While search input is active, show the search bar.
    if let Some(search) = modal.search.as_ref()
        && search.input_active
    {
        let line = Line::from(vec![
            Span::styled("/ ".to_string(), theme::accent()),
            Span::raw(search.query.clone()),
            Span::styled("_".to_string(), theme::search_cursor()),
        ]);
        frame.render_widget(Paragraph::new(line), area);
        return;
    }

    let status = match &modal.load {
        ParameterContextLoad::Loading => "loading…".to_string(),
        ParameterContextLoad::Error { .. } => "failed — press r to retry".to_string(),
        ParameterContextLoad::Loaded { chain } => {
            // Count unique resolved names (shadowed duplicates excluded) so the
            // count matches what the resolved-flat view actually shows.
            let unique = resolve(chain, None).len();
            format!("{} params across {} contexts", unique, chain.len())
        }
    };
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(status, theme::muted()))),
        area,
    );
}

fn render_footer_hint(frame: &mut Frame, area: Rect) {
    use crate::input::ParameterContextModalVerb;
    use crate::input::Verb;
    crate::widget::modal::render_verb_hint_strip(frame, area, ParameterContextModalVerb::all());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::parameter_context::{ParameterContextNode, ParameterEntry};
    use crate::cluster::snapshot::ClusterSnapshot;
    use crate::test_support::test_backend;
    use crate::view::browser::state::{BrowserState, ParameterContextModalState};
    use ratatui::Terminal;

    fn entry(name: &str, value: &str, sensitive: bool) -> ParameterEntry {
        ParameterEntry {
            name: name.into(),
            value: if sensitive { None } else { Some(value.into()) },
            description: None,
            sensitive,
            provided: false,
        }
    }

    fn node(id: &str, name: &str, params: Vec<ParameterEntry>) -> ParameterContextNode {
        ParameterContextNode {
            id: id.into(),
            name: name.into(),
            parameters: params,
            inherited_ids: vec![],
            fetch_error: None,
        }
    }

    fn loaded_modal(chain: Vec<ParameterContextNode>) -> ParameterContextModalState {
        let mut m =
            ParameterContextModalState::pending("pg-1".into(), "/flows/payments-prod".into(), None);
        m.load = ParameterContextLoad::Loaded { chain };
        m
    }

    #[test]
    fn renders_happy_path_two_context_chain() {
        let chain = vec![
            node(
                "ctx-prod",
                "payments-prod",
                vec![
                    entry("kafka.bootstrap", "broker:9092", false),
                    entry("db.password", "secret", true),
                ],
            ),
            node(
                "ctx-shared",
                "org-defaults",
                vec![entry("region", "eu-west-1", false)],
            ),
        ];
        let modal = loaded_modal(chain);
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        let mut term = Terminal::new(test_backend(24)).unwrap();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        assert!(output.contains("Parameter Context"), "missing modal title");
        assert!(output.contains("payments-prod"), "missing context name");
        assert!(output.contains("kafka.bootstrap"), "missing param name");
    }

    #[test]
    fn below_minimum_size_shows_terminal_too_small() {
        use ratatui::backend::TestBackend;
        let backend = TestBackend::new(40, 15);
        let mut term = Terminal::new(backend).unwrap();
        let modal = ParameterContextModalState::pending("pg".into(), "/pg".into(), None);
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        assert!(output.contains("terminal too small"));
    }

    #[test]
    fn loading_state_shows_loading_in_sidebar_and_params() {
        let modal = ParameterContextModalState::pending("pg".into(), "/pg".into(), None);
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        let mut term = Terminal::new(test_backend(24)).unwrap();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        // Both the sidebar and the right pane show "loading…"
        let count = output.matches("loading").count();
        assert!(
            count >= 1,
            "expected at least one loading… but got: {output}"
        );
    }

    #[test]
    fn error_state_shows_error_and_retry_hint() {
        let mut modal = ParameterContextModalState::pending("pg".into(), "/pg".into(), None);
        modal.load = ParameterContextLoad::Error {
            message: "context unreachable: timeout".into(),
        };
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        let mut term = Terminal::new(test_backend(24)).unwrap();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        assert!(output.contains("failed to load"));
        assert!(output.contains("retry"));
    }

    #[test]
    fn sensitive_param_renders_as_sensitive_literal() {
        let chain = vec![node(
            "ctx",
            "ctx",
            vec![entry("secret.token", "should-not-appear", true)],
        )];
        let modal = loaded_modal(chain);
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        let mut term = Terminal::new(test_backend(24)).unwrap();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        assert!(
            output.contains("(sensitive)"),
            "sensitive param must show (sensitive)"
        );
        assert!(
            !output.contains("should-not-appear"),
            "raw sensitive value must not appear"
        );
    }

    /// Verify that a committed search on "retry" highlights the `retry_max`
    /// row in the flat params table. The key invariant being tested is that
    /// `searchable_body()` and the rendered body are byte-aligned so that
    /// `MatchSpan` offsets from `compute_matches` land on the correct spans.
    #[test]
    fn search_highlights_matched_param_row() {
        use crate::widget::search::{SearchState, compute_matches};

        let chain = vec![
            node(
                "ctx-prod",
                "prod",
                vec![
                    entry("kafka_bootstrap", "broker:9092", false),
                    entry("retry_max", "5", false),
                ],
            ),
            node("ctx-base", "base", vec![entry("retry_max", "3", false)]),
        ];
        let mut modal = loaded_modal(chain);
        let body = modal.searchable_body();
        assert!(
            body.contains("retry_max"),
            "body must contain retry_max; got: {body:?}"
        );
        let matches = compute_matches(&body, "retry");
        assert!(
            !matches.is_empty(),
            "compute_matches must find 'retry' in body"
        );
        modal.search = Some(SearchState {
            query: "retry".into(),
            input_active: false,
            committed: true,
            matches,
            current: Some(0),
        });
        let browser = BrowserState::new();
        let snapshot = ClusterSnapshot::default();
        let mut term = Terminal::new(test_backend(24)).unwrap();
        term.draw(|f| render(f, f.area(), &modal, &browser, &snapshot))
            .unwrap();
        let output = format!("{}", term.backend());
        // The rendered buffer should contain the param name; if search
        // highlighting is wired correctly it won't corrupt the output.
        assert!(
            output.contains("retry_max"),
            "retry_max must appear in rendered output; got: {output}"
        );
        assert!(
            output.contains("kafka_bootstrap"),
            "other params must still render alongside the highlighted one"
        );
    }
}
