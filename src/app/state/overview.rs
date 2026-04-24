//! Overview tab key handler.

use super::{AppState, Modal, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Overview tab.
pub(crate) struct OverviewHandler;

impl ViewKeyHandler for OverviewHandler {
    fn handle_verb(_state: &mut AppState, _verb: crate::input::ViewVerb) -> Option<UpdateResult> {
        None
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

#[cfg(test)]
mod tests {
    use super::super::ViewKeyHandler;
    use super::super::tests::{fresh_state, tiny_config};
    use super::OverviewHandler;
    use crate::app::state::{Modal, ViewId};
    use crate::client::health::NodeHealthRow;
    use crate::input::FocusAction;
    use crate::view::overview::state::OverviewFocus;

    fn set_nodes(s: &mut crate::app::state::AppState, count: usize) {
        s.overview.nodes.nodes = (0..count)
            .map(|i| NodeHealthRow {
                node_address: format!("node{}:8080", i),
                heap_used_bytes: 512 * 1024 * 1024,
                heap_max_bytes: 1024 * 1024 * 1024,
                heap_percent: 50,
                heap_severity: crate::client::health::Severity::Green,
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
        use crate::input::{BulletinsVerb, ViewVerb};
        let mut s = fresh_state();
        s.current_tab = ViewId::Overview;
        assert!(
            OverviewHandler::handle_verb(&mut s, ViewVerb::Bulletins(BulletinsVerb::Refresh))
                .is_none()
        );
        assert!(OverviewHandler::handle_focus(&mut s, FocusAction::Descend).is_some());
    }

    #[test]
    fn overview_controller_status_redraw_mirrors_snapshot() {
        use crate::client::ControllerStatusSnapshot;
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::{Duration, Instant};

        // After Task 8 Overview is store-only. Seeding the cluster
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
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(10),
            },
        };
        crate::view::overview::state::redraw_controller_status(&mut s);
        assert_eq!(s.overview.controller.as_ref().unwrap().running, 7);
    }
}
