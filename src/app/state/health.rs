//! Health tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    AppState, Banner, BannerSeverity, PendingIntent, StatusLine, UpdateResult, ViewKeyHandler,
};
use crate::app::navigation::ListNavigation;

/// Zero-sized dispatch struct for the Health tab.
pub(crate) struct HealthHandler;

/// Rebuild the per-node detail for the currently selected repository row.
fn rebuild_repo_per_node(repos: &mut crate::client::health::RepositoryState) {
    if let Some(row) = repos.rows.get(repos.selected) {
        repos.per_node =
            crate::client::health::build_per_node_for_row(row, &repos.node_diagnostics);
    } else {
        repos.per_node.clear();
    }
}

impl ViewKeyHandler for HealthHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        use crate::view::health::state::HealthCategory;

        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }

        match key.code {
            // Category switching by number (1–4)
            KeyCode::Char(c @ '1'..='4') => {
                if let Some(cat) = HealthCategory::from_index(c as u8 - b'0') {
                    state.health.selected_category = cat;
                }
            }
            // Detail table navigation (j / down)
            KeyCode::Down | KeyCode::Char('j') => match state.health.selected_category {
                HealthCategory::Queues => {
                    state.health.queues.move_down();
                }
                HealthCategory::Nodes => {
                    state.health.nodes.move_down();
                }
                HealthCategory::Processors => {
                    state.health.processors.move_down();
                }
                HealthCategory::Repositories => {
                    state.health.repositories.move_down();
                    rebuild_repo_per_node(&mut state.health.repositories);
                }
            },
            // Detail table navigation (k / up)
            KeyCode::Up | KeyCode::Char('k') => match state.health.selected_category {
                HealthCategory::Queues => {
                    state.health.queues.move_up();
                }
                HealthCategory::Nodes => {
                    state.health.nodes.move_up();
                }
                HealthCategory::Processors => {
                    state.health.processors.move_up();
                }
                HealthCategory::Repositories => {
                    state.health.repositories.move_up();
                    rebuild_repo_per_node(&mut state.health.repositories);
                }
            },
            // Enter → jump to Browser for Queues and Processors
            KeyCode::Enter => {
                let cross_link = match state.health.selected_category {
                    HealthCategory::Queues => state
                        .health
                        .queues
                        .rows
                        .get(state.health.queues.selected)
                        .map(|r| crate::intent::CrossLink::OpenInBrowser {
                            component_id: r.connection_id.clone(),
                            group_id: r.group_id.clone(),
                        }),
                    HealthCategory::Processors => state
                        .health
                        .processors
                        .rows
                        .get(state.health.processors.selected)
                        .map(|r| crate::intent::CrossLink::OpenInBrowser {
                            component_id: r.processor_id.clone(),
                            group_id: r.group_id.clone(),
                        }),
                    HealthCategory::Repositories | HealthCategory::Nodes => {
                        let category_name = match state.health.selected_category {
                            HealthCategory::Repositories => "repository",
                            HealthCategory::Nodes => "node",
                            _ => unreachable!(),
                        };
                        state.status = StatusLine {
                            banner: Some(Banner {
                                severity: BannerSeverity::Info,
                                message: format!(
                                    "Cross-link not available for {category_name} rows"
                                ),
                                detail: None,
                            }),
                        };
                        return Some(UpdateResult {
                            redraw: true,
                            ..UpdateResult::default()
                        });
                    }
                };
                if let Some(link) = cross_link {
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                        tracer_followup: None,
                    });
                }
            }
            // r → refresh via RefreshView intent
            KeyCode::Char('r') => {
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(crate::intent::Intent::RefreshView(
                        super::ViewId::Health,
                    ))),
                    tracer_followup: None,
                });
            }
            _ => return None,
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::{PendingIntent, ViewId};
    use crate::client::health::{ConnectionStatusRow, FullPgStatusSnapshot, ProcessorStatusRow};
    use crate::event::{AppEvent, HealthPayload, ViewPayload};
    use crate::intent::{CrossLink, Intent};
    use crate::view::health::state::HealthCategory;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::Instant;

    fn seeded_health_state() -> (crate::app::state::AppState, crate::config::Config) {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Health;

        // Feed a PgStatus payload so queues and processors are non-empty.
        let snap = FullPgStatusSnapshot {
            connections: vec![ConnectionStatusRow {
                id: "conn-1".into(),
                group_id: "root".into(),
                name: "conn".into(),
                source_name: "src".into(),
                destination_name: "dst".into(),
                percent_use_count: 75,
                percent_use_bytes: 10,
                flow_files_queued: 100,
                bytes_queued: 1024,
                queued_display: "100".into(),
                bytes_in: 2048,
                bytes_out: 1024,
                predicted_millis_until_backpressure: None,
            }],
            processors: vec![ProcessorStatusRow {
                id: "proc-1".into(),
                group_id: "root".into(),
                name: "Gen".into(),
                group_path: "/root".into(),
                active_thread_count: 2,
                run_status: "Running".into(),
                tasks_duration_nanos: 0,
            }],
            fetched_at: Instant::now(),
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Health(HealthPayload::PgStatus(snap))),
            &c,
        );
        (s, c)
    }

    #[test]
    fn health_number_key_switches_category() {
        let (mut s, c) = seeded_health_state();
        assert_eq!(s.health.selected_category, HealthCategory::Queues);
        // Press '2' → Repositories
        update(&mut s, key(KeyCode::Char('2'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.selected_category, HealthCategory::Repositories);
        // Press '3' → Nodes
        update(&mut s, key(KeyCode::Char('3'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.selected_category, HealthCategory::Nodes);
        // Press '4' → Processors
        update(&mut s, key(KeyCode::Char('4'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.selected_category, HealthCategory::Processors);
        // Press '1' → Queues
        update(&mut s, key(KeyCode::Char('1'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.selected_category, HealthCategory::Queues);
    }

    #[test]
    fn health_j_k_navigate_queues() {
        let (mut s, c) = seeded_health_state();
        // Only one row; down should wrap to 0, up should wrap to 0.
        assert_eq!(s.health.queues.selected, 0);
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.queues.selected, 0); // wrap back to 0
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.queues.selected, 0);
    }

    #[test]
    fn health_enter_on_queue_emits_open_in_browser() {
        let (mut s, c) = seeded_health_state();
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "conn-1");
                assert_eq!(group_id, "root");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
    }

    #[test]
    fn health_enter_on_processor_emits_open_in_browser() {
        let (mut s, c) = seeded_health_state();
        s.health.selected_category = HealthCategory::Processors;
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "proc-1");
                assert_eq!(group_id, "root");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
    }

    #[test]
    fn health_enter_on_repository_shows_info_banner() {
        let (mut s, c) = seeded_health_state();
        s.health.selected_category = HealthCategory::Repositories;
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(r.redraw);
        assert!(r.intent.is_none());
        let banner = s.status.banner.as_ref().expect("banner should be set");
        assert_eq!(banner.severity, crate::app::state::BannerSeverity::Info);
        assert!(
            banner.message.contains("not available"),
            "message should mention 'not available': {}",
            banner.message
        );
        assert!(
            banner.message.contains("repository"),
            "message should mention 'repository': {}",
            banner.message
        );
    }

    #[test]
    fn health_enter_on_node_shows_info_banner() {
        let (mut s, c) = seeded_health_state();
        s.health.selected_category = HealthCategory::Nodes;
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(r.redraw);
        assert!(r.intent.is_none());
        let banner = s.status.banner.as_ref().expect("banner should be set");
        assert_eq!(banner.severity, crate::app::state::BannerSeverity::Info);
        assert!(
            banner.message.contains("not available"),
            "message should mention 'not available': {}",
            banner.message
        );
        assert!(
            banner.message.contains("node"),
            "message should mention 'node': {}",
            banner.message
        );
    }

    #[test]
    fn health_r_emits_refresh_view() {
        let (mut s, c) = seeded_health_state();
        let r = update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::Dispatch(Intent::RefreshView(ViewId::Health))) => {}
            other => panic!("expected RefreshView(Health), got {other:?}"),
        }
    }

    #[test]
    fn system_diag_fallback_applies_data_and_sets_warning_banner() {
        use crate::app::state::BannerSeverity;
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, SystemDiagAggregate, SystemDiagSnapshot,
        };

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Health;

        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate::default(),
            nodes: vec![NodeDiagnostics {
                address: "Cluster (aggregate)".to_string(),
                heap_used_bytes: 4_000,
                heap_max_bytes: 8_000,
                gc: vec![GcSnapshot {
                    name: "G1".into(),
                    collection_count: 42,
                    collection_millis: 300,
                }],
                load_average: Some(1.5),
                total_threads: 100,
                uptime: "2 days".into(),
                content_repos: Vec::new(),
                flowfile_repo: None,
                provenance_repos: Vec::new(),
            }],
            fetched_at: Instant::now(),
        };

        update(
            &mut s,
            AppEvent::Data(ViewPayload::Health(HealthPayload::SystemDiagFallback {
                diag,
                warning: "nodewise diagnostics unavailable; showing cluster aggregate".into(),
            })),
            &c,
        );

        // The nodes table should have one row.
        assert_eq!(s.health.nodes.nodes.len(), 1);
        assert_eq!(s.health.nodes.nodes[0].node_address, "Cluster (aggregate)");

        // A warning banner should be set.
        let banner = s.status.banner.as_ref().expect("banner must be set");
        assert_eq!(banner.severity, BannerSeverity::Warning);
        assert!(banner.message.contains("aggregate"));
    }

    #[test]
    fn health_j_k_navigate_repositories() {
        use crate::client::health::{
            NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Health;
        s.health.selected_category = HealthCategory::Repositories;

        // Seed system diagnostics with 2 content repos + per-node data.
        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![
                    RepoUsage {
                        identifier: "content-1".into(),
                        used_bytes: 60_000,
                        total_bytes: 100_000,
                        free_bytes: 40_000,
                        utilization_percent: 60,
                    },
                    RepoUsage {
                        identifier: "content-2".into(),
                        used_bytes: 20_000,
                        total_bytes: 100_000,
                        free_bytes: 80_000,
                        utilization_percent: 20,
                    },
                ],
                flowfile_repo: None,
                provenance_repos: Vec::new(),
            },
            nodes: vec![NodeDiagnostics {
                address: "node1:8080".into(),
                heap_used_bytes: 1_000,
                heap_max_bytes: 8_000,
                gc: Vec::new(),
                load_average: None,
                total_threads: 10,
                uptime: "1h".into(),
                content_repos: vec![
                    RepoUsage {
                        identifier: "content-1".into(),
                        used_bytes: 55_000,
                        total_bytes: 100_000,
                        free_bytes: 45_000,
                        utilization_percent: 55,
                    },
                    RepoUsage {
                        identifier: "content-2".into(),
                        used_bytes: 15_000,
                        total_bytes: 100_000,
                        free_bytes: 85_000,
                        utilization_percent: 15,
                    },
                ],
                flowfile_repo: None,
                provenance_repos: Vec::new(),
            }],
            fetched_at: Instant::now(),
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Health(HealthPayload::SystemDiag(diag))),
            &c,
        );

        // Should have 2 rows, selected=0 (content-1).
        assert_eq!(s.health.repositories.rows.len(), 2);
        assert_eq!(s.health.repositories.selected, 0);
        assert_eq!(s.health.repositories.rows[0].identifier, "content-1");
        // per_node should have 1 node with content-1 data.
        assert_eq!(s.health.repositories.per_node.len(), 1);
        assert_eq!(s.health.repositories.per_node[0].utilization_percent, 55);

        // Press j → selected=1 (content-2), per_node rebuilds.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.repositories.selected, 1);
        assert_eq!(s.health.repositories.per_node.len(), 1);
        assert_eq!(s.health.repositories.per_node[0].utilization_percent, 15);

        // Press k → selected=0 (content-1), per_node rebuilds.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        assert_eq!(s.health.repositories.selected, 0);
        assert_eq!(s.health.repositories.per_node[0].utilization_percent, 55);
    }
}
