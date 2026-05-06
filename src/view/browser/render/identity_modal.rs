//! Render for the drill-in Identity modal.

use crate::theme;
use crate::view::browser::state::identity_modal::{
    IdentityModalState, IdentityStatus, ResourceBucket,
};
use crate::widget::modal::{LoadGate, render_load_gate, render_too_small};
use crate::widget::panel::Panel;
use crate::widget::search::{MatchSpan, SearchState};
use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
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

    // Reserve a single footer row for the search strip when active.
    let (body_area, footer_area) = if state.search.is_some() {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Min(0), Constraint::Length(1)])
            .split(inner);
        (chunks[0], Some(chunks[1]))
    } else {
        (inner, None)
    };

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
        lines.push(Line::from(grant_line_spans(
            &grant.resource,
            i,
            &state.search,
            row_style,
        )));
    }

    state.scroll.last_viewport_rows = body_area.height as usize;
    if let Some(target_line) = grant_line_pos.get(state.scroll.selected).copied() {
        state.scroll.scroll_to_visible(target_line);
    }
    state.scroll.clamp_to_content(lines.len());
    let scroll_offset = u16::try_from(state.scroll.offset).unwrap_or(u16::MAX);

    frame.render_widget(Paragraph::new(lines).scroll((scroll_offset, 0)), body_area);

    if let (Some(area), Some(search)) = (footer_area, state.search.as_ref()) {
        crate::widget::search::render_search_strip(frame, area, search);
    }
}

/// Build the spans for one grant line: a 2-space indent followed by
/// the resource path, with search-match highlight bands when the row
/// has matches. `line_idx` in `MatchSpan` corresponds 1:1 to the
/// grant index AND to byte offsets in the resource path — see
/// `IdentityModalState::searchable_body`.
fn grant_line_spans<'a>(
    resource: &'a str,
    row_idx: usize,
    search: &Option<SearchState>,
    row_style: Style,
) -> Vec<Span<'a>> {
    let mut spans: Vec<Span<'a>> = vec![Span::styled("  ", row_style)];
    let row_matches: Vec<&MatchSpan> = match search.as_ref() {
        Some(s) if !s.matches.is_empty() => {
            s.matches.iter().filter(|m| m.line_idx == row_idx).collect()
        }
        _ => Vec::new(),
    };
    if row_matches.is_empty() {
        spans.push(Span::styled(resource, row_style));
        return spans;
    }
    let highlight_style = row_style.patch(theme::search_match());
    let bytes = resource.as_bytes();
    let mut cursor = 0usize;
    for m in row_matches {
        let start = m.byte_start.min(bytes.len());
        let end = m.byte_end.min(bytes.len());
        if end <= cursor || end == start {
            continue;
        }
        if start > cursor {
            spans.push(Span::styled(&resource[cursor..start], row_style));
        }
        spans.push(Span::styled(&resource[start..end], highlight_style));
        cursor = end;
    }
    if cursor < resource.len() {
        spans.push(Span::styled(&resource[cursor..], row_style));
    }
    spans
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
