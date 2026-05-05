//! Render for the per-component Access matrix modal.

use crate::theme;
use crate::view::browser::state::access_modal::{AccessModalState, Axis, MatrixCell, ModalStatus};
use crate::widget::modal::{LoadGate, render_load_gate, render_too_small};
use ratatui::Frame;
use ratatui::layout::Rect;
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

    // Header + legend each take 1 row; the rest is data rows.
    let visible_rows = inner.height.saturating_sub(2) as usize;
    let selected = state.scroll.selected;
    let matrix_len = state.matrix.len();
    state.scroll.last_viewport_rows = visible_rows;
    state.scroll.scroll_to_visible(selected);
    state.scroll.clamp_to_content(matrix_len);
    let header_off = state.scroll.offset;

    let mut lines: Vec<Line<'_>> = Vec::with_capacity(visible_rows + 2);

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
        let mut spans: Vec<Span<'_>> = vec![Span::styled(
            format!("{:<28}", row.tenant.identity),
            row_style,
        )];
        for axis in Axis::ALL {
            let cell = row.cells.get(&axis).map(MatrixCell::glyph).unwrap_or("—");
            spans.push(Span::styled(format!("  {:<5}", cell), row_style));
        }
        lines.push(Line::from(spans));
    }

    // Legend row at the bottom
    lines.push(Line::from(Span::styled(
        "legend  ✓ explicit · ↑ inherited · — none · ? error",
        theme::muted(),
    )));

    frame.render_widget(Paragraph::new(lines), inner);
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
