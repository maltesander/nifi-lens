//! Render for the drill-in Identity modal.

use crate::theme;
use crate::view::browser::state::identity_modal::{IdentityModalState, IdentityStatus};
use crate::widget::modal::render_too_small;
use crate::widget::panel::Panel;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

pub fn render_identity_modal(frame: &mut Frame, area: Rect, state: &IdentityModalState) {
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

    match &state.status {
        IdentityStatus::Loading => {
            frame.render_widget(
                Paragraph::new(Span::styled("loading…", theme::muted())),
                inner,
            );
            return;
        }
        IdentityStatus::Failed(err) => {
            frame.render_widget(
                Paragraph::new(Span::styled(format!("failed: {err}"), theme::error())),
                inner,
            );
            return;
        }
        IdentityStatus::Loaded => {}
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

    for (bucket, grants) in state.grouped_by_bucket() {
        lines.push(Line::from(Span::styled(
            format!("▸ {}", bucket.header()),
            theme::accent(),
        )));
        for grant in grants {
            lines.push(Line::from(vec![
                Span::raw("  "),
                Span::raw(grant.resource.clone()),
            ]));
        }
        lines.push(Line::raw(""));
    }

    frame.render_widget(Paragraph::new(lines), inner);
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
        let state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, TEST_BACKEND_WIDTH, 22);
            render_identity_modal(f, area, &state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn snapshot_drill_in_too_small() {
        let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
        let state = loaded_state();
        term.draw(|f| {
            let area = Rect::new(0, 0, 40, 10);
            render_identity_modal(f, area, &state);
        })
        .unwrap();
        assert_snapshot!(format!("{}", term.backend()));
    }
}
