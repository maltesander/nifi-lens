//! Fuzzy-find modal backed by nucleo.
//!
//! The widget owns state, reducer helpers, and the modal overlay render.
//! The corpus (`FlowIndex`) is shared with the Browser tab and populated
//! by `apply_tree_snapshot`. This widget never touches the corpus directly
//! — it receives a borrow at match time and writes results into its own
//! `matches` field.

use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::client::NodeKind;
use crate::theme;
use crate::view::browser::state::{FlowIndex, FlowIndexEntry};

/// Drift sub-filter used by `QueryFilter::Drift`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriftFilter {
    /// Any non-`UpToDate` version-control state.
    Any,
    /// `Stale` or `LocallyModifiedAndStale`.
    Stale,
    /// `LocallyModified` or `LocallyModifiedAndStale`.
    Modified,
    /// `SyncFailure`.
    SyncErr,
}

/// Parsed leading-token filter from a fuzzy-find query string.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QueryFilter {
    /// No filter — all entries pass.
    None,
    /// Restrict corpus to a single `NodeKind`.
    Kind(NodeKind),
    /// Restrict corpus to PGs whose version-control state matches the drift sub-filter.
    Drift(DriftFilter),
}

impl QueryFilter {
    /// Returns `true` when `entry` passes through this filter.
    pub fn matches(self, entry: &FlowIndexEntry) -> bool {
        use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;
        match self {
            Self::None => true,
            Self::Kind(k) => entry.kind == k,
            Self::Drift(f) => {
                if entry.kind != NodeKind::ProcessGroup {
                    return false;
                }
                let Some(state) = entry.version_state else {
                    return false;
                };
                match f {
                    DriftFilter::Any => state != S::UpToDate,
                    DriftFilter::Stale => matches!(state, S::Stale | S::LocallyModifiedAndStale),
                    DriftFilter::Modified => {
                        matches!(state, S::LocallyModified | S::LocallyModifiedAndStale)
                    }
                    DriftFilter::SyncErr => state == S::SyncFailure,
                }
            }
        }
    }
}

#[derive(Debug)]
pub struct FuzzyFindState {
    /// Raw user input — the only field external callers mutate.
    pub query: String,
    /// Filter parsed from the leading token of `query`. Derived
    /// state, refreshed in `rebuild_matches`.
    pub filter: QueryFilter,
    /// Fuzzy needle with the kind/drift prefix stripped. Derived state,
    /// refreshed in `rebuild_matches`.
    pub effective_query: String,
    pub matches: Vec<MatchedEntry>,
    pub selected: usize,
}

#[derive(Debug, Clone)]
pub struct MatchedEntry {
    /// Index into `FlowIndex.entries`.
    pub index_entry: usize,
    pub score: u16,
    /// Matched character positions for highlight rendering.
    pub highlights: Vec<u32>,
}

/// Lower return value means higher display priority.
fn kind_priority(kind: NodeKind) -> u8 {
    match kind {
        NodeKind::Processor => 0,
        NodeKind::ProcessGroup => 1,
        NodeKind::ControllerService => 2,
        NodeKind::Connection => 3,
        NodeKind::InputPort => 4,
        NodeKind::OutputPort => 5,
        NodeKind::Folder(_) => 6,
    }
}

/// Fixed alias table mapping colon-prefixed tokens to `NodeKind`.
/// Keep lowercase — `parse_prefix` compares case-insensitively.
const KIND_ALIASES: &[(&str, NodeKind)] = &[
    (":proc", NodeKind::Processor),
    (":pg", NodeKind::ProcessGroup),
    (":cs", NodeKind::ControllerService),
    (":conn", NodeKind::Connection),
    (":in", NodeKind::InputPort),
    (":out", NodeKind::OutputPort),
];

/// Fixed alias table mapping colon-prefixed tokens to `DriftFilter`.
/// Keep lowercase — `parse_prefix` compares case-insensitively.
const DRIFT_ALIASES: &[(&str, DriftFilter)] = &[
    (":drift", DriftFilter::Any),
    (":stale", DriftFilter::Stale),
    (":modified", DriftFilter::Modified),
    (":syncerr", DriftFilter::SyncErr),
];

/// Parse a filter prefix off the start of `query`. Returns the resolved
/// `QueryFilter` (kind or drift, or `None`) and the remaining fuzzy needle.
///
/// Parsing rules:
/// - Only the first whitespace-separated token is consulted.
/// - Leading whitespace is tolerated and stripped.
/// - Matching is case-insensitive.
/// - An unknown `:token` (or any non-alias token) is treated as plain
///   query text — no filter, no stripping.
/// - `:proc` alone → kind set, empty needle.
/// - `:drift` alone → drift filter set, empty needle.
/// - Trailing whitespace in the returned needle is preserved (the user
///   may be mid-typing the next word).
pub(crate) fn parse_prefix(query: &str) -> (QueryFilter, String) {
    let trimmed = query.trim_start();
    // Split off the first whitespace-delimited token without allocating
    // a Vec<&str>.
    let (head, tail) = match trimmed.find(char::is_whitespace) {
        Some(ws) => (&trimmed[..ws], &trimmed[ws..]),
        None => (trimmed, ""),
    };
    for (alias, kind) in KIND_ALIASES {
        if head.eq_ignore_ascii_case(alias) {
            // Drop a single separator whitespace; preserve any trailing
            // characters (further whitespace + the user's in-progress
            // needle).
            let rest = tail
                .strip_prefix(|c: char| c.is_whitespace())
                .unwrap_or(tail);
            return (QueryFilter::Kind(*kind), rest.to_string());
        }
    }
    for (alias, drift) in DRIFT_ALIASES {
        if head.eq_ignore_ascii_case(alias) {
            let rest = tail
                .strip_prefix(|c: char| c.is_whitespace())
                .unwrap_or(tail);
            return (QueryFilter::Drift(*drift), rest.to_string());
        }
    }
    (QueryFilter::None, query.to_string())
}

impl Default for FuzzyFindState {
    fn default() -> Self {
        Self::new()
    }
}

impl FuzzyFindState {
    pub fn new() -> Self {
        Self {
            query: String::new(),
            filter: QueryFilter::None,
            effective_query: String::new(),
            matches: Vec::new(),
            selected: 0,
        }
    }

    /// Rebuild `matches` against `index`. Top 50 by score descending.
    /// An empty effective query matches everything in the (optionally
    /// kind-filtered) corpus.
    pub fn rebuild_matches(&mut self, index: &FlowIndex) {
        let (filter, effective_query) = parse_prefix(&self.query);
        self.filter = filter;
        self.effective_query = effective_query;

        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut query_buf = Vec::new();
        let lowered = self.effective_query.to_lowercase();
        let pattern = Utf32Str::new(&lowered, &mut query_buf);
        let mut results: Vec<MatchedEntry> = Vec::new();
        for (i, entry) in index.entries.iter().enumerate() {
            if !self.filter.matches(entry) {
                continue;
            }
            let mut haystack_buf = Vec::new();
            let hay = Utf32Str::new(&entry.haystack, &mut haystack_buf);
            let mut indices: Vec<u32> = Vec::new();
            if let Some(score) = matcher.fuzzy_indices(hay, pattern, &mut indices) {
                results.push(MatchedEntry {
                    index_entry: i,
                    score,
                    highlights: indices,
                });
            }
        }
        results.sort_by(|a, b| {
            b.score.cmp(&a.score).then_with(|| {
                let ka = index.entries[a.index_entry].kind;
                let kb = index.entries[b.index_entry].kind;
                kind_priority(ka).cmp(&kind_priority(kb))
            })
        });
        results.truncate(50);
        self.matches = results;
        if self.selected >= self.matches.len() {
            self.selected = 0;
        }
    }

    pub fn push_char(&mut self, ch: char) {
        self.query.push(ch);
    }

    pub fn pop_char(&mut self) {
        self.query.pop();
    }

    pub fn move_down(&mut self) {
        if !self.matches.is_empty() && self.selected + 1 < self.matches.len() {
            self.selected += 1;
        }
    }

    pub fn move_up(&mut self) {
        if self.selected > 0 {
            self.selected -= 1;
        }
    }

    pub fn selected_entry<'a>(&self, index: &'a FlowIndex) -> Option<&'a FlowIndexEntry> {
        self.matches
            .get(self.selected)
            .and_then(|m| index.entries.get(m.index_entry))
    }
}

/// Render the fuzzy-find overlay as a titled table.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    fuzz: &FuzzyFindState,
    flow_index: &Option<FlowIndex>,
) {
    use crate::widget::filter_bar::{FilterChip, build_chip_line};
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Cell, Row, Table, TableState};

    let w = area.width.min(80);
    let h = area.height.min(17);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, rect);
    let block = crate::widget::panel::Panel::new(
        " Fuzzy Find — :proc :pg :cs :conn :in :out :drift :stale :modified :syncerr · esc close ",
    )
    .into_block();
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Inner layout: chip row, query line, separator, table body.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    // Chip row — read-only indicator of the parsed kind/drift filter.
    let mut chip_cells: Vec<FilterChip> = KIND_CHIPS
        .iter()
        .map(|(label, kind)| FilterChip {
            text: label,
            style: kind_chip_style(fuzz.filter == QueryFilter::Kind(*kind)),
        })
        .collect();
    chip_cells.extend(DRIFT_CHIPS.iter().map(|(label, drift)| FilterChip {
        text: label,
        style: kind_chip_style(fuzz.filter == QueryFilter::Drift(*drift)),
    }));
    frame.render_widget(
        Paragraph::new(build_chip_line(&chip_cells, "  ")),
        chunks[0],
    );

    frame.render_widget(
        Paragraph::new(Line::from(format!("> {}_", fuzz.query))),
        chunks[1],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[2].width as usize),
            theme::muted(),
        ))),
        chunks[2],
    );

    let Some(idx) = flow_index else {
        let body = Paragraph::new(Line::from(Span::styled(
            "no index (visit Browser tab first)",
            theme::muted(),
        )));
        frame.render_widget(body, chunks[3]);
        return;
    };

    if fuzz.matches.is_empty() {
        let msg = if fuzz.effective_query.is_empty() && fuzz.filter == QueryFilter::None {
            "no entries"
        } else {
            "no matches"
        };
        let body = Paragraph::new(Line::from(Span::styled(msg, theme::muted())));
        frame.render_widget(body, chunks[3]);
        return;
    }

    let header = Row::new(vec![
        Cell::from(Span::styled("Kind", theme::muted())),
        Cell::from(Span::styled("Name", theme::muted())),
        Cell::from(Span::styled("Path", theme::muted())),
        Cell::from(Span::styled("State", theme::muted())),
    ]);

    let rows: Vec<Row> = fuzz
        .matches
        .iter()
        .filter_map(|m| {
            let entry = idx.entries.get(m.index_entry)?;
            let kind_cell = Cell::from(Span::styled(kind_short_label(entry.kind), theme::muted()));
            let name_cell = Cell::from(Line::from(highlight_spans_for_name(
                &entry.name,
                &m.highlights,
            )));
            let path_cell = Cell::from(Span::styled(entry.group_path.clone(), theme::muted()));
            let state_cell = state_cell(&entry.state);
            Some(Row::new(vec![kind_cell, name_cell, path_cell, state_cell]))
        })
        .collect();

    let widths = [
        Constraint::Length(6),
        Constraint::Percentage(45),
        Constraint::Percentage(45),
        Constraint::Length(10),
    ];
    let row_count = rows.len();
    let table = Table::new(rows, widths)
        .header(header)
        .row_highlight_style(theme::cursor_row());

    let mut ts = TableState::default();
    ts.select(Some(fuzz.selected.min(row_count.saturating_sub(1))));
    frame.render_stateful_widget(table, chunks[3], &mut ts);
}

/// Emit a name-column `Span` list with matched characters highlighted
/// (bold + accent). `highlights` are character offsets into the
/// haystack; only positions that fall inside `0..name.chars().count()`
/// are rendered as highlighted.
fn highlight_spans_for_name<'a>(name: &'a str, highlights: &[u32]) -> Vec<Span<'a>> {
    use ratatui::style::Modifier;
    let name_len = name.chars().count() as u32;
    let mut positions: Vec<u32> = highlights
        .iter()
        .copied()
        .filter(|p| *p < name_len)
        .collect();
    positions.sort_unstable();
    positions.dedup();
    if positions.is_empty() {
        return vec![Span::raw(name)];
    }

    let highlight_style = theme::accent().add_modifier(Modifier::BOLD);
    let mut spans: Vec<Span<'a>> = Vec::new();
    let mut cursor: usize = 0;
    let chars: Vec<(usize, char)> = name.char_indices().collect();
    let mut p_iter = positions.into_iter().peekable();

    while cursor < chars.len() {
        if let Some(&next_p) = p_iter.peek()
            && next_p as usize == cursor
        {
            // Consume the contiguous run of highlighted positions.
            let start_byte = chars[cursor].0;
            let mut end_char = cursor;
            while let Some(&p) = p_iter.peek() {
                if p as usize == end_char {
                    end_char += 1;
                    p_iter.next();
                } else {
                    break;
                }
            }
            let end_byte = if end_char < chars.len() {
                chars[end_char].0
            } else {
                name.len()
            };
            spans.push(Span::styled(&name[start_byte..end_byte], highlight_style));
            cursor = end_char;
        } else {
            let start_byte = chars[cursor].0;
            // unwrap_or(chars.len()) covers the drain case: when p_iter is
            // exhausted, end_char becomes chars.len(), end_byte = name.len(),
            // and cursor exits the while loop after this final plain span.
            let next_highlight_char = p_iter.peek().map(|p| *p as usize).unwrap_or(chars.len());
            let end_char = next_highlight_char.min(chars.len());
            let end_byte = if end_char < chars.len() {
                chars[end_char].0
            } else {
                name.len()
            };
            spans.push(Span::raw(&name[start_byte..end_byte]));
            cursor = end_char;
        }
    }
    spans
}

fn kind_short_label(kind: NodeKind) -> &'static str {
    match kind {
        NodeKind::Processor => "Proc",
        NodeKind::ProcessGroup => "PG",
        NodeKind::Connection => "Conn",
        NodeKind::ControllerService => "CS",
        NodeKind::InputPort => "In",
        NodeKind::OutputPort => "Out",
        NodeKind::Folder(_) => "Folder",
    }
}

fn state_cell<'a>(
    badge: &'a crate::view::browser::state::StateBadge,
) -> ratatui::widgets::Cell<'a> {
    use crate::view::browser::state::StateBadge;
    use ratatui::widgets::Cell;
    match badge {
        StateBadge::Processor { glyph, style } => {
            Cell::from(Span::styled(glyph.to_string(), *style))
        }
        StateBadge::Cs { label, style } => Cell::from(Span::styled(label.as_str(), *style)),
        StateBadge::Pg { invalid } => {
            if *invalid > 0 {
                Cell::from(Span::styled(format!("\u{26A0}{invalid}"), theme::warning()))
            } else {
                Cell::from("")
            }
        }
        StateBadge::Conn { fill_percent } => {
            Cell::from(Span::styled(format!("{fill_percent}%"), theme::muted()))
        }
        StateBadge::Port => Cell::from(""),
    }
}

/// Style for a kind chip. Active chip = bold accent; inactive = muted.
fn kind_chip_style(active: bool) -> ratatui::style::Style {
    use ratatui::style::Modifier;
    if active {
        theme::accent().add_modifier(Modifier::BOLD)
    } else {
        theme::muted()
    }
}

/// Ordered chip list for the kind filter row. Order stays stable so
/// the visual layout does not shift as the filter changes.
const KIND_CHIPS: &[(&str, NodeKind)] = &[
    ("Proc", NodeKind::Processor),
    ("PG", NodeKind::ProcessGroup),
    ("CS", NodeKind::ControllerService),
    ("Conn", NodeKind::Connection),
    ("In", NodeKind::InputPort),
    ("Out", NodeKind::OutputPort),
];

/// Ordered chip list for the drift filter row. Order stays stable so
/// the visual layout does not shift as the filter changes.
const DRIFT_CHIPS: &[(&str, DriftFilter)] = &[
    ("Drift", DriftFilter::Any),
    ("Stale", DriftFilter::Stale),
    ("Modified", DriftFilter::Modified),
    ("SyncErr", DriftFilter::SyncErr),
];

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::NodeKind;

    fn sample_index() -> FlowIndex {
        use crate::view::browser::state::StateBadge;
        FlowIndex {
            entries: vec![
                FlowIndexEntry {
                    id: "p1".into(),
                    group_id: "root".into(),
                    kind: NodeKind::Processor,
                    name: "PutKafka".into(),
                    group_path: "root/publish".into(),
                    state: StateBadge::Processor {
                        glyph: '\u{25CF}',
                        style: crate::theme::success(),
                    },
                    haystack: "putkafka   processor   root/publish".into(),
                    version_state: None,
                },
                FlowIndexEntry {
                    id: "p2".into(),
                    group_id: "root".into(),
                    kind: NodeKind::Processor,
                    name: "GenerateFlowFile".into(),
                    group_path: "root".into(),
                    state: StateBadge::Processor {
                        glyph: '\u{25CF}',
                        style: crate::theme::success(),
                    },
                    haystack: "generateflowfile   processor   root".into(),
                    version_state: None,
                },
                FlowIndexEntry {
                    id: "cs1".into(),
                    group_id: "root".into(),
                    kind: NodeKind::ControllerService,
                    name: "kafka-brokers".into(),
                    group_path: "(controller)".into(),
                    state: StateBadge::Cs {
                        label: "ENABLED".into(),
                        style: crate::theme::success(),
                    },
                    haystack: "kafka-brokers   cs   (controller)".into(),
                    version_state: None,
                },
            ],
        }
    }

    #[test]
    fn empty_query_matches_everything() {
        let mut s = FuzzyFindState::new();
        s.rebuild_matches(&sample_index());
        assert_eq!(s.matches.len(), 3);
    }

    #[test]
    fn query_narrows_to_putkafka() {
        let mut s = FuzzyFindState::new();
        s.query = "putk".into();
        let idx = sample_index();
        s.rebuild_matches(&idx);
        assert!(!s.matches.is_empty());
        let top = s.selected_entry(&idx).unwrap();
        assert_eq!(top.id, "p1");
    }

    #[test]
    fn query_matches_kafka_across_processor_and_cs() {
        let mut s = FuzzyFindState::new();
        s.query = "kafka".into();
        s.rebuild_matches(&sample_index());
        assert!(s.matches.len() >= 2);
    }

    #[test]
    fn move_down_clamps_at_end() {
        let mut s = FuzzyFindState::new();
        s.rebuild_matches(&sample_index());
        for _ in 0..10 {
            s.move_down();
        }
        assert!(s.selected < s.matches.len());
    }

    #[test]
    fn fuzzy_table_renders_header_row_and_columns() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let out = format!("{}", term.backend());

        assert!(out.contains("Kind"), "expected Kind header in:\n{out}");
        assert!(out.contains("Name"), "expected Name header in:\n{out}");
        assert!(out.contains("Path"), "expected Path header in:\n{out}");
        assert!(out.contains("State"), "expected State header in:\n{out}");
        assert!(out.contains("PutKafka"), "expected sample row in:\n{out}");
        assert!(out.contains("Proc"), "expected Kind cell Proc in:\n{out}");
    }

    #[test]
    fn kind_priority_tiebreak_puts_processor_above_pg() {
        use crate::view::browser::state::StateBadge;
        // Two entries engineered to tie on fuzzy score: identical names so
        // nucleo returns the same score. Kinds differ — Processor should
        // land first.
        let index = FlowIndex {
            entries: vec![
                FlowIndexEntry {
                    id: "pg1".into(),
                    group_id: "root".into(),
                    kind: NodeKind::ProcessGroup,
                    name: "auth".into(),
                    group_path: "(root)".into(),
                    state: StateBadge::Pg { invalid: 0 },
                    haystack: "auth".into(),
                    version_state: None,
                },
                FlowIndexEntry {
                    id: "p1".into(),
                    group_id: "root".into(),
                    kind: NodeKind::Processor,
                    name: "auth".into(),
                    group_path: "root".into(),
                    state: StateBadge::Processor {
                        glyph: '\u{25CF}',
                        style: crate::theme::success(),
                    },
                    haystack: "auth".into(),
                    version_state: None,
                },
            ],
        };
        let mut s = FuzzyFindState::new();
        s.query = "auth".into();
        s.rebuild_matches(&index);
        let first = s.selected_entry(&index).unwrap();
        assert_eq!(first.id, "p1", "processor should tie-break above PG");
    }

    #[test]
    fn fuzzy_table_renders_no_index_message_when_flow_index_is_none() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;

        let fuzz = FuzzyFindState::new();
        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &None)).unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains("no index"),
            "expected 'no index' placeholder in:\n{out}"
        );
    }

    #[test]
    fn highlight_spans_for_name_only_uses_name_range_positions() {
        // "PutKafka" has 8 chars; haystack starts with "putkafka".
        // Query "pk" should match positions 0 (P) and 3 (K) inside the
        // name. Positions outside 0..8 must be dropped.
        let name = "PutKafka";
        let highlights: Vec<u32> = vec![0, 3, 12]; // 12 is in "processor"
        let spans = highlight_spans_for_name(name, &highlights);
        // Flatten the rendered span strings back to a (string, bold?) pair.
        let flattened: Vec<(String, bool)> = spans
            .iter()
            .map(|s| {
                let bold = s
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD);
                (s.content.to_string(), bold)
            })
            .collect();
        assert_eq!(
            flattened,
            vec![
                ("P".into(), true),
                ("ut".into(), false),
                ("K".into(), true),
                ("afka".into(), false),
            ]
        );
    }

    #[test]
    fn fuzzy_table_highlights_matched_name_chars_in_rendered_buffer() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = "put".into();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let buffer = term.backend().buffer();
        // Find at least one cell containing 'P' with BOLD set.
        let bold_p_found = buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "P" && cell.style().add_modifier.contains(Modifier::BOLD));
        assert!(
            bold_p_found,
            "expected at least one bold 'P' cell in the rendered fuzzy table"
        );
    }

    #[test]
    fn fuzzy_table_renders_no_matches_when_query_excludes_everything() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = "zzzzzzzz_no_such".into();
        fuzz.rebuild_matches(&idx);
        assert!(fuzz.matches.is_empty(), "precondition: query excludes all");

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx)))
            .unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains("no matches"),
            "expected 'no matches' placeholder in:\n{out}"
        );
    }

    #[test]
    fn highlight_spans_for_name_empty_highlights_returns_single_raw_span() {
        let name = "PutKafka";
        let highlights: Vec<u32> = vec![];
        let spans = highlight_spans_for_name(name, &highlights);
        assert_eq!(spans.len(), 1);
        assert_eq!(spans[0].content.as_ref(), "PutKafka");
        assert!(
            !spans[0]
                .style
                .add_modifier
                .contains(ratatui::style::Modifier::BOLD)
        );
    }

    #[test]
    fn highlight_spans_for_name_adjacent_positions_collapse_into_one_span() {
        // Positions 1, 2, 3 in "PutKafka" should merge into a single
        // styled "utK" span flanked by "P" and "afka".
        let name = "PutKafka";
        let highlights: Vec<u32> = vec![1, 2, 3];
        let spans = highlight_spans_for_name(name, &highlights);
        let flattened: Vec<(String, bool)> = spans
            .iter()
            .map(|s| {
                let bold = s
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD);
                (s.content.to_string(), bold)
            })
            .collect();
        assert_eq!(
            flattened,
            vec![
                ("P".into(), false),
                ("utK".into(), true),
                ("afka".into(), false),
            ]
        );
    }

    #[test]
    fn highlight_spans_for_name_handles_multibyte_characters() {
        // "café" has 4 characters but 5 bytes (é = 2 bytes). Highlight
        // positions 2 and 3 cover 'f' and 'é' — the returned spans must
        // slice on valid UTF-8 boundaries and must not panic.
        let name = "café";
        let highlights: Vec<u32> = vec![2, 3];
        let spans = highlight_spans_for_name(name, &highlights);
        let flattened: Vec<(String, bool)> = spans
            .iter()
            .map(|s| {
                let bold = s
                    .style
                    .add_modifier
                    .contains(ratatui::style::Modifier::BOLD);
                (s.content.to_string(), bold)
            })
            .collect();
        assert_eq!(flattened, vec![("ca".into(), false), ("fé".into(), true),]);
    }

    #[test]
    fn parse_prefix_strips_leading_proc_token() {
        assert_eq!(
            parse_prefix(":proc kafka"),
            (QueryFilter::Kind(NodeKind::Processor), "kafka".to_string())
        );
    }

    #[test]
    fn parse_prefix_bare_alias_sets_kind_and_empty_query() {
        assert_eq!(
            parse_prefix(":proc"),
            (QueryFilter::Kind(NodeKind::Processor), String::new())
        );
    }

    #[test]
    fn parse_prefix_tolerates_leading_whitespace() {
        assert_eq!(
            parse_prefix("   :cs aws"),
            (
                QueryFilter::Kind(NodeKind::ControllerService),
                "aws".to_string()
            )
        );
    }

    #[test]
    fn parse_prefix_non_leading_alias_is_plain_text() {
        assert_eq!(
            parse_prefix("kafka :proc"),
            (QueryFilter::None, "kafka :proc".to_string())
        );
    }

    #[test]
    fn parse_prefix_unknown_alias_is_plain_text() {
        assert_eq!(
            parse_prefix(":nope foo"),
            (QueryFilter::None, ":nope foo".to_string())
        );
    }

    #[test]
    fn parse_prefix_case_insensitive_alias() {
        assert_eq!(
            parse_prefix(":PROC kafka"),
            (QueryFilter::Kind(NodeKind::Processor), "kafka".to_string())
        );
        assert_eq!(
            parse_prefix(":Proc kafka"),
            (QueryFilter::Kind(NodeKind::Processor), "kafka".to_string())
        );
    }

    #[test]
    fn parse_prefix_all_aliases() {
        assert_eq!(
            parse_prefix(":pg").0,
            QueryFilter::Kind(NodeKind::ProcessGroup)
        );
        assert_eq!(
            parse_prefix(":conn").0,
            QueryFilter::Kind(NodeKind::Connection)
        );
        assert_eq!(
            parse_prefix(":in").0,
            QueryFilter::Kind(NodeKind::InputPort)
        );
        assert_eq!(
            parse_prefix(":out").0,
            QueryFilter::Kind(NodeKind::OutputPort)
        );
    }

    #[test]
    fn parse_prefix_preserves_trailing_whitespace_in_query() {
        assert_eq!(
            parse_prefix(":proc kafka "),
            (QueryFilter::Kind(NodeKind::Processor), "kafka ".to_string())
        );
    }

    #[test]
    fn parse_prefix_empty_query() {
        assert_eq!(parse_prefix(""), (QueryFilter::None, String::new()));
    }

    #[test]
    fn rebuild_matches_filters_by_parsed_kind() {
        let mut s = FuzzyFindState::new();
        s.query = ":cs".into();
        s.rebuild_matches(&sample_index());
        assert_eq!(s.matches.len(), 1);
        assert_eq!(s.filter, QueryFilter::Kind(NodeKind::ControllerService));
    }

    #[test]
    fn rebuild_matches_applies_filter_and_then_fuzzy_needle() {
        let mut s = FuzzyFindState::new();
        s.query = ":proc kafka".into();
        let idx = sample_index();
        s.rebuild_matches(&idx);
        assert_eq!(s.filter, QueryFilter::Kind(NodeKind::Processor));
        assert_eq!(s.effective_query, "kafka");
        assert_eq!(s.matches.len(), 1);
        let top = s.selected_entry(&idx).unwrap();
        assert_eq!(top.id, "p1");
    }

    #[test]
    fn rebuild_matches_empty_needle_with_filter_returns_all_of_that_kind() {
        let mut s = FuzzyFindState::new();
        s.query = ":proc".into();
        s.rebuild_matches(&sample_index());
        assert_eq!(s.matches.len(), 2);
    }

    #[test]
    fn rebuild_matches_no_prefix_preserves_original_behavior() {
        let mut s = FuzzyFindState::new();
        s.query = "kafka".into();
        s.rebuild_matches(&sample_index());
        assert_eq!(s.filter, QueryFilter::None);
        assert_eq!(s.effective_query, "kafka");
        assert!(s.matches.len() >= 2);
    }

    #[test]
    fn rebuild_matches_unknown_prefix_falls_through_to_fuzzy() {
        let mut s = FuzzyFindState::new();
        s.query = ":nope".into();
        s.rebuild_matches(&sample_index());
        assert_eq!(s.filter, QueryFilter::None);
        assert_eq!(s.effective_query, ":nope");
    }

    #[test]
    fn render_draws_all_chip_labels_including_drift() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let out = format!("{}", term.backend());

        for label in [
            "Proc", "PG", "CS", "Conn", "In", "Out", "Drift", "Stale", "Modified", "SyncErr",
        ] {
            assert!(
                out.contains(label),
                "expected chip label {label} in:\n{out}"
            );
        }
    }

    #[test]
    fn render_highlights_drift_chip_when_drift_filter_active() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = ":drift".into();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let buffer = term.backend().buffer();

        // The "D" of "Drift" should be bold (active chip).
        let bold_d = buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "D" && cell.style().add_modifier.contains(Modifier::BOLD));
        assert!(
            bold_d,
            "expected the active Drift chip to render with BOLD set"
        );
    }

    #[test]
    fn render_highlights_stale_chip_when_stale_filter_active() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = ":stale".into();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let buffer = term.backend().buffer();

        // The "S" of "Stale" should be bold (active chip).
        // Note: "S" also appears in "SyncErr" and "CS" — we look for bold "S"
        // followed by "t" to confirm it's "Stale".
        let content = buffer.content();
        let bold_stale = content.windows(2).any(|w| {
            w[0].symbol() == "S"
                && w[0].style().add_modifier.contains(Modifier::BOLD)
                && w[1].symbol() == "t"
        });
        assert!(
            bold_stale,
            "expected the active Stale chip to render with BOLD set"
        );
    }

    #[test]
    fn render_highlights_kind_chip_unchanged_when_kind_filter_active() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = ":proc".into();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let buffer = term.backend().buffer();

        // "P" of "Proc" should be bold (kind filter active).
        let bold_p = buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "P" && cell.style().add_modifier.contains(Modifier::BOLD));
        assert!(
            bold_p,
            "expected the active Proc chip to render with BOLD set (kind filter regression)"
        );
        // Drift chip "D" of "Drift" should NOT be bold when kind filter is active.
        let bold_d = buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "D" && cell.style().add_modifier.contains(Modifier::BOLD));
        assert!(
            !bold_d,
            "Drift chip should be inactive (muted) when a kind filter is active"
        );
    }

    #[test]
    fn render_active_kind_chip_uses_bold_style() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = ":proc".into();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx.clone())))
            .unwrap();
        let buffer = term.backend().buffer();

        let bold_p = buffer
            .content()
            .iter()
            .any(|cell| cell.symbol() == "P" && cell.style().add_modifier.contains(Modifier::BOLD));
        assert!(
            bold_p,
            "expected the active Proc chip to render with BOLD set"
        );
    }

    #[test]
    fn render_title_advertises_kind_aliases() {
        use crate::test_support::{TEST_BACKEND_SHORT, test_backend};
        use ratatui::Terminal;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.rebuild_matches(&idx);

        let backend = test_backend(TEST_BACKEND_SHORT);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), &fuzz, &Some(idx)))
            .unwrap();
        let out = format!("{}", term.backend());
        assert!(
            out.contains(":proc"),
            "expected title to mention :proc alias — got:\n{out}"
        );
    }

    // ── Drift alias tests ─────────────────────────────────────────────────

    #[test]
    fn parse_prefix_drift_alias() {
        assert_eq!(
            parse_prefix(":drift"),
            (QueryFilter::Drift(DriftFilter::Any), String::new())
        );
        assert_eq!(
            parse_prefix(":stale"),
            (QueryFilter::Drift(DriftFilter::Stale), String::new())
        );
        assert_eq!(
            parse_prefix(":modified ingest"),
            (
                QueryFilter::Drift(DriftFilter::Modified),
                "ingest".to_string()
            )
        );
        assert_eq!(
            parse_prefix(":syncerr"),
            (QueryFilter::Drift(DriftFilter::SyncErr), String::new())
        );
    }

    #[test]
    fn parse_prefix_kind_alias_returns_kind_filter() {
        assert_eq!(
            parse_prefix(":proc kafka"),
            (QueryFilter::Kind(NodeKind::Processor), "kafka".to_string())
        );
    }

    #[test]
    fn parse_prefix_no_alias_returns_none_filter() {
        assert_eq!(
            parse_prefix("kafka"),
            (QueryFilter::None, "kafka".to_string())
        );
    }

    #[test]
    fn query_filter_drift_matches_only_pg_with_state() {
        use crate::view::browser::state::StateBadge;
        use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;

        fn entry(kind: NodeKind, version_state: Option<S>) -> FlowIndexEntry {
            FlowIndexEntry {
                id: "x".into(),
                group_id: String::new(),
                kind,
                name: "x".into(),
                group_path: "(root)".into(),
                state: StateBadge::Port,
                haystack: String::new(),
                version_state,
            }
        }
        let stale_pg = entry(NodeKind::ProcessGroup, Some(S::Stale));
        let stale_modified_pg = entry(NodeKind::ProcessGroup, Some(S::LocallyModifiedAndStale));
        let modified_pg = entry(NodeKind::ProcessGroup, Some(S::LocallyModified));
        let processor = entry(NodeKind::Processor, None);

        assert!(QueryFilter::Drift(DriftFilter::Stale).matches(&stale_pg));
        assert!(QueryFilter::Drift(DriftFilter::Stale).matches(&stale_modified_pg));
        assert!(!QueryFilter::Drift(DriftFilter::Stale).matches(&modified_pg));
        assert!(!QueryFilter::Drift(DriftFilter::Stale).matches(&processor));

        assert!(QueryFilter::Drift(DriftFilter::Modified).matches(&modified_pg));
        assert!(QueryFilter::Drift(DriftFilter::Modified).matches(&stale_modified_pg));
        assert!(!QueryFilter::Drift(DriftFilter::Modified).matches(&stale_pg));

        assert!(QueryFilter::Drift(DriftFilter::Any).matches(&stale_pg));
        assert!(QueryFilter::Drift(DriftFilter::Any).matches(&modified_pg));
        assert!(!QueryFilter::Drift(DriftFilter::Any).matches(&processor));
    }

    #[test]
    fn query_filter_kind_matches_kind_only() {
        use crate::view::browser::state::StateBadge;

        fn entry(kind: NodeKind) -> FlowIndexEntry {
            FlowIndexEntry {
                id: "x".into(),
                group_id: String::new(),
                kind,
                name: "x".into(),
                group_path: "(root)".into(),
                state: StateBadge::Port,
                haystack: String::new(),
                version_state: None,
            }
        }
        assert!(QueryFilter::Kind(NodeKind::Processor).matches(&entry(NodeKind::Processor)));
        assert!(!QueryFilter::Kind(NodeKind::Processor).matches(&entry(NodeKind::ProcessGroup)));
        assert!(QueryFilter::None.matches(&entry(NodeKind::Processor)));
        assert!(QueryFilter::None.matches(&entry(NodeKind::ProcessGroup)));
    }
}
