use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::app::state::ViewId;

const GLOBAL_TEXT: &str = "\
Global Keys:
  Tab / Shift+Tab   Cycle tabs
  F1..F4            Jump to tab
  Ctrl+K            Switch context
  Ctrl+F            Global fuzzy find (requires Browser seed)
  ?                 Toggle this help
  q / Ctrl+Q        Quit
  Esc               Close modal
";

const OVERVIEW_TEXT: &str = "\
Overview Tab:
  (auto-refresh every 10s; no tab-local keys yet)
";

const BULLETINS_TEXT: &str = "\
Bulletins Tab:
  j / ↓             Move selection down
  k / ↑             Move selection up
  g / Home          Jump to oldest
  G / End           Jump to newest (resume auto-scroll)
  p                 Toggle auto-scroll pause
  e / w / i         Toggle Error / Warning / Info chip
  T                 Cycle component-type chip
  /                 Enter text filter mode
  c                 Clear all filters
  Enter             Jump to Browser (Phase 3 stub)
  t                 Trace component (Phase 4 stub)
";

const BROWSER_TEXT: &str = "\
Browser Tab:
  ↑/↓ or j/k       Move selection
  PgUp/PgDn         Page scroll
  Home/End          Jump to first / last row
  Enter / → / l     Expand PG and drill in (leaf: no-op)
  Backspace / ← / h Collapse PG / move to parent
  r                 Force-refresh tree
  e                 Expand properties (Processor/CS with detail)
  c                 Copy selected node id to clipboard
  t                 Trace selected processor (Phase 4 stub)
  Ctrl+F            Open fuzzy find
";

const TRACER_TEXT: &str = "\
Tracer Tab:

Entry mode (empty paste form):
  Enter       submit UUID
  Esc / Ctrl+U   clear input

Latest events mode (from Bulletins/Browser):
  j / k          move selection
  Enter          trace selected flowfile
  r              refresh list
  c              copy selected uuid
  Esc            back to Entry

Lineage running mode:
  Esc            cancel query

Lineage view mode:
  j / k          move selection (resets event detail)
  Enter          load event detail
  i              load input content
  o              load output content
  s              save content to file
  a              toggle attribute diff mode (All / Changed)
  r              re-run lineage query
  c              copy selected event's flowfile uuid
  Esc / /        back to Entry
";

pub fn render(frame: &mut Frame, area: Rect, current_tab: ViewId) {
    let per_view = match current_tab {
        ViewId::Overview => OVERVIEW_TEXT,
        ViewId::Bulletins => BULLETINS_TEXT,
        ViewId::Browser => BROWSER_TEXT,
        ViewId::Tracer => TRACER_TEXT,
    };
    let text = format!("{GLOBAL_TEXT}\n{per_view}");
    let modal = center(area, 70, 22);
    frame.render_widget(Clear, modal);
    let block = Block::default().title(" Help ").borders(Borders::ALL);
    let p = Paragraph::new(text).alignment(Alignment::Left).block(block);
    frame.render_widget(p, modal);
}

fn center(area: Rect, pct_x: u16, height: u16) -> Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(height),
            Constraint::Fill(1),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - pct_x) / 2),
            Constraint::Percentage(pct_x),
            Constraint::Percentage((100 - pct_x) / 2),
        ])
        .split(vertical[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn render_with(tab: ViewId) -> String {
        let backend = TestBackend::new(80, 30);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, f.area(), tab)).unwrap();
        format!("{}", term.backend())
    }

    #[test]
    fn bulletins_help_lists_view_local_keys() {
        let out = render_with(ViewId::Bulletins);
        assert!(out.contains("e / w / i"));
        assert!(out.contains("Toggle auto-scroll pause"));
        assert!(out.contains("Jump to Browser"));
    }

    #[test]
    fn overview_help_does_not_list_bulletins_keys() {
        let out = render_with(ViewId::Overview);
        assert!(!out.contains("Toggle Error"));
    }
}
