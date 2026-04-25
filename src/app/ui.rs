//! Top-level frame renderer: lays out chrome + current tab + any modal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::widgets::{Block, Borders};

use crate::app::state::{AppState, Modal, ViewId};
use crate::view::{browser, bulletins, events, overview, tracer};
use crate::widget::{context_switcher, help_modal, status_bar};

pub fn render(frame: &mut Frame, state: &mut AppState) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1), // top bar (tabs + identity)
            Constraint::Fill(1),   // content
            Constraint::Length(1), // footer row 1: banner + refresh age
            Constraint::Length(1), // footer row 2: hint bar
        ])
        .split(root);

    crate::widget::top_bar::render(frame, chunks[0], state);
    render_content(frame, chunks[1], state);
    status_bar::render(frame, chunks[2], state);

    let hints = crate::app::state::collect_hints(state);
    crate::widget::hint_bar::render(frame, chunks[3], &hints);

    if let Some(modal) = &state.modal {
        match modal {
            Modal::Help => help_modal::render(
                frame,
                root,
                state.current_tab,
                state.tracer.content_modal.is_some(),
            ),
            Modal::ContextSwitcher(cs) => context_switcher::render(frame, root, cs),
            Modal::ErrorDetail => render_error_detail(frame, root, state),
            Modal::FuzzyFind(fs) => {
                crate::widget::fuzzy_find::render(frame, frame.area(), fs, &state.flow_index);
            }
            Modal::Properties(ps) => {
                crate::view::browser::render::render_properties_modal(
                    frame,
                    frame.area(),
                    ps,
                    &state.browser,
                );
            }
            Modal::SaveEventContent(save) => {
                crate::widget::save_modal::render(frame, frame.area(), save);
            }
            Modal::NodeDetail(row) => {
                crate::view::overview::render::render_node_detail_modal(frame, frame.area(), row);
            }
            Modal::GotoMenu(jm) => {
                crate::widget::goto_menu::render(frame, frame.area(), jm);
            }
        }
    }
}

fn render_content(frame: &mut Frame, area: Rect, state: &mut AppState) {
    match state.current_tab {
        ViewId::Overview => overview::render(frame, area, &state.overview),
        ViewId::Bulletins => {
            let browser = &state.browser;
            let cfg = &state.timestamp_cfg;
            bulletins::render(frame, area, &mut state.bulletins, browser, cfg);
        }
        ViewId::Browser => browser::render(
            frame,
            area,
            &state.browser,
            &state.flow_index,
            &state.bulletins.ring,
            &state.cluster.snapshot,
        ),
        ViewId::Events => events::render::render(frame, area, &state.events, &state.timestamp_cfg),
        ViewId::Tracer => {
            tracer::render(frame, area, &state.tracer, &state.timestamp_cfg);
            if let Some(modal) = state.tracer.content_modal.as_mut() {
                crate::view::tracer::modal::render(
                    frame,
                    area,
                    modal,
                    &state.tracer_config.ceiling,
                );
            }
        }
    }
}

fn render_error_detail(frame: &mut Frame, area: Rect, state: &AppState) {
    use ratatui::widgets::{Clear, Paragraph};
    let text = state.error_detail.clone().unwrap_or_default();
    let block = Block::default()
        .title(" Error detail (e/Esc to close) ")
        .borders(Borders::ALL);
    let p = Paragraph::new(text).block(block);
    let modal = center(
        area,
        crate::layout::ERROR_DETAIL_MODAL_WIDTH_PCT,
        crate::layout::ERROR_DETAIL_MODAL_HEIGHT,
    );
    frame.render_widget(Clear, modal);
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
    use crate::test_support::fresh_state;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn top_bar_renders_on_row_zero() {
        let mut state = fresh_state();
        state.context_name = "dev-nifi-2-9-0".into();
        let backend = TestBackend::new(100, 25);
        let mut term = Terminal::new(backend).unwrap();
        term.draw(|f| render(f, &mut state)).unwrap();
        let snapshot = format!("{}", term.backend());
        let first_line = snapshot.lines().next().unwrap();
        assert!(
            first_line.contains("Overview"),
            "first line missing tab bar: {first_line:?}"
        );
        assert!(
            first_line.contains("[dev-nifi-2-9-0]"),
            "first line missing identity strip: {first_line:?}"
        );
    }
}
