//! Renderer for the Overview tab's top "Components" panel.
//!
//! Three-row table — process groups, processors, controller services.
//! Display-only; not focusable. Aligned columns: 2-pad + 20-label +
//! 4-count + 4-gap + repeating 12-slot (8 label + 4 value).

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;

use super::super::state::OverviewState;
use crate::client::{ControllerServiceCounts, ControllerStatusSnapshot, ProcessorStateCounts};
use crate::theme;

/// Three-row Components table — process groups, processors, controller
/// services. Display-only; not focusable.
///
/// All projections are sourced from `OverviewState` fields mirrored
/// from `AppState.cluster.snapshot` by the `redraw_*` reducers. The
/// renderer shows "loading…" until both `root_pg_status` and
/// `controller_status` have landed in the cluster snapshot. The CS row
/// degrades to "cs list unavailable" when `state.cs_counts` is `None`.
pub(super) fn render_components_table(frame: &mut Frame, area: Rect, state: &OverviewState) {
    let (Some(controller), Some(root_pg)) = (state.controller.as_ref(), state.root_pg.as_ref())
    else {
        let line = Line::from(Span::styled("loading…", theme::muted()));
        frame.render_widget(Paragraph::new(line), area);
        return;
    };
    let lines = vec![
        pg_row(controller, root_pg),
        processors_row(&root_pg.processors),
        controller_services_row(state.cs_counts.as_ref()),
    ];
    frame.render_widget(Paragraph::new(lines), area);
}

fn pg_row(
    controller: &ControllerStatusSnapshot,
    root_pg: &crate::client::RootPgStatusSnapshot,
) -> Line<'static> {
    let mut spans = label_and_count("Process groups", root_pg.process_group_count);
    let stale = controller.stale;
    let modified = controller.locally_modified;
    let sync_err = controller.sync_failure;
    if stale + modified + sync_err == 0 {
        spans.extend(slot_text("all in sync", theme::muted()));
    } else {
        spans.extend(slot("STALE", stale, theme::warning()));
        spans.extend(slot_gap());
        spans.extend(slot("MODIFIED", modified, theme::warning()));
        spans.extend(slot_gap());
        spans.extend(slot("SYNC-ERR", sync_err, theme::error()));
    }
    spans.extend(slot_gap());
    spans.extend(slot("INPUTS", root_pg.input_port_count, theme::muted()));
    spans.extend(slot_gap());
    spans.extend(slot("OUTPUTS", root_pg.output_port_count, theme::muted()));
    Line::from(spans)
}

fn processors_row(p: &ProcessorStateCounts) -> Line<'static> {
    let mut spans = label_and_count("Processors", p.total());
    spans.extend(slot("RUNNING", p.running, theme::success()));
    spans.extend(slot_gap());
    spans.extend(slot("STOPPED", p.stopped, theme::warning()));
    spans.extend(slot_gap());
    spans.extend(slot("INVALID", p.invalid, theme::error()));
    spans.extend(slot_gap());
    spans.extend(slot("DISABLED", p.disabled, theme::muted()));
    Line::from(spans)
}

fn controller_services_row(counts: Option<&ControllerServiceCounts>) -> Line<'static> {
    match counts {
        Some(c) => {
            let mut spans = label_and_count("Controller services", c.total());
            spans.extend(slot("ENABLED", c.enabled, theme::success()));
            spans.extend(slot_gap());
            spans.extend(slot("DISABLED", c.disabled, theme::muted()));
            spans.extend(slot_gap());
            spans.extend(slot("INVALID", c.invalid, theme::error()));
            Line::from(spans)
        }
        None => Line::from(vec![
            Span::raw("  "),
            Span::raw(format!("{:<20}", "Controller services")),
            Span::styled(format!("{:>4}", "?"), theme::muted()),
            Span::raw("    "),
            Span::styled("cs list unavailable", theme::error()),
        ]),
    }
}

/// Returns `[pad, label-padded-to-20, count-right-aligned-in-4, 4-space-gap]`
/// — the fixed prefix every row shares.
fn label_and_count(label: &str, count: u32) -> Vec<Span<'static>> {
    vec![
        Span::raw("  "),
        Span::raw(format!("{:<20}", label)),
        Span::styled(format!("{:>4}", count), theme::accent()),
        Span::raw("    "),
    ]
}

/// One status slot (12 chars total): label left-aligned in 8, value
/// right-aligned in 4. Returns 2 spans (label, value) — caller adds the gap.
fn slot(label: &'static str, value: u32, value_style: Style) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("{:<8}", label), theme::muted()),
        Span::styled(format!("{:>4}", value), value_style),
    ]
}

/// One status slot occupied by a single text chip (e.g. "all in sync"),
/// padded to 12 chars total to stay aligned with the numeric slots.
fn slot_text(text: &'static str, style: Style) -> Vec<Span<'static>> {
    vec![Span::styled(format!("{:<12}", text), style)]
}

/// Two-space gap between consecutive slots.
fn slot_gap() -> Vec<Span<'static>> {
    vec![Span::raw("  ")]
}
