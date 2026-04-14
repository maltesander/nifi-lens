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

#[derive(Debug)]
pub struct FuzzyFindState {
    pub query: String,
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
    }
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
            matches: Vec::new(),
            selected: 0,
        }
    }

    /// Rebuild `matches` against `index`. Top 50 by score descending.
    /// An empty query matches everything in the corpus.
    pub fn rebuild_matches(&mut self, index: &FlowIndex) {
        let mut matcher = Matcher::new(Config::DEFAULT.match_paths());
        let mut query_buf = Vec::new();
        let lowered = self.query.to_lowercase();
        let pattern = Utf32Str::new(&lowered, &mut query_buf);
        let mut results: Vec<MatchedEntry> = Vec::new();
        for (i, entry) in index.entries.iter().enumerate() {
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
    use ratatui::layout::{Constraint, Direction, Layout};
    use ratatui::widgets::{Cell, Row, Table, TableState};

    let w = area.width.min(80);
    let h = area.height.min(16);
    let x = area.x + (area.width - w) / 2;
    let y = area.y + (area.height - h) / 2;
    let rect = Rect {
        x,
        y,
        width: w,
        height: h,
    };

    frame.render_widget(Clear, rect);
    let block = crate::widget::panel::Panel::new(" Fuzzy Find — esc to close ").into_block();
    let inner = block.inner(rect);
    frame.render_widget(block, rect);

    // Inner layout: query line, separator, table body.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Min(0),
        ])
        .split(inner);

    frame.render_widget(
        Paragraph::new(Line::from(format!("> {}_", fuzz.query))),
        chunks[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[1].width as usize),
            theme::muted(),
        ))),
        chunks[1],
    );

    let Some(idx) = flow_index else {
        let body = Paragraph::new(Line::from(Span::styled(
            "no index (visit Browser tab first)",
            theme::muted(),
        )));
        frame.render_widget(body, chunks[2]);
        return;
    };

    if fuzz.matches.is_empty() {
        let msg = if fuzz.query.is_empty() {
            "no entries"
        } else {
            "no matches"
        };
        let body = Paragraph::new(Line::from(Span::styled(msg, theme::muted())));
        frame.render_widget(body, chunks[2]);
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
    frame.render_stateful_widget(table, chunks[2], &mut ts);
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
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.rebuild_matches(&idx);

        let backend = TestBackend::new(100, 20);
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
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let fuzz = FuzzyFindState::new();
        let backend = TestBackend::new(100, 20);
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
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;
        use ratatui::style::Modifier;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = "put".into();
        fuzz.rebuild_matches(&idx);

        let backend = TestBackend::new(100, 20);
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
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let idx = sample_index();
        let mut fuzz = FuzzyFindState::new();
        fuzz.query = "zzzzzzzz_no_such".into();
        fuzz.rebuild_matches(&idx);
        assert!(fuzz.matches.is_empty(), "precondition: query excludes all");

        let backend = TestBackend::new(100, 20);
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
}
