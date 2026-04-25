//! Version-control modal renderer. Identity panel + diff body + footer.
//!
//! Task 20 ships the scaffold and Identity panel; Tasks 21-23 fill in
//! the diff body, footer hint bar, and search highlights.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph};

use crate::theme;
use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
use crate::widget::panel::Panel;

const MIN_WIDTH: u16 = 60;
const MIN_HEIGHT: u16 = 20;

pub fn render(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    if area.width < MIN_WIDTH || area.height < MIN_HEIGHT {
        let line = Line::from(Span::styled("terminal too small", theme::muted()));
        frame.render_widget(Clear, area);
        frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        return;
    }

    frame.render_widget(Clear, area);
    let outer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled(modal.pg_name.as_str(), theme::accent()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("version control", theme::muted()),
        Span::raw(" "),
    ]);
    let outer = Panel::new(outer_title).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6),
            Constraint::Fill(1),
            Constraint::Length(2),
        ])
        .split(inner);

    render_identity(frame, rows[0], modal);
    render_diff_body(frame, rows[1], modal);
    render_footer(frame, rows[2], modal);
}

fn render_identity(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    let block = Block::default().borders(Borders::ALL).title(" Identity ");
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let id = match &modal.identity {
        Some(s) => s,
        None => {
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled("loading…", theme::muted()))),
                inner,
            );
            return;
        }
    };
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState as S;
    let state_word = match id.state {
        S::UpToDate => "UP TO DATE",
        S::Stale => "STALE",
        S::LocallyModified => "MODIFIED",
        S::LocallyModifiedAndStale => "STALE+MOD",
        S::SyncFailure => "SYNC-ERR",
        _ => "UNKNOWN",
    };
    let state_style: Style = match id.state {
        S::UpToDate => theme::muted(),
        S::Stale | S::LocallyModified | S::LocallyModifiedAndStale => theme::warning(),
        S::SyncFailure => theme::error(),
        _ => theme::muted(),
    };
    let registry = format!(
        "{} / bucket={}{}",
        id.registry_name.as_deref().unwrap_or("?"),
        id.bucket_name.as_deref().unwrap_or("?"),
        id.branch
            .as_deref()
            .map(|b| format!(" / branch={b}"))
            .unwrap_or_default(),
    );
    let flow = format!(
        "{} · flow_id={} · v{}",
        id.flow_name.as_deref().unwrap_or("?"),
        short_id(id.flow_id.as_deref().unwrap_or("?")),
        id.version.as_deref().unwrap_or("?"),
    );
    let why = id.state_explanation.as_deref().unwrap_or("—");
    let lines = vec![
        Line::from(vec![
            Span::styled(format!("{:<10}", "registry"), theme::muted()),
            Span::raw(registry),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "flow"), theme::muted()),
            Span::raw(flow),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "state"), theme::muted()),
            Span::styled(state_word, state_style.add_modifier(Modifier::BOLD)),
        ]),
        Line::from(vec![
            Span::styled(format!("{:<10}", "why"), theme::muted()),
            Span::styled(why.to_string(), theme::muted()),
        ]),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn short_id(id: &str) -> String {
    if id.len() <= 4 {
        id.to_string()
    } else {
        format!("{}…", &id[..4])
    }
}

fn render_diff_body(frame: &mut Frame, area: Rect, modal: &VersionControlModalState) {
    // Task 20: render Pending state only. Tasks 21/23 fill in the rest.
    match &modal.differences {
        VersionControlDifferenceLoad::Pending => {
            let line = Line::from(Span::styled("loading…", theme::muted()));
            frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        }
        _ => {
            // Stub for Task 20 — Task 21 implements Loaded/Empty/Failed bodies.
            let line = Line::from(Span::styled("(diff body — Task 21)", theme::muted()));
            frame.render_widget(Paragraph::new(line).alignment(Alignment::Center), area);
        }
    }
}

fn render_footer(frame: &mut Frame, area: Rect, _modal: &VersionControlModalState) {
    // Task 20: minimal footer. Task 22 implements the full status + hint bar.
    let line = Line::from(Span::styled(
        "press esc to close · / search · e env · r refresh · c copy",
        theme::muted(),
    ));
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::snapshot::VersionControlSummary;
    use crate::test_support::test_backend;
    use crate::view::browser::state::VersionControlModalState;
    use insta::assert_snapshot;
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
    use ratatui::Terminal;

    fn modal_with_identity(state: VersionControlInformationDtoState) -> VersionControlModalState {
        VersionControlModalState::pending(
            "pg-1".into(),
            "ingest".into(),
            Some(VersionControlSummary {
                state,
                registry_name: Some("ops-registry".into()),
                bucket_name: Some("ops".into()),
                branch: Some("main".into()),
                flow_id: Some("4f3a-aaaa".into()),
                flow_name: Some("diff-pipeline".into()),
                version: Some("3".into()),
                state_explanation: Some(
                    "Local changes have been made and a newer version exists".into(),
                ),
            }),
        )
    }

    #[test]
    fn renders_identity_for_stale_modified() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::LocallyModifiedAndStale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_identity_stale_modified",
            format!("{}", term.backend())
        );
    }

    #[test]
    fn pending_diff_body_shows_loading() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::Stale);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!("vc_modal_pending_loading", format!("{}", term.backend()));
    }

    #[test]
    fn renders_identity_for_sync_failure() {
        let mut term = Terminal::new(test_backend(24)).unwrap();
        let modal = modal_with_identity(VersionControlInformationDtoState::SyncFailure);
        term.draw(|f| render(f, f.area(), &modal)).unwrap();
        assert_snapshot!(
            "vc_modal_identity_sync_failure",
            format!("{}", term.backend())
        );
    }
}
