//! Full-screen overlay for the Overview reporting-tasks modal.
//!
//! Layout: outer Panel with title/context, body split into a left list
//! pane (~40 cols, focused by default) and a right detail pane (Tasks
//! 16-18 fill the detail body; this file scaffolds the frame). Footer
//! is the modal's verb-hint strip.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};

use crate::client::{
    ReportingTaskRow, ReportingTaskState, ReportingTasksSnapshot, ValidationStatus,
};
use crate::cluster::EndpointState;
use crate::input::{OverviewReportingTasksVerb, Verb};
use crate::theme;
use crate::view::overview::reporting_tasks_modal::{ModalPaneFocus, ReportingTasksModalState};
use crate::widget::panel::Panel;

/// A view-model wrapper around the cluster snapshot's
/// `EndpointState<ReportingTasksSnapshot>` so the renderer can pattern-
/// match cleanly.
pub enum MaybeSnapshot<'a> {
    Ready(&'a ReportingTasksSnapshot),
    Failed { last_error: &'a str },
    Loading,
}

impl<'a> MaybeSnapshot<'a> {
    pub fn from_endpoint_state(s: &'a EndpointState<ReportingTasksSnapshot>) -> Self {
        match s {
            EndpointState::Ready { data, .. } => Self::Ready(data),
            EndpointState::Failed { error, .. } => Self::Failed {
                last_error: error.as_str(),
            },
            EndpointState::Loading => Self::Loading,
        }
    }
}

pub fn render(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    data: MaybeSnapshot<'_>,
    context_label: &str,
) {
    if crate::widget::modal::render_too_small(frame, area) {
        return;
    }
    frame.render_widget(Clear, area);

    let outer_title = Line::from(vec![
        Span::raw(" "),
        Span::styled("Reporting tasks", theme::muted()),
        Span::raw(" "),
        Span::styled("·", theme::muted()),
        Span::raw(" "),
        Span::styled("context:", theme::muted()),
        Span::raw(" "),
        Span::styled(context_label.to_string(), theme::accent()),
        Span::raw(" "),
    ]);
    let outer = Panel::new(outer_title).into_block();
    let inner = outer.inner(area);
    frame.render_widget(outer, area);

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(1),    // body
            Constraint::Length(1), // hint strip
        ])
        .split(inner);

    match data {
        MaybeSnapshot::Failed { last_error } => {
            render_centered(
                frame,
                rows[0],
                &format!("failed to load reporting tasks · {last_error}"),
                theme::warning(),
            );
        }
        MaybeSnapshot::Loading => {
            render_centered(frame, rows[0], "loading…", theme::muted());
        }
        MaybeSnapshot::Ready(snapshot) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(40), Constraint::Min(1)])
                .split(rows[0]);
            render_list_pane(frame, cols[0], state, snapshot);
            render_detail_pane(frame, cols[1], state);
        }
    }
    crate::widget::modal::render_verb_hint_strip(frame, rows[1], OverviewReportingTasksVerb::all());
}

fn render_centered(frame: &mut Frame, area: Rect, msg: &str, style: Style) {
    let line = Line::from(Span::styled(msg.to_string(), style));
    let p = Paragraph::new(line).alignment(Alignment::Center);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Fill(1),
            Constraint::Length(1),
            Constraint::Fill(1),
        ])
        .split(area);
    frame.render_widget(p, rows[1]);
}

fn render_list_pane(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    snapshot: &ReportingTasksSnapshot,
) {
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(format!("Tasks ({})", snapshot.tasks.len()), theme::accent()),
        Span::raw(" "),
    ]);
    let panel = Panel::new(title).focused(matches!(state.focus, ModalPaneFocus::List));
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let mut lines: Vec<Line<'static>> = Vec::with_capacity(state.filtered_indices.len());
    for &i in &state.filtered_indices {
        let Some(task) = snapshot.tasks.get(i) else {
            continue;
        };
        let is_selected = state.selected_id.as_deref() == Some(task.id.as_str());
        lines.push(build_list_row(task, is_selected, inner.width as usize));
    }

    let total = lines.len();
    let visible = inner.height as usize;
    let offset = state.list_scroll.offset.min(total.saturating_sub(visible));
    let end = (offset + visible).min(total);
    let visible_lines: Vec<Line<'static>> = lines
        .get(offset..end)
        .map(|s| s.to_vec())
        .unwrap_or_default();

    frame.render_widget(Paragraph::new(visible_lines), inner);
}

fn build_list_row(task: &ReportingTaskRow, selected: bool, _width: usize) -> Line<'static> {
    let icon = state_icon_span(task);
    let validation = validation_chip_span(task);
    let threads = format!("{}t", task.active_thread_count);
    let period = truncate_chars(&task.scheduling_period, 6);
    let name = truncate_chars(&task.name, 20);

    let row_style = if selected {
        Style::default().add_modifier(Modifier::REVERSED)
    } else {
        Style::default()
    };

    Line::from(vec![
        icon,
        Span::raw(" "),
        Span::styled(format!("{name:<20}"), row_style),
        Span::raw(" "),
        Span::styled(format!("{period:<6}"), row_style),
        Span::raw(" "),
        Span::styled(format!("{threads:<4}"), row_style),
        Span::raw(" "),
        validation,
    ])
}

fn state_icon_span(task: &ReportingTaskRow) -> Span<'static> {
    match (task.state, task.validation_status) {
        (_, ValidationStatus::Invalid) => Span::styled("!", theme::warning()),
        (ReportingTaskState::Running, _) => Span::styled("●", theme::success()),
        (ReportingTaskState::Stopped, _) | (ReportingTaskState::Disabled, _) => {
            Span::styled("○", theme::muted())
        }
    }
}

fn validation_chip_span(task: &ReportingTaskRow) -> Span<'static> {
    match task.validation_status {
        ValidationStatus::Valid => Span::styled("✓", theme::success()),
        ValidationStatus::Invalid => Span::styled(
            format!("⚠ {}", task.validation_errors.len()),
            theme::warning(),
        ),
        ValidationStatus::Validating => Span::styled("…", theme::muted()),
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    let count = s.chars().count();
    if count <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// Stub renderer for the detail pane — Tasks 16-18 fill the body.
fn render_detail_pane(frame: &mut Frame, area: Rect, state: &ReportingTasksModalState) {
    let panel = Panel::new(" Detail ").focused(matches!(state.focus, ModalPaneFocus::Detail));
    let block = panel.into_block();
    frame.render_widget(block, area);
    // Body intentionally blank for Tasks 14/15. Tasks 16+ will populate.
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::test_backend;
    use ratatui::Terminal;
    use std::collections::BTreeMap;
    use std::time::Instant;

    /// Build a 5-task snapshot mirroring the wiremock `full.json` fixture:
    /// 2 RUNNING+VALID, 1 STOPPED, 1 DISABLED, 1 RUNNING+INVALID.
    pub(super) fn reporting_tasks_modal_fixture_5_tasks() -> ReportingTasksSnapshot {
        ReportingTasksSnapshot {
            tasks: vec![
                task(
                    "id-1",
                    "PrometheusReportingTask",
                    "org.apache.nifi.reporting.prometheus.PrometheusReportingTask",
                    ReportingTaskState::Running,
                    ValidationStatus::Valid,
                    "30s",
                    1,
                    vec![],
                ),
                task(
                    "id-2",
                    "S2S Bulletin Reporter",
                    "org.apache.nifi.reporting.SiteToSiteBulletinReportingTask",
                    ReportingTaskState::Running,
                    ValidationStatus::Valid,
                    "1m",
                    0,
                    vec![],
                ),
                task(
                    "id-3",
                    "MonitorMemory",
                    "org.apache.nifi.controller.MonitorMemory",
                    ReportingTaskState::Stopped,
                    ValidationStatus::Valid,
                    "5m",
                    0,
                    vec![],
                ),
                task(
                    "id-4",
                    "MonitorDiskUsage",
                    "org.apache.nifi.controller.MonitorDiskUsage",
                    ReportingTaskState::Running,
                    ValidationStatus::Invalid,
                    "5m",
                    0,
                    vec![
                        "Property 'Directory Location' is required but is not set".to_string(),
                        "Property 'Threshold' must be a valid percent value".to_string(),
                    ],
                ),
                task(
                    "id-5",
                    "StatusHistoryReportingTask",
                    "org.apache.nifi.reporting.StandardReportingTask",
                    ReportingTaskState::Disabled,
                    ValidationStatus::Valid,
                    "30s",
                    0,
                    vec![],
                ),
            ],
            fetched_at: Instant::now(),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn task(
        id: &str,
        name: &str,
        task_type: &str,
        state: ReportingTaskState,
        validation_status: ValidationStatus,
        period: &str,
        threads: u32,
        errors: Vec<String>,
    ) -> ReportingTaskRow {
        ReportingTaskRow {
            id: id.into(),
            name: name.into(),
            task_type: task_type.into(),
            state,
            scheduling_strategy: "TIMER_DRIVEN".into(),
            scheduling_period: period.into(),
            active_thread_count: threads,
            validation_status,
            validation_errors: errors,
            comments: None,
            properties: BTreeMap::new(),
            descriptors: BTreeMap::new(),
        }
    }

    #[test]
    fn reporting_tasks_modal_renders_ready() {
        let snapshot = reporting_tasks_modal_fixture_5_tasks();
        let modal_state = ReportingTasksModalState::open(&snapshot);
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Ready(&snapshot),
                "prod-cluster",
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn reporting_tasks_modal_renders_failed() {
        let modal_state = ReportingTasksModalState::default();
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Failed {
                    last_error: "connection refused",
                },
                "prod-cluster",
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn reporting_tasks_modal_renders_loading() {
        let modal_state = ReportingTasksModalState::default();
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(f, f.area(), &modal_state, MaybeSnapshot::Loading, "dev");
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }
}
