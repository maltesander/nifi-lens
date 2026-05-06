//! Render for the per-component Access matrix modal.

use crate::theme;
use crate::view::browser::state::access_modal::{AccessModalState, Axis, MatrixCell, ModalStatus};
use crate::widget::modal::{LoadGate, render_load_gate, render_too_small};
use crate::widget::search::{MatchSpan, SearchState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::widget::panel::Panel;

pub fn render_access_modal(frame: &mut Frame, area: Rect, state: &mut AccessModalState) {
    if render_too_small(frame, area) {
        return;
    }

    frame.render_widget(Clear, area);

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Access", theme::muted()),
        Span::raw(" · "),
        Span::styled(state.component_label.as_str(), theme::accent()),
        Span::raw(" "),
    ]);
    let block = Panel::new(title).into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let gate = match &state.status {
        ModalStatus::Loading => LoadGate::Loading,
        ModalStatus::Failed(err) => LoadGate::Failed(err),
        ModalStatus::Loaded => LoadGate::Loaded,
    };
    if render_load_gate(frame, inner, gate) {
        return;
    }

    // Reserve a single footer row for either the search strip (when
    // active) or the legend. Both occupy one row, so the matrix area
    // stays the same regardless. Header takes one row above.
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(0), Constraint::Length(1)])
        .split(inner);
    let matrix_area = chunks[0];
    let footer_area = chunks[1];

    let visible_rows = matrix_area.height.saturating_sub(1) as usize;
    let selected = state.scroll.selected;
    let matrix_len = state.matrix.len();
    state.scroll.last_viewport_rows = visible_rows;
    state.scroll.scroll_to_visible(selected);
    state.scroll.clamp_to_content(matrix_len);
    let header_off = state.scroll.offset;

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(visible_rows + 1);

    // Header row — must match the per-cell format below so columns align.
    let mut header_spans: Vec<Span<'_>> =
        vec![Span::styled(format!("{:<28}", "identity"), theme::muted())];
    for axis in Axis::ALL {
        header_spans.push(Span::styled(
            format!("  {:<5}", axis.header()),
            theme::muted(),
        ));
    }
    lines.push(Line::from(header_spans));

    for (idx, row) in state
        .matrix
        .iter()
        .enumerate()
        .skip(header_off)
        .take(visible_rows)
    {
        let row_style = if idx == state.scroll.selected {
            theme::cursor_row()
        } else if row.is_group {
            theme::group()
        } else {
            Style::default()
        };
        let identity_cell =
            highlighted_identity_cell(&row.tenant.identity, idx, &state.search, row_style);
        let mut spans: Vec<Span<'_>> = identity_cell;
        for axis in Axis::ALL {
            let cell = row.cells.get(&axis).map(MatrixCell::glyph).unwrap_or("—");
            spans.push(Span::styled(format!("  {:<5}", cell), row_style));
        }
        lines.push(Line::from(spans));
    }

    frame.render_widget(Paragraph::new(lines), matrix_area);

    if let Some(search) = state.search.as_ref() {
        crate::widget::search::render_search_strip(frame, footer_area, search);
    } else {
        let legend = Paragraph::new(Line::from(Span::styled(
            "legend  ✓ explicit · ↑ inherited · — none · ? error",
            theme::muted(),
        )));
        frame.render_widget(legend, footer_area);
    }
}

/// Build the spans for the 28-column identity cell, applying
/// search-match highlight bands when search is active and the row has
/// matches. The padded width is preserved by adding a final `Span` of
/// trailing spaces if the identity is shorter than 28 columns.
fn highlighted_identity_cell<'a>(
    identity: &'a str,
    row_idx: usize,
    search: &Option<SearchState>,
    base_style: Style,
) -> Vec<Span<'a>> {
    const WIDTH: usize = 28;
    let row_matches: Vec<&MatchSpan> = match search.as_ref() {
        Some(s) if !s.matches.is_empty() => {
            s.matches.iter().filter(|m| m.line_idx == row_idx).collect()
        }
        _ => Vec::new(),
    };
    if row_matches.is_empty() {
        // No matches on this row — keep the existing single-span format.
        let padded = format!("{:<width$}", identity, width = WIDTH);
        return vec![Span::styled(padded, base_style)];
    }
    let highlight_style = base_style.patch(theme::search_match());
    let mut spans: Vec<Span<'a>> = Vec::new();
    let bytes = identity.as_bytes();
    let mut cursor = 0usize;
    for m in row_matches {
        let start = m.byte_start.min(bytes.len());
        let end = m.byte_end.min(bytes.len());
        if start > cursor {
            spans.push(Span::styled(&identity[cursor..start], base_style));
        }
        if end > start {
            spans.push(Span::styled(&identity[start..end], highlight_style));
        }
        cursor = end;
    }
    if cursor < identity.len() {
        spans.push(Span::styled(&identity[cursor..], base_style));
    }
    let used_cols = identity.chars().count();
    if used_cols < WIDTH {
        let pad = " ".repeat(WIDTH - used_cols);
        spans.push(Span::styled(pad, base_style));
    }
    spans
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::NodeKind;
    use crate::client::access::AccessFetchResult;
    use crate::test_support::{TEST_BACKEND_WIDTH, test_backend};
    use crate::view::browser::state::access_modal::{AxisOutcome, TenantRef};
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn alice() -> TenantRef {
        TenantRef {
            id: "u1".into(),
            identity: "alice@corp".into(),
            member_count: None,
        }
    }

    fn bob() -> TenantRef {
        TenantRef {
            id: "u2".into(),
            identity: "bob@corp".into(),
            member_count: None,
        }
    }

    fn ops() -> TenantRef {
        TenantRef {
            id: "g1".into(),
            identity: "ops-team".into(),
            member_count: Some(12),
        }
    }

    fn loaded_state() -> AccessModalState {
        let mut state =
            AccessModalState::new("p1".into(), NodeKind::Processor, "EnrichOrders".into());
        let mut result = AccessFetchResult::default();
        result.outcomes.insert(
            Axis::ViewComponent,
            AxisOutcome::Direct {
                users: vec![alice(), bob()],
                groups: vec![ops()],
            },
        );
        result.outcomes.insert(
            Axis::ModifyComponent,
            AxisOutcome::Direct {
                users: vec![alice()],
                groups: vec![ops()],
            },
        );
        result.outcomes.insert(
            Axis::ViewData,
            AxisOutcome::Inherited {
                source: "/process-groups/orders".into(),
                users: vec![alice()],
                groups: vec![],
            },
        );
        result.outcomes.insert(
            Axis::Operate,
            AxisOutcome::Inherited {
                source: "/process-groups/orders".into(),
                users: vec![alice()],
                groups: vec![ops()],
            },
        );
        result.outcomes.insert(
            Axis::ManagePolicies,
            AxisOutcome::Inherited {
                source: "/process-groups/root".into(),
                users: vec![],
                groups: vec![],
            },
        );
        state.apply_fetch(result);
        state
    }

    #[test]
    fn snapshot_loaded_matrix() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, 20);
            render_access_modal(f, area, &mut state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_loading_state() {
        let mut term = Terminal::new(test_backend(20)).unwrap();
        let mut state =
            AccessModalState::new("p1".into(), NodeKind::Processor, "EnrichOrders".into());
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, 20);
            render_access_modal(f, area, &mut state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_too_small() {
        // Below MIN_WIDTH × MIN_HEIGHT (60 × 20) the render degrades.
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, 40, 10);
            render_access_modal(f, area, &mut state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    // Verify TEST_BACKEND_WIDTH is used as the standard width.
    const _: () = assert!(TEST_BACKEND_WIDTH == 100);
}
