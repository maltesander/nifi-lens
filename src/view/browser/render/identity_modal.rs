//! Render for the drill-in Identity modal.

use crate::theme;
use crate::view::browser::state::identity_modal::{
    IdentityModalState, IdentityStatus, ResourceBucket,
};
use crate::widget::modal::{LoadGate, render_load_gate, render_too_small};
use crate::widget::panel::Panel;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub fn render_identity_modal(frame: &mut Frame, area: Rect, state: &mut IdentityModalState) {
    if render_too_small(frame, area) {
        return;
    }

    frame.render_widget(Clear, area);

    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Identity", theme::muted()),
        Span::raw(" · "),
        Span::styled(state.identity.as_str(), theme::accent()),
        Span::raw(" "),
    ]);
    let block = Panel::new(title).into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let gate = match &state.status {
        IdentityStatus::Loading => LoadGate::Loading,
        IdentityStatus::Failed(err) => LoadGate::Failed(err),
        IdentityStatus::Loaded => LoadGate::Loaded,
    };
    if render_load_gate(frame, inner, gate) {
        return;
    }

    let mut lines: Vec<Line<'_>> = Vec::new();
    lines.push(Line::from(Span::styled(
        format!(
            "user · {} group memberships · {} grants",
            state.group_memberships.len(),
            state.grants.len(),
        ),
        theme::muted(),
    )));
    lines.push(Line::raw(""));

    // Walk grants in canonical (post-sort) order, emitting section
    // headers on bucket transitions, and recording each grant's line
    // position so the renderer can scroll the selected grant into view.
    let mut grant_line_pos: Vec<usize> = Vec::with_capacity(state.grants.len());
    let mut current_bucket: Option<ResourceBucket> = None;
    for (i, grant) in state.grants.iter().enumerate() {
        if Some(grant.bucket) != current_bucket {
            if current_bucket.is_some() {
                lines.push(Line::raw(""));
            }
            lines.push(Line::from(Span::styled(
                format!("▸ {}", grant.bucket.header()),
                theme::accent(),
            )));
            current_bucket = Some(grant.bucket);
        }
        grant_line_pos.push(lines.len());
        let row_style = if i == state.scroll.selected {
            theme::cursor_row()
        } else {
            Style::default()
        };
        lines.push(Line::from(vec![
            Span::styled("  ", row_style),
            Span::styled(grant.resource.clone(), row_style),
        ]));
    }

    state.scroll.last_viewport_rows = inner.height as usize;
    if let Some(target_line) = grant_line_pos.get(state.scroll.selected).copied() {
        state.scroll.scroll_to_visible(target_line);
    }
    state.scroll.clamp_to_content(lines.len());
    let scroll_offset = u16::try_from(state.scroll.offset).unwrap_or(u16::MAX);

    frame.render_widget(Paragraph::new(lines).scroll((scroll_offset, 0)), inner);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::access::IdentityFetchResult;
    use crate::test_support::{TEST_BACKEND_WIDTH, test_backend};
    use crate::view::browser::state::identity_modal::{
        GrantSource, IdentityGrant, IdentityKind, ResourceBucket, axis_from_action_and_resource,
    };
    use insta::assert_snapshot;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn grant(resource: &str, action: &str, bucket: ResourceBucket) -> IdentityGrant {
        IdentityGrant {
            axis: axis_from_action_and_resource(action, resource),
            resource: resource.into(),
            bucket,
            source: GrantSource::Direct,
        }
    }

    fn loaded_state() -> IdentityModalState {
        let mut state =
            IdentityModalState::pending(IdentityKind::User, "u1".into(), "alice@corp".into());
        let result = IdentityFetchResult {
            identity: "alice@corp".into(),
            grants: vec![
                grant(
                    "/process-groups/orders",
                    "read",
                    ResourceBucket::ProcessGroups,
                ),
                grant(
                    "/process-groups/orders",
                    "write",
                    ResourceBucket::ProcessGroups,
                ),
                grant(
                    "/processors/EnrichOrders",
                    "read",
                    ResourceBucket::Processors,
                ),
                grant(
                    "/controller-services/SslContext",
                    "read",
                    ResourceBucket::ControllerServices,
                ),
                grant("/flow", "read", ResourceBucket::Global),
            ],
            group_memberships: vec!["ops-team".into()],
        };
        state.apply_fetch(result);
        state
    }

    #[test]
    fn snapshot_loaded_drill_in() {
        let mut term = Terminal::new(test_backend(22)).unwrap();
        let mut state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, 22);
            render_identity_modal(f, area, &mut state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_drill_in_too_small() {
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let mut state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, 40, 10);
            render_identity_modal(f, area, &mut state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }
}
