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
use ratatui::widgets::{Cell, Clear, Paragraph, Row, Table, TableState};

use crate::client::{
    BulletinSnapshot, ReportingTaskRow, ReportingTaskState, ReportingTasksSnapshot,
    ValidationStatus,
};
use crate::cluster::EndpointState;
use crate::cluster::snapshot::BulletinRing;
use crate::input::{OverviewReportingTasksVerb, Verb};
use crate::theme;
use crate::view::overview::reporting_tasks_modal::{
    DetailSection, ModalFocus, ReportingTasksModalState, section_list,
};
use crate::widget::modal::LOADING_LABEL;
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
    bulletins: &BulletinRing,
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
            render_centered(frame, rows[0], LOADING_LABEL, theme::muted());
        }
        MaybeSnapshot::Ready(snapshot) => {
            let cols = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(40), Constraint::Min(1)])
                .split(rows[0]);
            render_list_pane(frame, cols[0], state, snapshot);
            render_detail_pane(frame, cols[1], state, snapshot, bulletins);
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
    let panel = Panel::new(title).focused(matches!(state.focus, ModalFocus::List));
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

/// Detects `#{name}` references in a NiFi property value.
/// Honors `##{...}` as a literal escape (NOT a real ref).
pub fn contains_param_ref(s: &str) -> bool {
    let bytes = s.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'#' {
            // Escape: ##{...} — skip past the closing brace if present.
            if i + 2 < bytes.len()
                && bytes[i + 2] == b'{'
                && let Some(close) = bytes[i + 3..].iter().position(|&b| b == b'}')
            {
                i += 3 + close + 1;
                continue;
            }
            i += 2;
            continue;
        }
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1] == b'{' {
            // Real ref: #{...}
            if bytes[i + 2..].contains(&b'}') {
                return true;
            }
        }
        i += 1;
    }
    false
}

fn render_detail_pane(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    snapshot: &ReportingTasksSnapshot,
    bulletins: &BulletinRing,
) {
    let Some(task) = state.selected_row(snapshot) else {
        // No selection (empty filter result, or empty snapshot post-reconcile).
        return;
    };
    let sections = section_list(task);
    let recent_bulletins: Vec<&BulletinSnapshot> = bulletins
        .buf
        .iter()
        .rev()
        .filter(|b| b.source_id == task.id)
        .take(10)
        .collect();

    let has_validation = !task.validation_errors.is_empty();
    let mut constraints: Vec<Constraint> = Vec::new();
    constraints.push(Constraint::Length(7)); // Identity (5 lines + 2 borders)
    constraints.push(Constraint::Min(5)); // Properties
    if has_validation {
        let h = (task
            .validation_errors
            .len()
            .min(crate::layout::VALIDATION_ERROR_ROWS_MAX)
            + 2) as u16;
        constraints.push(Constraint::Length(h));
    }
    constraints.push(Constraint::Length(5)); // Recent bulletins (3 rows + 2 borders)

    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(area);

    let mut idx = 0;
    render_identity_subpanel(frame, rows[idx], task);
    idx += 1;
    render_properties_subpanel(frame, rows[idx], state, task, sections);
    idx += 1;
    if has_validation {
        render_validation_subpanel(frame, rows[idx], state, task, sections);
        idx += 1;
    }
    render_bulletins_subpanel(frame, rows[idx], state, &recent_bulletins, sections);
}

fn render_identity_subpanel(frame: &mut Frame, area: Rect, task: &ReportingTaskRow) {
    let block = Panel::new(" Identity ").into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);
    let (state_label, state_style) = match task.state {
        ReportingTaskState::Running => ("RUNNING", theme::success()),
        ReportingTaskState::Stopped => ("STOPPED", theme::muted()),
        ReportingTaskState::Disabled => ("DISABLED", theme::muted()),
    };
    let lines = vec![
        Line::from(Span::styled(
            task.name.clone(),
            Style::default().add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::raw(truncate_chars(
            &task.task_type,
            inner.width as usize,
        ))),
        Line::from(Span::styled(format!("id  {}", task.id), theme::muted())),
        Line::from(vec![
            Span::raw(format!("{:<20}", "state")),
            Span::styled(state_label, state_style),
            Span::raw(format!("  period  {}", task.scheduling_period)),
        ]),
        Line::from(Span::raw(format!("threads  {}", task.active_thread_count))),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

fn render_properties_subpanel(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    task: &ReportingTaskRow,
    sections: &[DetailSection],
) {
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("Properties ({})", task.properties.len()),
            theme::accent(),
        ),
        Span::raw(" "),
    ]);
    let focused = matches!(
        state.focus,
        ModalFocus::Detail { idx, .. }
            if sections.get(idx) == Some(&DetailSection::Properties)
    );
    let panel = Panel::new(title).focused(focused);
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Row> = task
        .properties
        .iter()
        .map(|(name, value)| {
            let descriptor = task.descriptors.get(name);
            let sensitive = descriptor.map(|d| d.sensitive).unwrap_or(false);
            let display_name = descriptor
                .map(|d| d.display_name.as_str())
                .unwrap_or(name.as_str());
            let value_cell = if sensitive || value.is_none() {
                Cell::from(Span::styled("[sensitive]", theme::muted()))
            } else {
                let s = value.as_deref().unwrap_or_default();
                if s.is_empty() {
                    Cell::from(Span::styled("(empty)", theme::muted()))
                } else if contains_param_ref(s) {
                    Cell::from(Line::from(vec![
                        Span::raw(s.to_string()),
                        Span::styled(" →", theme::accent()),
                    ]))
                } else {
                    Cell::from(Span::raw(s.to_string()))
                }
            };
            Row::new(vec![Cell::from(display_name.to_string()), value_cell])
        })
        .collect();

    let widths = [Constraint::Length(28), Constraint::Min(1)];
    let table = Table::new(rows, widths).header(
        Row::new(vec!["KEY", "VALUE"]).style(Style::default().add_modifier(Modifier::BOLD)),
    );
    let mut ts = TableState::default();
    if let ModalFocus::Detail { idx, rows: r } = state.focus
        && focused
    {
        ts.select(Some(r[idx]));
    }
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn render_validation_subpanel(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    task: &ReportingTaskRow,
    sections: &[DetailSection],
) {
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("Validation errors ({})", task.validation_errors.len()),
            theme::warning(),
        ),
        Span::raw(" "),
    ]);
    let focused = matches!(
        state.focus,
        ModalFocus::Detail { idx, .. }
            if sections.get(idx) == Some(&DetailSection::ValidationErrors)
    );
    let panel = Panel::new(title).focused(focused);
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let rows: Vec<Row> = task
        .validation_errors
        .iter()
        .map(|e| Row::new(vec![Cell::from(e.clone())]))
        .collect();
    let widths = [Constraint::Min(1)];
    let table = Table::new(rows, widths);
    let mut ts = TableState::default();
    if let ModalFocus::Detail { idx, rows: r } = state.focus
        && focused
    {
        ts.select(Some(r[idx]));
    }
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn render_bulletins_subpanel(
    frame: &mut Frame,
    area: Rect,
    state: &ReportingTasksModalState,
    bulletins: &[&BulletinSnapshot],
    sections: &[DetailSection],
) {
    let title = Line::from(vec![
        Span::raw(" "),
        Span::styled(
            format!("Recent bulletins ({})", bulletins.len()),
            theme::accent(),
        ),
        Span::raw(" "),
    ]);
    let focused = matches!(
        state.focus,
        ModalFocus::Detail { idx, .. }
            if sections.get(idx) == Some(&DetailSection::RecentBulletins)
    );
    let panel = Panel::new(title).focused(focused);
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if bulletins.is_empty() {
        let p = Paragraph::new(Line::from(Span::styled(
            "no bulletins in ring buffer",
            theme::muted(),
        )));
        frame.render_widget(p, inner);
        return;
    }

    let rows: Vec<Row> = bulletins
        .iter()
        .map(|b| {
            Row::new(vec![
                Cell::from(b.timestamp_human.clone()),
                Cell::from(Span::styled(b.level.clone(), level_style(&b.level))),
                Cell::from(truncate_chars(&b.message, 80)),
            ])
        })
        .collect();
    let widths = [
        Constraint::Length(20),
        Constraint::Length(8),
        Constraint::Min(1),
    ];
    let table = Table::new(rows, widths);
    let mut ts = TableState::default();
    if let ModalFocus::Detail { idx, rows: r } = state.focus
        && focused
    {
        ts.select(Some(r[idx]));
    }
    frame.render_stateful_widget(table, inner, &mut ts);
}

fn level_style(level: &str) -> Style {
    if level.eq_ignore_ascii_case("ERROR") {
        theme::error()
    } else if level.eq_ignore_ascii_case("WARNING") || level.eq_ignore_ascii_case("WARN") {
        theme::warning()
    } else {
        theme::muted()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::reporting_tasks::ReportingTaskPropertyDescriptor;
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
                &BulletinRing::new(100),
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
                &BulletinRing::new(100),
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
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Loading,
                "dev",
                &BulletinRing::new(100),
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn param_ref_detection() {
        assert!(contains_param_ref("#{usd_rate}"));
        assert!(contains_param_ref("prefix #{x} suffix"));
        assert!(contains_param_ref("#{a}#{b}"));
        assert!(!contains_param_ref("##{escaped}"));
        assert!(!contains_param_ref("plain text"));
        assert!(!contains_param_ref("#{unterminated"));
        // Mixed: escape THEN real ref → must detect the real one.
        assert!(contains_param_ref("##{esc}#{real}"));
    }

    #[test]
    fn reporting_tasks_modal_detail_renders_full_content() {
        // Build a snapshot with one task that has:
        // - Three properties: one plain, one sensitive (None value), one with #{param} ref
        // - Two validation errors
        // The bulletin ring contains 3 bulletins for this task's id.
        let task_id = "task-full-001";

        let mut properties: BTreeMap<String, Option<String>> = BTreeMap::new();
        properties.insert(
            "endpoint.url".to_string(),
            Some("https://metrics.example.com/push".to_string()),
        );
        properties.insert("api.password".to_string(), None); // masked sensitive
        properties.insert(
            "metric.prefix".to_string(),
            Some("#{env_prefix}.nifi".to_string()),
        );

        let mut descriptors: BTreeMap<String, ReportingTaskPropertyDescriptor> = BTreeMap::new();
        descriptors.insert(
            "endpoint.url".to_string(),
            ReportingTaskPropertyDescriptor {
                display_name: "Endpoint URL".to_string(),
                sensitive: false,
                required: true,
                default_value: None,
            },
        );
        descriptors.insert(
            "api.password".to_string(),
            ReportingTaskPropertyDescriptor {
                display_name: "API Password".to_string(),
                sensitive: true,
                required: false,
                default_value: None,
            },
        );
        descriptors.insert(
            "metric.prefix".to_string(),
            ReportingTaskPropertyDescriptor {
                display_name: "Metric Prefix".to_string(),
                sensitive: false,
                required: false,
                default_value: None,
            },
        );

        let snapshot = ReportingTasksSnapshot {
            tasks: vec![ReportingTaskRow {
                id: task_id.to_string(),
                name: "PrometheusPushTask".to_string(),
                task_type: "org.apache.nifi.reporting.prometheus.PrometheusPushReportingTask"
                    .to_string(),
                state: ReportingTaskState::Running,
                scheduling_strategy: "TIMER_DRIVEN".to_string(),
                scheduling_period: "60s".to_string(),
                active_thread_count: 2,
                validation_status: ValidationStatus::Invalid,
                validation_errors: vec![
                    "Endpoint URL must use HTTPS scheme".to_string(),
                    "API Password is required".to_string(),
                ],
                comments: None,
                properties,
                descriptors,
            }],
            fetched_at: Instant::now(),
        };

        let modal_state = ReportingTasksModalState::open(&snapshot);

        // Build a bulletin ring with 3 bulletins for this task.
        let mut ring = BulletinRing::new(100);
        ring.merge(vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".to_string(),
                message: "Connection refused to metrics endpoint".to_string(),
                source_id: task_id.to_string(),
                source_name: "PrometheusPushTask".to_string(),
                source_type: "REPORTING_TASK".to_string(),
                group_id: String::new(),
                timestamp_iso: "2026-05-01T10:00:00Z".to_string(),
                timestamp_human: "05/01/2026 10:00:00 UTC".to_string(),
            },
            BulletinSnapshot {
                id: 2,
                level: "WARNING".to_string(),
                message: "Retry attempt 2 of 3".to_string(),
                source_id: task_id.to_string(),
                source_name: "PrometheusPushTask".to_string(),
                source_type: "REPORTING_TASK".to_string(),
                group_id: String::new(),
                timestamp_iso: "2026-05-01T10:00:05Z".to_string(),
                timestamp_human: "05/01/2026 10:00:05 UTC".to_string(),
            },
            BulletinSnapshot {
                id: 3,
                level: "INFO".to_string(),
                message: "Task started successfully".to_string(),
                source_id: "other-task-id".to_string(), // should NOT appear
                source_name: "OtherTask".to_string(),
                source_type: "REPORTING_TASK".to_string(),
                group_id: String::new(),
                timestamp_iso: "2026-05-01T10:00:10Z".to_string(),
                timestamp_human: "05/01/2026 10:00:10 UTC".to_string(),
            },
        ]);

        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Ready(&snapshot),
                "dev",
                &ring,
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn reporting_tasks_modal_properties_focused() {
        let snapshot = reporting_tasks_modal_fixture_5_tasks();
        let mut modal_state = ReportingTasksModalState::open(&snapshot);
        // Task id-1 is RUNNING+VALID with no errors, no properties.
        // Section list = [Properties, RecentBulletins]; idx 0 = Properties.
        modal_state.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Ready(&snapshot),
                "prod-cluster",
                &BulletinRing::new(100),
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn reporting_tasks_modal_bulletins_focused() {
        let snapshot = reporting_tasks_modal_fixture_5_tasks();
        let mut modal_state = ReportingTasksModalState::open(&snapshot);
        // No validation errors on id-1, so sections = [Properties,
        // RecentBulletins]; idx 1 = RecentBulletins.
        modal_state.focus = ModalFocus::Detail {
            idx: 1,
            rows: [0; 3],
        };
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Ready(&snapshot),
                "prod-cluster",
                &BulletinRing::new(100),
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }

    #[test]
    fn reporting_tasks_modal_validation_focused() {
        let snapshot = reporting_tasks_modal_fixture_5_tasks();
        let mut modal_state = ReportingTasksModalState::open(&snapshot);
        // Select task id-4 (INVALID + 2 errors). Section list =
        // [Properties, ValidationErrors, RecentBulletins]; idx 1 =
        // ValidationErrors.
        modal_state.selected_id = Some("id-4".to_string());
        modal_state.selected_ordinal = 3;
        modal_state.focus = ModalFocus::Detail {
            idx: 1,
            rows: [0; 3],
        };
        let mut term = Terminal::new(test_backend(30)).unwrap();
        term.draw(|f| {
            render(
                f,
                f.area(),
                &modal_state,
                MaybeSnapshot::Ready(&snapshot),
                "prod-cluster",
                &BulletinRing::new(100),
            );
        })
        .unwrap();
        insta::assert_snapshot!(format!("{}", term.backend()));
    }
}
