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
    BulletinSnapshot, ReportingTaskRow, ReportingTaskState, ReportingTasksSnapshot,
    ValidationStatus,
};
use crate::cluster::EndpointState;
use crate::cluster::snapshot::BulletinRing;
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
            render_centered(frame, rows[0], "loading…", theme::muted());
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
    let panel = Panel::new(" Detail ").focused(matches!(state.focus, ModalPaneFocus::Detail));
    let block = panel.into_block();
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(task) = state.selected_row(snapshot) else {
        // No selection (empty filter result, or empty snapshot post-reconcile).
        // Snapshot tests for these states are covered by the empty/failed
        // frame snapshots; the detail pane just stays empty.
        return;
    };

    let mut lines: Vec<Line<'static>> = Vec::new();
    push_identity(&mut lines, task, inner.width as usize);
    lines.push(Line::raw(""));
    push_scheduling(&mut lines, task);
    push_properties(&mut lines, task);
    push_validation_errors(&mut lines, task);
    push_recent_bulletins(&mut lines, task, bulletins);

    // Apply scroll
    let total = lines.len();
    let visible = inner.height as usize;
    let offset = state
        .detail_scroll
        .offset
        .min(total.saturating_sub(visible));
    let end = (offset + visible).min(total);
    let visible_lines: Vec<Line<'static>> = lines
        .get(offset..end)
        .map(|s| s.to_vec())
        .unwrap_or_default();

    frame.render_widget(Paragraph::new(visible_lines), inner);
}

fn push_identity(lines: &mut Vec<Line<'static>>, task: &ReportingTaskRow, width: usize) {
    lines.push(Line::from(Span::styled(
        task.name.clone(),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::raw(truncate_chars(
        &task.task_type,
        width,
    ))));
    lines.push(Line::from(Span::styled(
        format!("id  {}", task.id),
        theme::muted(),
    )));
}

fn push_scheduling(lines: &mut Vec<Line<'static>>, task: &ReportingTaskRow) {
    lines.push(Line::from(Span::styled(
        "Scheduling",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    lines.push(kv_line("strategy", &task.scheduling_strategy));
    lines.push(kv_line("period", &task.scheduling_period));
    let (state_label, state_style) = match task.state {
        ReportingTaskState::Running => ("RUNNING", theme::success()),
        ReportingTaskState::Stopped => ("STOPPED", theme::muted()),
        ReportingTaskState::Disabled => ("DISABLED", theme::muted()),
    };
    lines.push(Line::from(vec![
        Span::raw(format!("  {:<20}", "state")),
        Span::styled(state_label, state_style),
    ]));
    lines.push(kv_line(
        "active threads",
        &task.active_thread_count.to_string(),
    ));
}

fn push_properties(lines: &mut Vec<Line<'static>>, task: &ReportingTaskRow) {
    if task.properties.is_empty() {
        return;
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Properties",
        Style::default().add_modifier(Modifier::BOLD),
    )));
    for (name, value) in &task.properties {
        let descriptor = task.descriptors.get(name);
        let sensitive = descriptor.map(|d| d.sensitive).unwrap_or(false);
        let display_name = descriptor
            .map(|d| d.display_name.as_str())
            .unwrap_or(name.as_str());
        let mut spans = vec![Span::raw(format!(
            "  {:<28}",
            truncate_chars(display_name, 28)
        ))];
        if sensitive || value.is_none() {
            spans.push(Span::styled("[sensitive]", theme::muted()));
        } else {
            match value.as_deref() {
                Some("") | None => spans.push(Span::styled("(empty)", theme::muted())),
                Some(s) => {
                    spans.push(Span::raw(s.to_string()));
                    if contains_param_ref(s) {
                        spans.push(Span::styled(" →", theme::accent()));
                    }
                }
            }
        }
        lines.push(Line::from(spans));
    }
}

fn push_validation_errors(lines: &mut Vec<Line<'static>>, task: &ReportingTaskRow) {
    if task.validation_errors.is_empty() {
        return;
    }
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        "Validation errors",
        theme::warning(),
    )));
    for e in &task.validation_errors {
        lines.push(Line::from(vec![
            Span::styled("  • ", theme::warning()),
            Span::raw(e.clone()),
        ]));
    }
}

fn push_recent_bulletins(
    lines: &mut Vec<Line<'static>>,
    task: &ReportingTaskRow,
    ring: &BulletinRing,
) {
    let matches: Vec<&BulletinSnapshot> = ring
        .buf
        .iter()
        .rev() // newest first
        .filter(|b| b.source_id == task.id)
        .take(10)
        .collect();
    lines.push(Line::raw(""));
    lines.push(Line::from(Span::styled(
        format!("Recent bulletins ({})", matches.len()),
        Style::default().add_modifier(Modifier::BOLD),
    )));
    if matches.is_empty() {
        lines.push(Line::from(Span::styled(
            "  no bulletins in ring buffer",
            theme::muted(),
        )));
        return;
    }
    for b in matches {
        lines.push(Line::from(vec![
            Span::raw(format!("  {} ", b.timestamp_human)),
            Span::styled(b.level.clone(), level_style(&b.level)),
            Span::raw(format!("  {}", truncate_chars(&b.message, 80))),
        ]));
    }
}

fn kv_line(k: &str, v: &str) -> Line<'static> {
    Line::from(vec![
        Span::raw(format!("  {k:<20}")),
        Span::raw(v.to_string()),
    ])
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
}
