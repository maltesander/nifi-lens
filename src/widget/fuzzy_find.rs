//! Fuzzy-find modal backed by nucleo.
//!
//! The widget is state + reducer-side helpers only; the modal overlay
//! render lands in Task 18. The corpus (`FlowIndex`) is shared with the
//! Browser tab and populated by `apply_tree_snapshot`. This widget
//! never touches the corpus directly — it receives a borrow at match
//! time and writes results into its own `matches` field.

use nucleo::{Config, Matcher, Utf32Str};
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::client::NodeKind;
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

/// Render the fuzzy-find overlay.
///
/// This is a transitional implementation — still a `Paragraph` of
/// pre-formatted lines. Task 6 rewrites it to use `Table` with
/// structured columns.
pub fn render(
    frame: &mut Frame,
    area: Rect,
    fuzz: &FuzzyFindState,
    flow_index: &Option<FlowIndex>,
) {
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

    let block = Block::default()
        .borders(Borders::ALL)
        .title(" Fuzzy Find — esc to close ");
    let inner = block.inner(rect);
    frame.render_widget(Clear, rect);
    frame.render_widget(block, rect);

    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(format!("> {}_", fuzz.query)));
    lines.push(Line::from(Span::styled(
        "─".repeat(inner.width as usize),
        crate::theme::muted(),
    )));
    if let Some(idx) = flow_index {
        let max_rows = (inner.height as usize).saturating_sub(3);
        for (i, m) in fuzz.matches.iter().enumerate().take(max_rows) {
            let Some(entry) = idx.entries.get(m.index_entry) else {
                continue;
            };
            let marker = if i == fuzz.selected { "▸ " } else { "  " };
            let style = if i == fuzz.selected {
                crate::theme::cursor_row()
            } else {
                Style::default()
            };
            lines.push(Line::from(vec![
                Span::raw(marker),
                Span::styled(format!("{}   {}", entry.name, entry.group_path), style),
            ]));
        }
    } else {
        lines.push(Line::from(Span::styled("no index", crate::theme::muted())));
    }
    frame.render_widget(Paragraph::new(lines), inner);
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
}
