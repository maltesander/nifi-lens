//! Top-level frame renderer: lays out chrome + current tab + any modal.

use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Style};
use ratatui::text::Line;
use ratatui::widgets::{Block, Borders, Tabs};

use crate::app::state::{AppState, Modal, ViewId};
use crate::view::{browser, bulletins, health, overview, tracer};
use crate::widget::{context_switcher, help_modal, status_bar};

pub fn render(frame: &mut Frame, state: &AppState) {
    let root = frame.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Fill(1),   // content
            Constraint::Length(1), // status bar
        ])
        .split(root);

    render_tab_bar(frame, chunks[0], state);
    render_content(frame, chunks[1], state);
    status_bar::render(frame, chunks[2], state);

    if let Some(modal) = &state.modal {
        match modal {
            Modal::Help => help_modal::render(frame, root, state.current_tab),
            Modal::ContextSwitcher(cs) => context_switcher::render(frame, root, cs),
            Modal::ErrorDetail => render_error_detail(frame, root, state),
            Modal::FuzzyFind(fs) => {
                crate::view::browser::render::render_fuzzy_find_modal(
                    frame,
                    frame.area(),
                    fs,
                    &state.flow_index,
                );
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
        }
    }
}

fn render_tab_bar(frame: &mut Frame, area: Rect, state: &AppState) {
    let titles = vec![
        Line::from("Overview"),
        Line::from("Bulletins"),
        Line::from("Browser"),
        Line::from("Tracer"),
        Line::from("Health"),
    ];
    let idx = match state.current_tab {
        ViewId::Overview => 0,
        ViewId::Bulletins => 1,
        ViewId::Browser => 2,
        ViewId::Tracer => 3,
        ViewId::Health => 4,
    };
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" nifi-lens "))
        .select(idx)
        .highlight_style(Style::default().fg(Color::Cyan));
    frame.render_widget(tabs, area);
}

fn render_content(frame: &mut Frame, area: Rect, state: &AppState) {
    match state.current_tab {
        ViewId::Overview => overview::render(frame, area, &state.overview),
        ViewId::Bulletins => bulletins::render(frame, area, &state.bulletins),
        ViewId::Browser => browser::render(frame, area, &state.browser, &state.flow_index),
        ViewId::Tracer => tracer::render(frame, area, &state.tracer),
        ViewId::Health => health::render(frame, area, &state.health),
    }
}

fn render_error_detail(frame: &mut Frame, area: Rect, state: &AppState) {
    use ratatui::widgets::{Clear, Paragraph};
    let text = state.error_detail.clone().unwrap_or_default();
    let block = Block::default()
        .title(" Error detail (e/Esc to close) ")
        .borders(Borders::ALL);
    let p = Paragraph::new(text).block(block);
    let modal = center(area, 80, 15);
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
