//! Overview tab key handler.

use super::{AppState, Modal, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Overview tab.
pub(crate) struct OverviewHandler;

impl ViewKeyHandler for OverviewHandler {
    fn handle_verb(state: &mut AppState, verb: crate::input::ViewVerb) -> Option<UpdateResult> {
        use crate::input::{OverviewVerb, ViewVerb};
        match verb {
            ViewVerb::Overview(OverviewVerb::OpenReportingTasksModal) => {
                use crate::view::overview::reporting_tasks_modal::ReportingTasksModalState;
                let modal = if let Some(snap) = state.cluster.snapshot.reporting_tasks.latest() {
                    ReportingTasksModalState::open(snap)
                } else {
                    ReportingTasksModalState::default()
                };
                state.overview.reporting_tasks_modal = Some(modal);
                Some(UpdateResult {
                    redraw: true,
                    ..Default::default()
                })
            }
            ViewVerb::OverviewReportingTasksModal(v) => handle_reporting_tasks_modal_verb(state, v),
            _ => None,
        }
    }

    fn handle_focus(
        state: &mut AppState,
        action: crate::input::FocusAction,
    ) -> Option<UpdateResult> {
        use crate::app::navigation::ListNavigation;
        use crate::input::FocusAction as FA;
        use crate::view::overview::state::OverviewFocus;

        let done = || {
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
                tracer_diff_followup: None,
            })
        };

        // Cycle order for NextPane/PrevPane: None → Nodes → Noisy → Queues → None
        match action {
            FA::NextPane => {
                state.overview.focus = match state.overview.focus {
                    OverviewFocus::None => OverviewFocus::Nodes,
                    OverviewFocus::Nodes => OverviewFocus::Noisy,
                    OverviewFocus::Noisy => OverviewFocus::Queues,
                    OverviewFocus::Queues => OverviewFocus::None,
                };
                return done();
            }
            FA::PrevPane => {
                state.overview.focus = match state.overview.focus {
                    OverviewFocus::None => OverviewFocus::Queues,
                    OverviewFocus::Nodes => OverviewFocus::None,
                    OverviewFocus::Noisy => OverviewFocus::Nodes,
                    OverviewFocus::Queues => OverviewFocus::Noisy,
                };
                return done();
            }
            FA::Left | FA::Right => return None,
            _ => {}
        }

        match state.overview.focus {
            OverviewFocus::None => match action {
                FA::Descend => {
                    state.overview.focus = OverviewFocus::Nodes;
                    done()
                }
                _ => None,
            },
            OverviewFocus::Nodes => match action {
                FA::Ascend => {
                    state.overview.focus = OverviewFocus::None;
                    done()
                }
                FA::Up => {
                    state.overview.nodes.move_up();
                    done()
                }
                FA::Down => {
                    state.overview.nodes.move_down();
                    done()
                }
                FA::Descend => {
                    if let Some(row) = state
                        .overview
                        .nodes
                        .nodes
                        .get(state.overview.nodes.selected)
                    {
                        state.modal = Some(Modal::NodeDetail(Box::new(row.clone())));
                    }
                    done()
                }
                _ => None,
            },
            OverviewFocus::Noisy => match action {
                FA::Ascend => {
                    state.overview.focus = OverviewFocus::None;
                    done()
                }
                FA::Up => {
                    state.overview.noisy_nav().move_up();
                    done()
                }
                FA::Down => {
                    state.overview.noisy_nav().move_down();
                    done()
                }
                _ => None,
            },
            OverviewFocus::Queues => match action {
                FA::Ascend => {
                    state.overview.focus = OverviewFocus::None;
                    done()
                }
                FA::Up => {
                    state.overview.queues_nav().move_up();
                    done()
                }
                FA::Down => {
                    state.overview.queues_nav().move_down();
                    done()
                }
                _ => None,
            },
        }
    }
}

/// Row count for the detail sub-section currently focused by `idx`.
/// Returns `None` if the snapshot or selection isn't resolvable, or if
/// `idx` is out of range for the current section list.
fn detail_row_count_for_focus(
    cluster_snapshot: &crate::cluster::snapshot::ClusterSnapshot,
    modal: &crate::view::overview::reporting_tasks_modal::ReportingTasksModalState,
    idx: usize,
) -> Option<usize> {
    use crate::view::overview::reporting_tasks_modal::{DetailSection, section_list};
    let snap = cluster_snapshot.reporting_tasks.latest()?;
    let task = modal.selected_row(snap)?;
    let sections = section_list(task);
    let &section = sections.get(idx)?;
    Some(match section {
        DetailSection::Properties => task.properties.len(),
        DetailSection::ValidationErrors => task.validation_errors.len(),
        DetailSection::RecentBulletins => cluster_snapshot
            .bulletins
            .buf
            .iter()
            .filter(|b| b.source_id == task.id)
            .take(10)
            .count(),
    })
}

/// Handler for Overview reporting-tasks modal verbs.
///
/// Covers: Esc cascade (search → close), Copy, row navigation, Enter
/// cross-links (param-ref → parameter-context modal; bulletin → Bulletins
/// tab), search open/next/prev, and force-refresh.
fn handle_reporting_tasks_modal_verb(
    state: &mut AppState,
    verb: crate::input::OverviewReportingTasksVerb,
) -> Option<UpdateResult> {
    use crate::input::{CommonVerb, OverviewReportingTasksVerb as V};
    use crate::view::overview::reporting_tasks_modal::ModalFocus;

    let redraw = || {
        Some(UpdateResult {
            redraw: true,
            ..Default::default()
        })
    };

    match verb {
        // ---- Esc cascade: leave Detail focus → search → close modal ----
        V::Common(CommonVerb::Close) => {
            let snap = state.cluster.snapshot.reporting_tasks.latest().cloned();
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            // Step 1: if focused inside Detail, return to List.
            if matches!(modal.focus, ModalFocus::Detail { .. }) {
                modal.focus = ModalFocus::List;
                return redraw();
            }
            // Step 2: clear active search query if present.
            if !modal.search.query.is_empty() {
                modal.search.query.clear();
                if let Some(snap) = snap {
                    modal.refilter(&snap);
                }
                return redraw();
            }
            // Step 3: close the modal.
            state.overview.reporting_tasks_modal = None;
            redraw()
        }

        // ---- Enter (FocusDetail / cross-link from detail) ----
        V::FocusDetail => {
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            if matches!(modal.focus, ModalFocus::List) {
                modal.focus = ModalFocus::Detail {
                    idx: 0,
                    rows: [0; 3],
                };
                return redraw();
            }
            // Detail focus — Enter cross-link wired in Task 7.
            redraw()
        }

        // ---- Row navigation (list pane or detail section) ----
        V::RowUp => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    if modal.selected_ordinal > 0 {
                        modal.selected_ordinal -= 1;
                    }
                    if let Some(&raw_idx) = modal.filtered_indices.get(modal.selected_ordinal) {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    rows[idx] = rows[idx].saturating_sub(1);
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }
        V::RowDown => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    let max = modal.filtered_indices.len().saturating_sub(1);
                    if modal.selected_ordinal < max {
                        modal.selected_ordinal += 1;
                    }
                    if let Some(&raw_idx) = modal.filtered_indices.get(modal.selected_ordinal) {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    let count =
                        detail_row_count_for_focus(cluster_snapshot, modal, idx).unwrap_or(0);
                    let max = count.saturating_sub(1);
                    if rows[idx] < max {
                        rows[idx] += 1;
                    }
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }
        V::PageUp => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    modal.selected_ordinal = modal.selected_ordinal.saturating_sub(10);
                    if let Some(&raw_idx) = modal.filtered_indices.get(modal.selected_ordinal) {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    rows[idx] = rows[idx].saturating_sub(10);
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }
        V::PageDown => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    let max = modal.filtered_indices.len().saturating_sub(1);
                    modal.selected_ordinal = (modal.selected_ordinal + 10).min(max);
                    if let Some(&raw_idx) = modal.filtered_indices.get(modal.selected_ordinal) {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    let count =
                        detail_row_count_for_focus(cluster_snapshot, modal, idx).unwrap_or(0);
                    let max = count.saturating_sub(1);
                    rows[idx] = (rows[idx] + 10).min(max);
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }
        V::JumpTop => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    modal.selected_ordinal = 0;
                    if let Some(&raw_idx) = modal.filtered_indices.first() {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    rows[idx] = 0;
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }
        V::JumpBottom => {
            let cluster_snapshot = &state.cluster.snapshot;
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => {
                    modal.selected_ordinal = modal.filtered_indices.len().saturating_sub(1);
                    if let Some(&raw_idx) = modal.filtered_indices.last() {
                        modal.selected_id = cluster_snapshot
                            .reporting_tasks
                            .latest()
                            .and_then(|s| s.tasks.get(raw_idx))
                            .map(|t| t.id.clone());
                    }
                }
                ModalFocus::Detail { idx, mut rows } => {
                    let count =
                        detail_row_count_for_focus(cluster_snapshot, modal, idx).unwrap_or(0);
                    rows[idx] = count.saturating_sub(1);
                    modal.focus = ModalFocus::Detail { idx, rows };
                }
            }
            redraw()
        }

        // ---- Copy ----
        V::Common(CommonVerb::Copy) => {
            let snap = state.cluster.snapshot.reporting_tasks.latest().cloned();
            let modal = state.overview.reporting_tasks_modal.as_ref()?;
            let text = match modal.focus {
                ModalFocus::List => snap.as_ref().and_then(|s| modal.selected_row(s)).map(|t| {
                    use crate::view::overview::reporting_tasks_modal::short_type;
                    format!(
                        "{}\t{}\t{}\t{}\t{}",
                        t.id,
                        t.name,
                        short_type(&t.task_type),
                        t.scheduling_period,
                        t.active_thread_count,
                    )
                }),
                ModalFocus::Detail { .. } => None, // wired in Task 8
            };
            if let Some(text) = text {
                super::clipboard_copy(state, &text);
            }
            redraw()
        }

        // ---- Section cycling (Tab / Shift+Tab) ----
        V::NextPane => {
            let snap = state.cluster.snapshot.reporting_tasks.latest().cloned();
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            let ModalFocus::Detail { idx, rows } = modal.focus else {
                return redraw();
            };
            let Some(snap) = snap.as_ref() else {
                return redraw();
            };
            let Some(task) = modal.selected_row(snap) else {
                return redraw();
            };
            let sections = crate::view::overview::reporting_tasks_modal::section_list(task);
            let new_idx = idx + 1;
            if new_idx >= sections.len() {
                modal.focus = ModalFocus::List;
            } else {
                modal.focus = ModalFocus::Detail { idx: new_idx, rows };
            }
            redraw()
        }
        V::PrevPane => {
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            match modal.focus {
                ModalFocus::List => redraw(),
                ModalFocus::Detail { idx: 0, .. } => {
                    modal.focus = ModalFocus::List;
                    redraw()
                }
                ModalFocus::Detail { idx, rows } => {
                    modal.focus = ModalFocus::Detail { idx: idx - 1, rows };
                    redraw()
                }
            }
        }

        // ---- Search ----
        V::Common(CommonVerb::OpenSearch) => {
            let modal = state.overview.reporting_tasks_modal.as_mut()?;
            modal.search.input_active = true;
            redraw()
        }
        V::Common(CommonVerb::SearchNext) => {
            // No-op — search is a simple text filter, not a highlight cursor.
            redraw()
        }
        V::Common(CommonVerb::SearchPrev) => {
            // No-op — search is a simple text filter, not a highlight cursor.
            redraw()
        }

        // ---- Refresh ----
        V::Common(CommonVerb::Refresh) => {
            state
                .cluster
                .force(crate::cluster::ClusterEndpoint::ReportingTasks);
            redraw()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::ViewKeyHandler;
    use super::super::tests::{fresh_state, tiny_config};
    use super::OverviewHandler;
    use crate::app::state::{Modal, ViewId};
    use crate::client::overview::NodeHealthRow;
    use crate::input::FocusAction;
    use crate::view::overview::state::OverviewFocus;

    fn set_nodes(s: &mut crate::app::state::AppState, count: usize) {
        s.overview.nodes.nodes = (0..count)
            .map(|i| NodeHealthRow {
                node_address: format!("node{}:8080", i),
                heap_used_bytes: crate::bytes::FIXTURE_HEAP_USED,
                heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
                heap_percent: 50,
                heap_severity: crate::client::overview::Severity::Green,
                gc_collection_count: 10,
                gc_delta: None,
                gc_millis: 50,
                load_average: Some(1.5),
                available_processors: Some(4),
                uptime: "1h".into(),
                total_threads: 40,
                gc: vec![],
                content_repos: vec![],
                flowfile_repo: None,
                provenance_repos: vec![],
                cluster: None,
                tls_cert: None,
            })
            .collect();
    }

    #[test]
    fn descend_from_none_enters_nodes() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        let r = OverviewHandler::handle_focus(&mut s, FocusAction::Descend);
        assert!(r.unwrap().redraw);
        assert_eq!(s.overview.focus, OverviewFocus::Nodes);
    }

    #[test]
    fn other_action_from_none_falls_through() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        assert!(OverviewHandler::handle_focus(&mut s, FocusAction::Up).is_none());
        assert!(OverviewHandler::handle_focus(&mut s, FocusAction::Down).is_none());
        assert!(OverviewHandler::handle_focus(&mut s, FocusAction::Left).is_none());
    }

    #[test]
    fn ascend_returns_to_none() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        OverviewHandler::handle_focus(&mut s, FocusAction::Ascend);
        assert_eq!(s.overview.focus, OverviewFocus::None);
    }

    #[test]
    fn next_pane_from_none_enters_nodes() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        let r = OverviewHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.unwrap().redraw);
        assert_eq!(s.overview.focus, OverviewFocus::Nodes);
    }

    #[test]
    fn prev_pane_from_none_wraps_to_queues() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        let r = OverviewHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert!(r.unwrap().redraw);
        assert_eq!(s.overview.focus, OverviewFocus::Queues);
    }

    #[test]
    fn next_pane_cycles_nodes_noisy_queues_none() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        OverviewHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert_eq!(s.overview.focus, OverviewFocus::Noisy);
        OverviewHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert_eq!(s.overview.focus, OverviewFocus::Queues);
        OverviewHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert_eq!(s.overview.focus, OverviewFocus::None);
    }

    #[test]
    fn next_pane_from_queues_wraps_to_none() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Queues;
        OverviewHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert_eq!(s.overview.focus, OverviewFocus::None);
    }

    #[test]
    fn prev_pane_cycles_in_reverse() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        OverviewHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert_eq!(s.overview.focus, OverviewFocus::None);
        OverviewHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert_eq!(s.overview.focus, OverviewFocus::Queues);
        OverviewHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert_eq!(s.overview.focus, OverviewFocus::Noisy);
    }

    #[test]
    fn left_right_are_unmapped_in_all_overview_focus_states() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        for focus in [
            OverviewFocus::None,
            OverviewFocus::Nodes,
            OverviewFocus::Noisy,
            OverviewFocus::Queues,
        ] {
            s.overview.focus = focus;
            assert!(
                OverviewHandler::handle_focus(&mut s, FocusAction::Left).is_none(),
                "Left should be unmapped in {focus:?}"
            );
            assert!(
                OverviewHandler::handle_focus(&mut s, FocusAction::Right).is_none(),
                "Right should be unmapped in {focus:?}"
            );
        }
    }

    #[test]
    fn down_in_nodes_increments_cursor() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        set_nodes(&mut s, 3);
        OverviewHandler::handle_focus(&mut s, FocusAction::Down);
        assert_eq!(s.overview.nodes.selected, 1);
    }

    #[test]
    fn down_in_nodes_clamped_at_last_row() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        set_nodes(&mut s, 2);
        s.overview.nodes.selected = 1;
        OverviewHandler::handle_focus(&mut s, FocusAction::Down);
        assert_eq!(s.overview.nodes.selected, 1, "should not go past len-1");
    }

    #[test]
    fn up_in_nodes_saturates_at_zero() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        set_nodes(&mut s, 3);
        s.overview.nodes.selected = 0;
        OverviewHandler::handle_focus(&mut s, FocusAction::Up);
        assert_eq!(s.overview.nodes.selected, 0);
    }

    #[test]
    fn descend_in_nodes_opens_node_detail_modal() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        set_nodes(&mut s, 2);
        s.overview.nodes.selected = 1;
        OverviewHandler::handle_focus(&mut s, FocusAction::Descend);
        assert!(
            matches!(&s.modal, Some(Modal::NodeDetail(row)) if row.node_address == "node1:8080"),
            "modal should be NodeDetail for node1"
        );
    }

    #[test]
    fn descend_in_nodes_noop_when_empty() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Nodes;
        // No nodes populated.
        let r = OverviewHandler::handle_focus(&mut s, FocusAction::Descend);
        assert!(r.is_some()); // returns a result (redraw=true)
        assert!(s.modal.is_none());
    }

    #[test]
    fn down_in_noisy_increments_cursor() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Noisy;
        s.overview.noisy = vec![
            crate::view::overview::state::NoisyComponent {
                source_id: "a".into(),
                group_id: "g".into(),
                source_name: "A".into(),
                count: 1,
                max_severity: crate::view::overview::state::Severity::Info,
            },
            crate::view::overview::state::NoisyComponent {
                source_id: "b".into(),
                group_id: "g".into(),
                source_name: "B".into(),
                count: 1,
                max_severity: crate::view::overview::state::Severity::Info,
            },
        ];
        OverviewHandler::handle_focus(&mut s, FocusAction::Down);
        assert_eq!(s.overview.noisy_selected, 1);
    }

    #[test]
    fn down_in_queues_increments_cursor() {
        use crate::view::overview::state::UnhealthyQueue;
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        s.overview.focus = OverviewFocus::Queues;
        s.overview.unhealthy = vec![
            UnhealthyQueue {
                id: "c0".into(),
                group_id: "g".into(),
                name: "q0".into(),
                source_name: "A".into(),
                destination_name: "B".into(),
                fill_percent: 80,
                flow_files_queued: 100,
                bytes_queued: 0,
                queued_display: "100".into(),
            },
            UnhealthyQueue {
                id: "c1".into(),
                group_id: "g".into(),
                name: "q1".into(),
                source_name: "C".into(),
                destination_name: "D".into(),
                fill_percent: 70,
                flow_files_queued: 50,
                bytes_queued: 0,
                queued_display: "50".into(),
            },
        ];
        OverviewHandler::handle_focus(&mut s, FocusAction::Down);
        assert_eq!(s.overview.queues_selected, 1);
    }

    // Keep the existing noop / data-event tests:
    #[test]
    fn overview_handle_verb_is_noop() {
        use crate::input::{BulletinsVerb, CommonVerb, ViewVerb};
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        assert!(
            OverviewHandler::handle_verb(
                &mut s,
                ViewVerb::Bulletins(BulletinsVerb::Common(CommonVerb::Refresh))
            )
            .is_none()
        );
        assert!(OverviewHandler::handle_focus(&mut s, FocusAction::Descend).is_some());
    }

    #[test]
    fn overview_controller_status_redraw_mirrors_snapshot() {
        use crate::client::ControllerStatusSnapshot;
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::{Duration, Instant};

        // Overview is store-only. Seeding the cluster
        // snapshot directly and invoking the reducer mirrors what the
        // main loop's `ClusterChanged(ControllerStatus)` arm does.
        let mut s = fresh_state();
        let _ = tiny_config();
        s.cluster.snapshot.controller_status = EndpointState::Ready {
            data: ControllerStatusSnapshot {
                running: 7,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                stale: 0,
                locally_modified: 0,
                sync_failure: 0,
                up_to_date: 0,
            },
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: crate::test_support::default_fetch_duration(),
                next_interval: Duration::from_secs(10),
            },
        };
        crate::view::overview::state::redraw_controller_status(&mut s);
        assert_eq!(s.overview.controller.as_ref().unwrap().running, 7);
    }

    // ---- Reporting-tasks modal handler tests ----

    use crate::app::state::PendingIntent;
    use crate::client::reporting_tasks::{
        ReportingTaskRow, ReportingTaskState, ReportingTasksSnapshot, ValidationStatus,
    };
    use crate::cluster::snapshot::{EndpointState, FetchMeta};
    use crate::input::{CommonVerb, OverviewReportingTasksVerb as MV, ViewVerb};
    use crate::view::overview::reporting_tasks_modal::{ModalFocus, ReportingTasksModalState};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    fn bare_task(id: &str) -> ReportingTaskRow {
        ReportingTaskRow {
            id: id.into(),
            name: format!("Task-{id}"),
            task_type: "org.x.Y".into(),
            state: ReportingTaskState::Running,
            scheduling_strategy: "TIMER_DRIVEN".into(),
            scheduling_period: "30s".into(),
            active_thread_count: 0,
            validation_status: ValidationStatus::Valid,
            validation_errors: vec![],
            comments: None,
            properties: BTreeMap::new(),
            descriptors: BTreeMap::new(),
        }
    }

    fn snap_with_tasks(tasks: Vec<ReportingTaskRow>) -> ReportingTasksSnapshot {
        ReportingTasksSnapshot {
            tasks,
            fetched_at: Instant::now(),
        }
    }

    fn seed_rt_snapshot(s: &mut crate::app::state::AppState, snap: ReportingTasksSnapshot) {
        s.cluster.snapshot.reporting_tasks = EndpointState::Ready {
            data: snap,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: crate::test_support::default_fetch_duration(),
                next_interval: Duration::from_secs(10),
            },
        };
    }

    fn open_modal_with_tasks(s: &mut crate::app::state::AppState, tasks: Vec<ReportingTaskRow>) {
        let snap = snap_with_tasks(tasks);
        let modal = ReportingTasksModalState::open(&snap);
        seed_rt_snapshot(s, snap);
        s.overview.reporting_tasks_modal = Some(modal);
    }

    fn dispatch_modal_verb(
        s: &mut crate::app::state::AppState,
        verb: MV,
    ) -> Option<crate::app::state::UpdateResult> {
        OverviewHandler::handle_verb(s, ViewVerb::OverviewReportingTasksModal(verb))
    }

    #[test]
    fn enter_on_list_shifts_focus_to_detail() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1"), bare_task("t2")]);

        let r = dispatch_modal_verb(&mut s, MV::FocusDetail);
        assert!(r.unwrap().redraw);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert!(matches!(modal.focus, ModalFocus::Detail { .. }));
    }

    #[test]
    fn esc_from_detail_returns_to_list() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };

        dispatch_modal_verb(&mut s, MV::Common(CommonVerb::Close));
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(modal.focus, ModalFocus::List);
        // Modal still open after first Esc.
        assert!(s.overview.reporting_tasks_modal.is_some());
    }

    #[test]
    fn esc_with_search_query_clears_search_keeps_modal_open() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        s.overview
            .reporting_tasks_modal
            .as_mut()
            .unwrap()
            .search
            .query = "foo".into();

        dispatch_modal_verb(&mut s, MV::Common(CommonVerb::Close));
        let modal = s.overview.reporting_tasks_modal.as_ref();
        assert!(
            modal.is_some(),
            "modal should still be open after search clear"
        );
        assert!(
            modal.unwrap().search.query.is_empty(),
            "search query should be cleared"
        );
    }

    #[test]
    fn esc_without_search_closes_modal() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);

        dispatch_modal_verb(&mut s, MV::Common(CommonVerb::Close));
        assert!(
            s.overview.reporting_tasks_modal.is_none(),
            "modal should be closed"
        );
    }

    #[test]
    #[ignore = "cross-link from Detail focus is wired in Task 7 of the detail-panels plan"]
    fn enter_on_bulletin_emits_bulletins_cross_link() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        let task = bare_task("task-id-1");
        open_modal_with_tasks(&mut s, vec![task.clone()]);

        // Seed a bulletin for this task
        use crate::client::BulletinSnapshot;
        s.cluster
            .snapshot
            .bulletins
            .buf
            .push_back(BulletinSnapshot {
                id: 1,
                level: "WARN".into(),
                message: "test bulletin".into(),
                source_id: "task-id-1".into(),
                source_name: "Task-task-id-1".into(),
                source_type: "REPORTING_TASK".into(),
                group_id: "".into(),
                timestamp_iso: "2026-01-01T00:00:00Z".into(),
                timestamp_human: "00:00:00 UTC".into(),
            });

        // Shift focus to detail pane
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };

        // Press Enter
        let r = dispatch_modal_verb(&mut s, MV::FocusDetail);
        let r = r.expect("should return Some");
        assert!(r.redraw);
        // Modal should be closed
        assert!(s.overview.reporting_tasks_modal.is_none());
        // Intent should be OpenBulletins
        match r.intent {
            Some(PendingIntent::Goto(crate::intent::CrossLink::OpenBulletins { source_id })) => {
                assert_eq!(source_id, "task-id-1");
            }
            other => panic!("expected Goto(OpenBulletins), got {other:?}"),
        }
    }

    #[test]
    fn row_down_increments_selection() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(
            &mut s,
            vec![bare_task("t1"), bare_task("t2"), bare_task("t3")],
        );

        dispatch_modal_verb(&mut s, MV::RowDown);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(modal.selected_ordinal, 1);
        assert_eq!(modal.selected_id.as_deref(), Some("t2"));
    }

    #[test]
    fn row_up_decrements_selection_clamped() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1"), bare_task("t2")]);
        // Already at 0
        dispatch_modal_verb(&mut s, MV::RowUp);
        assert_eq!(
            s.overview
                .reporting_tasks_modal
                .as_ref()
                .unwrap()
                .selected_ordinal,
            0
        );
    }

    fn task_with_validation_error(id: &str) -> ReportingTaskRow {
        let mut t = bare_task(id);
        t.validation_status = ValidationStatus::Invalid;
        t.validation_errors = vec!["err".into()];
        t
    }

    #[test]
    fn tab_from_list_enters_detail_at_first_section() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![task_with_validation_error("t1")]);

        dispatch_modal_verb(&mut s, MV::FocusDetail);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert!(matches!(modal.focus, ModalFocus::Detail { idx: 0, .. }));

        dispatch_modal_verb(&mut s, MV::NextPane);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert!(matches!(modal.focus, ModalFocus::Detail { idx: 1, .. }));
    }

    #[test]
    fn tab_from_last_section_wraps_to_list() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        // No validation errors → sections = [Properties, RecentBulletins], len 2.
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 1,
            rows: [0; 3],
        };

        dispatch_modal_verb(&mut s, MV::NextPane);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(modal.focus, ModalFocus::List);
    }

    #[test]
    fn shift_tab_from_first_section_wraps_to_list() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };

        dispatch_modal_verb(&mut s, MV::PrevPane);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(modal.focus, ModalFocus::List);
    }

    #[test]
    fn shift_tab_from_list_is_noop() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        dispatch_modal_verb(&mut s, MV::PrevPane);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(modal.focus, ModalFocus::List);
    }

    fn task_with_three_properties(id: &str) -> ReportingTaskRow {
        let mut t = bare_task(id);
        let mut props = BTreeMap::new();
        props.insert("p1".into(), Some("v1".into()));
        props.insert("p2".into(), Some("v2".into()));
        props.insert("p3".into(), Some("v3".into()));
        t.properties = props;
        t
    }

    #[test]
    fn row_down_in_detail_moves_section_row_cursor() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![task_with_three_properties("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };

        dispatch_modal_verb(&mut s, MV::RowDown);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(
            modal.focus,
            ModalFocus::Detail {
                idx: 0,
                rows: [1, 0, 0]
            }
        );
    }

    #[test]
    fn row_up_in_detail_clamps_at_zero() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![task_with_three_properties("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [0; 3],
        };

        dispatch_modal_verb(&mut s, MV::RowUp);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(
            modal.focus,
            ModalFocus::Detail {
                idx: 0,
                rows: [0; 3]
            }
        );
    }

    #[test]
    fn row_down_in_detail_clamps_at_section_row_count() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![task_with_three_properties("t1")]);
        let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
        // Last property index = 2.
        modal.focus = ModalFocus::Detail {
            idx: 0,
            rows: [2, 0, 0],
        };

        dispatch_modal_verb(&mut s, MV::RowDown);
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(
            modal.focus,
            ModalFocus::Detail {
                idx: 0,
                rows: [2, 0, 0]
            }
        );
    }

    #[test]
    fn jump_top_and_bottom_work() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(
            &mut s,
            vec![bare_task("t1"), bare_task("t2"), bare_task("t3")],
        );
        dispatch_modal_verb(&mut s, MV::JumpBottom);
        assert_eq!(
            s.overview
                .reporting_tasks_modal
                .as_ref()
                .unwrap()
                .selected_ordinal,
            2
        );
        dispatch_modal_verb(&mut s, MV::JumpTop);
        assert_eq!(
            s.overview
                .reporting_tasks_modal
                .as_ref()
                .unwrap()
                .selected_ordinal,
            0
        );
    }

    #[test]
    fn open_search_activates_search_input() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        open_modal_with_tasks(&mut s, vec![bare_task("t1")]);
        dispatch_modal_verb(&mut s, MV::Common(CommonVerb::OpenSearch));
        assert!(
            s.overview
                .reporting_tasks_modal
                .as_ref()
                .unwrap()
                .search
                .input_active,
            "search input should be active"
        );
    }

    // ---- OverviewVerb ('t' chord) tests ----

    fn dispatch_overview_verb(
        s: &mut crate::app::state::AppState,
        verb: crate::input::OverviewVerb,
    ) -> Option<crate::app::state::UpdateResult> {
        use crate::input::ViewVerb;
        OverviewHandler::handle_verb(s, ViewVerb::Overview(verb))
    }

    #[test]
    fn t_opens_modal_with_data() {
        use crate::input::OverviewVerb;
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        seed_rt_snapshot(
            &mut s,
            snap_with_tasks(vec![bare_task("r1"), bare_task("r2")]),
        );
        let r = dispatch_overview_verb(&mut s, OverviewVerb::OpenReportingTasksModal);
        assert!(r.unwrap().redraw, "should request a redraw");
        let modal = s
            .overview
            .reporting_tasks_modal
            .as_ref()
            .expect("modal should be open");
        // First task selected.
        assert_eq!(modal.selected_id.as_deref(), Some("r1"));
    }

    #[test]
    fn t_opens_empty_modal_before_first_fetch() {
        use crate::input::OverviewVerb;
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        // No snapshot seeded → EndpointState::Loading by default.
        let r = dispatch_overview_verb(&mut s, OverviewVerb::OpenReportingTasksModal);
        assert!(r.unwrap().redraw, "should request a redraw");
        // Modal opens in default (empty) state; selected_id is None.
        let modal = s
            .overview
            .reporting_tasks_modal
            .as_ref()
            .expect("modal should be open");
        assert!(
            modal.selected_id.is_none(),
            "no selection before first fetch"
        );
    }

    #[test]
    fn cluster_changed_reporting_tasks_reconciles_open_modal_selection() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        // Open modal with tasks [a, b, c].
        open_modal_with_tasks(&mut s, vec![bare_task("a"), bare_task("b"), bare_task("c")]);
        // Manually select "c" at ordinal 2.
        {
            let modal = s.overview.reporting_tasks_modal.as_mut().unwrap();
            modal.selected_id = Some("c".into());
            modal.selected_ordinal = 2;
        }
        // Simulate an arrival of a snapshot where "c" is gone (shrunk to [a, b]).
        let new_snap = snap_with_tasks(vec![bare_task("a"), bare_task("b")]);
        seed_rt_snapshot(&mut s, new_snap);
        crate::view::overview::state::redraw_components(&mut s);
        // reconcile_selection should fall back to ordinal 2 clamped → "b".
        let modal = s.overview.reporting_tasks_modal.as_ref().unwrap();
        assert_eq!(
            modal.selected_id.as_deref(),
            Some("b"),
            "selection should clamp to last row when selected id disappears"
        );
    }
}
