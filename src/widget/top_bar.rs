//! Top-bar widget: one-row tab bar plus right-aligned cluster identity strip.
//!
//! Layout C from the UI reorg spec (2026-04-13). The identity strip
//! degrades gracefully on narrow terminals by dropping elements from the
//! tail, in this priority order:
//!
//! 1. `[context]` — always visible
//! 2. `v<version>` — dropped first
//! 3. `nodes N/M` — dropped second (but kept if context name has to be
//!    truncated instead)
//!
//! Minimum renderable form is `[ctx] nodes N/M`.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Tabs};
use semver::Version;
use unicode_width::UnicodeWidthStr;

use crate::app::state::{AppState, ClusterSummary, ViewId};
use crate::theme;

/// Build the right-aligned identity spans for a given budget in columns.
/// Returns spans that together are at most `budget` display columns wide.
///
/// The returned span list contains no leading padding — the caller is
/// responsible for right-aligning it inside the available area.
pub fn build_identity_spans(
    context_name: &str,
    version: &Version,
    cluster: &ClusterSummary,
    budget: usize,
) -> Vec<Span<'static>> {
    let ctx = format!("[{context_name}]");
    let ver = format!("v{}.{}.{}", version.major, version.minor, version.patch);
    let nodes_text = match (cluster.connected_nodes, cluster.total_nodes) {
        (Some(c), Some(t)) => format!("nodes {c}/{t}"),
        _ => "nodes ?/?".to_string(),
    };
    let nodes_style = nodes_style(cluster);

    // Full form: "[ctx] v2.9.0 · nodes 3/3"
    let full_width = ctx.width() + 1 + ver.width() + 3 + nodes_text.width();
    if full_width <= budget {
        return vec![
            Span::styled(ctx, theme::muted()),
            Span::raw(" "),
            Span::styled(ver, theme::muted()),
            Span::styled(" \u{00b7} ", theme::muted()),
            Span::styled(nodes_text, nodes_style),
        ];
    }

    // Drop version: "[ctx] · nodes 3/3"
    let no_version_width = ctx.width() + 3 + nodes_text.width();
    if no_version_width <= budget {
        return vec![
            Span::styled(ctx, theme::muted()),
            Span::styled(" \u{00b7} ", theme::muted()),
            Span::styled(nodes_text, nodes_style),
        ];
    }

    // Drop nodes, keep context only: "[ctx]"
    if ctx.width() <= budget {
        return vec![Span::styled(ctx, theme::muted())];
    }

    // Truncate context with ellipsis. Budget of 0 returns empty.
    if budget == 0 {
        return vec![];
    }
    let truncated = truncate_to_width(&ctx, budget);
    vec![Span::styled(truncated, theme::muted())]
}

fn nodes_style(cluster: &ClusterSummary) -> ratatui::style::Style {
    match (cluster.connected_nodes, cluster.total_nodes) {
        (Some(c), Some(t)) if c == t => theme::muted(),
        (Some(c), Some(t)) if c * 2 < t => theme::error(),
        (Some(_), Some(_)) => theme::warning(),
        _ => theme::muted(),
    }
}

fn truncate_to_width(s: &str, max_cols: usize) -> String {
    if s.width() <= max_cols {
        return s.to_string();
    }
    if max_cols == 0 {
        return String::new();
    }
    let budget = max_cols.saturating_sub(1);
    let mut out = String::new();
    let mut cols = 0;
    for c in s.chars() {
        let w = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if cols + w > budget {
            break;
        }
        out.push(c);
        cols += w;
    }
    out.push('\u{2026}');
    out
}

// Phase 1 layout. `Health` is deleted in Phase 3; all other tabs are
// the final shape.
const TAB_LABELS: &[&str] = &[
    "Overview",
    "Bulletins",
    "Health",
    "Browser",
    "Events",
    "Tracer",
];

/// Render the 1-row top bar into `area`. Tabs left-aligned, identity
/// strip right-aligned. No bordered box.
pub fn render(frame: &mut Frame, area: Rect, state: &AppState) {
    let tab_width = tab_bar_width() as u16;
    let chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(tab_width), Constraint::Fill(1)])
        .split(area);

    render_tabs(frame, chunks[0], state.current_tab);
    render_identity(frame, chunks[1], state);
}

fn render_tabs(frame: &mut Frame, area: Rect, current: ViewId) {
    let idx = tab_index(current);
    let tabs = Tabs::new(
        TAB_LABELS
            .iter()
            .map(|label| Line::from(*label))
            .collect::<Vec<_>>(),
    )
    .select(idx)
    .highlight_style(theme::accent())
    .divider(" \u{2502} ");
    frame.render_widget(tabs, area);
}

fn render_identity(frame: &mut Frame, area: Rect, state: &AppState) {
    let budget = area.width as usize;
    let spans = build_identity_spans(
        &state.context_name,
        &state.detected_version,
        &state.cluster_summary,
        budget,
    );
    let line = Line::from(spans);
    frame.render_widget(Paragraph::new(line).alignment(Alignment::Right), area);
}

fn tab_index(view: ViewId) -> usize {
    match view {
        ViewId::Overview => 0,
        ViewId::Bulletins => 1,
        ViewId::Health => 2,
        ViewId::Browser => 3,
        ViewId::Events => 4,
        ViewId::Tracer => 5,
    }
}

fn tab_bar_width() -> usize {
    let labels: usize = TAB_LABELS.iter().map(|l| l.width()).sum();
    let separators = (TAB_LABELS.len() - 1) * 3;
    labels + separators + 1
}

#[cfg(test)]
mod tests {
    use super::*;

    fn span_text(spans: &[Span<'_>]) -> String {
        spans.iter().map(|s| s.content.as_ref()).collect()
    }

    fn ver() -> Version {
        Version::new(2, 9, 0)
    }

    fn cluster_healthy() -> ClusterSummary {
        ClusterSummary {
            connected_nodes: Some(3),
            total_nodes: Some(3),
        }
    }

    fn cluster_none() -> ClusterSummary {
        ClusterSummary::default()
    }

    #[test]
    fn full_form_fits_in_40_cols() {
        let spans = build_identity_spans("dev-nifi-2-9-0", &ver(), &cluster_healthy(), 40);
        assert_eq!(
            span_text(&spans),
            "[dev-nifi-2-9-0] v2.9.0 \u{00b7} nodes 3/3"
        );
    }

    #[test]
    fn drops_version_on_narrow_budget() {
        // "[dev-nifi-2-9-0] v2.9.0 · nodes 3/3" is 35 cols; cap at 30.
        let spans = build_identity_spans("dev-nifi-2-9-0", &ver(), &cluster_healthy(), 30);
        assert_eq!(span_text(&spans), "[dev-nifi-2-9-0] \u{00b7} nodes 3/3");
    }

    #[test]
    fn drops_nodes_on_very_narrow_budget() {
        let spans = build_identity_spans("dev-nifi-2-9-0", &ver(), &cluster_healthy(), 20);
        assert_eq!(span_text(&spans), "[dev-nifi-2-9-0]");
    }

    #[test]
    fn truncates_context_with_ellipsis_when_nothing_else_fits() {
        let spans = build_identity_spans(
            "very-long-cluster-context-name",
            &ver(),
            &cluster_healthy(),
            10,
        );
        let text = span_text(&spans);
        assert!(text.ends_with('\u{2026}'));
        assert!(text.width() <= 10);
    }

    #[test]
    fn placeholder_nodes_when_cluster_summary_is_empty() {
        let spans = build_identity_spans("dev", &ver(), &cluster_none(), 40);
        assert_eq!(span_text(&spans), "[dev] v2.9.0 \u{00b7} nodes ?/?");
    }

    #[test]
    fn nodes_style_all_connected_is_muted() {
        let c = ClusterSummary {
            connected_nodes: Some(3),
            total_nodes: Some(3),
        };
        assert_eq!(nodes_style(&c), theme::muted());
    }

    #[test]
    fn nodes_style_partial_down_is_warning() {
        let c = ClusterSummary {
            connected_nodes: Some(2),
            total_nodes: Some(3),
        };
        assert_eq!(nodes_style(&c), theme::warning());
    }

    #[test]
    fn nodes_style_majority_down_is_error() {
        let c = ClusterSummary {
            connected_nodes: Some(1),
            total_nodes: Some(3),
        };
        assert_eq!(nodes_style(&c), theme::error());
    }

    #[test]
    fn zero_budget_returns_empty() {
        let spans = build_identity_spans("dev", &ver(), &cluster_healthy(), 0);
        assert!(spans.is_empty());
    }

    #[test]
    fn every_view_id_indexes_inside_tab_labels() {
        use crate::app::state::ViewId;
        // Ensures tab_index never returns an out-of-bounds index for any
        // ViewId variant that still exists. Guards against the Phase 1
        // Health-included tab layout drifting out of sync with TAB_LABELS.
        for view in [
            ViewId::Overview,
            ViewId::Bulletins,
            ViewId::Health,
            ViewId::Browser,
            ViewId::Events,
            ViewId::Tracer,
        ] {
            let idx = super::tab_index(view);
            assert!(
                idx < super::TAB_LABELS.len(),
                "tab_index({view:?}) = {idx} is out of bounds for TAB_LABELS (len {})",
                super::TAB_LABELS.len()
            );
        }
    }

    #[test]
    fn renders_tab_bar_with_right_aligned_identity() {
        use crate::app::state::{ClusterSummary, ViewId};
        use crate::test_support::fresh_state;
        use ratatui::Terminal;
        use ratatui::backend::TestBackend;

        let mut state = fresh_state();
        state.context_name = "dev-nifi-2-9-0".into();
        state.current_tab = ViewId::Bulletins;
        state.cluster_summary = ClusterSummary {
            connected_nodes: Some(3),
            total_nodes: Some(3),
        };

        let backend = TestBackend::new(100, 1);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| super::render(f, f.area(), &state)).unwrap();
        let rendered = format!("{}", term.backend());

        // Tab bar appears on the left.
        assert!(rendered.contains("Overview"));
        assert!(rendered.contains("Bulletins"));
        // Identity strip appears somewhere on the row.
        assert!(rendered.contains("[dev-nifi-2-9-0]"));
        assert!(rendered.contains("v2.9.0"));
        assert!(rendered.contains("nodes 3/3"));
    }
}
