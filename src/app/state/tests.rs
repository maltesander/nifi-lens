use super::*;
use crate::config::{
    AuthConfig, Context, PasswordAuthConfig, PasswordCredentials, VersionStrategy,
};

pub(super) fn fresh_state() -> AppState {
    let c = tiny_config();
    AppState::new(
        "dev".into(),
        Version::new(2, 9, 0),
        &c,
        "https://nifi.test:8443".into(),
    )
}

pub(super) fn tiny_config() -> Config {
    Config {
        current_context: "dev".into(),
        bulletins: Default::default(),
        ui: Default::default(),
        polling: Default::default(),
        tracer: Default::default(),
        contexts: vec![
            Context {
                name: "dev".into(),
                url: "https://dev:8443".into(),
                auth: AuthConfig::Password(PasswordAuthConfig {
                    username: "admin".into(),
                    credentials: PasswordCredentials::Plain {
                        password: "x".into(),
                    },
                }),
                version_strategy: VersionStrategy::Strict,
                insecure_tls: false,
                ca_cert_path: None,
                proxied_entities_chain: None,
                proxy_url: None,
                http_proxy_url: None,
                https_proxy_url: None,
            },
            Context {
                name: "prod".into(),
                url: "https://prod:8443".into(),
                auth: AuthConfig::Password(PasswordAuthConfig {
                    username: "admin".into(),
                    credentials: PasswordCredentials::Plain {
                        password: "y".into(),
                    },
                }),
                version_strategy: VersionStrategy::Strict,
                insecure_tls: false,
                ca_cert_path: None,
                proxied_entities_chain: None,
                proxy_url: None,
                http_proxy_url: None,
                https_proxy_url: None,
            },
        ],
    }
}

pub(super) fn key(code: KeyCode, mods: KeyModifiers) -> AppEvent {
    AppEvent::Input(Event::Key(KeyEvent::new(code, mods)))
}

pub(super) fn seeded_browser_state() -> (AppState, Config) {
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use std::time::SystemTime;

    let mut s = fresh_state();
    let c = tiny_config();
    let snap = RecursiveSnapshot {
        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "root".into(),
                group_id: "root".into(),
                name: "root".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "gen".into(),
                group_id: "root".into(),
                name: "Gen".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "Running".into(),
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::ProcessGroup,
                id: "ingest".into(),
                group_id: "root".into(),
                name: "ingest".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
        ],
        fetched_at: SystemTime::now(),
    };
    crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
    s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));
    s.current_tab = ViewId::Browser;
    (s, c)
}

#[test]
fn tab_no_longer_cycles_tabs() {
    // Tab is now FocusAction::NextPane (pane cycling within a view),
    // not a tab-switch action. Pressing Tab must not change the active tab.
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Overview);
}

#[test]
fn back_tab_no_longer_cycles_tabs() {
    // BackTab is now FocusAction::PrevPane. It must not change the active tab.
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::BackTab, KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Overview);
}

#[test]
fn function_keys_goto_tabs() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::F(3), KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Browser);
    update(&mut s, key(KeyCode::F(4), KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Events);
}

#[test]
fn f_keys_leave_tracer_while_in_entry_mode() {
    // Regression: Tracer starts in TracerMode::Entry (UUID input),
    // which routes printable chars to handle_text_input but must NOT
    // suppress global F1-F5 tab-switch shortcuts. Otherwise the user
    // is trapped in the Tracer tab.
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Tracer;
    assert!(
        matches!(
            s.tracer.mode,
            crate::view::tracer::state::TracerMode::Entry(_)
        ),
        "Tracer should start in Entry mode"
    );
    update(&mut s, key(KeyCode::F(1), KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Overview, "F1 should leave Tracer");
}

#[test]
fn q_requests_quit() {
    let mut s = fresh_state();
    let c = tiny_config();
    let r = update(&mut s, key(KeyCode::Char('q'), KeyModifiers::NONE), &c);
    assert!(s.should_quit);
    assert!(matches!(r.intent, Some(PendingIntent::Quit)));
}

#[test]
fn ctrl_c_requests_quit() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('c'), KeyModifiers::CONTROL), &c);
    assert!(s.should_quit);
}

#[test]
fn help_modal_toggles() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
    assert!(matches!(s.modal, Some(Modal::Help)));
    update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn esc_closes_modal() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('?'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn esc_dismisses_the_status_banner_at_top_level() {
    let mut s = fresh_state();
    let c = tiny_config();
    // Seed a banner — any severity works.
    s.status.banner = Some(Banner {
        severity: BannerSeverity::Warning,
        message: "nodewise diagnostics unavailable".into(),
        detail: None,
    });
    // Ensure no modal is open so Esc reaches the global dispatch.
    assert!(s.modal.is_none());
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.status.banner.is_none(), "Esc must clear the banner");
}

#[test]
fn esc_with_no_banner_is_idempotent() {
    let mut s = fresh_state();
    let c = tiny_config();
    assert!(s.status.banner.is_none());
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.status.banner.is_none());
}

#[test]
fn next_input_clears_warning_banner() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.post_warning("clipboard: no display".to_string());
    // Any non-Ascend input should clear the warning toast.
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_none(),
        "warning banner must auto-clear on next input"
    );
}

#[test]
fn next_input_clears_info_banner() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.post_info("copied: foo".to_string());
    update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_none(),
        "info banner must auto-clear on next input"
    );
}

#[test]
fn next_input_does_not_clear_error_banner() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.post_error("query failed".to_string(), Some("stack".into()));
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_some(),
        "error banner must stay sticky across input events"
    );
    let b = s.status.banner.as_ref().unwrap();
    assert_eq!(b.severity, BannerSeverity::Error);
    assert_eq!(b.detail.as_deref(), Some("stack"));
}

#[test]
fn capital_k_opens_context_switcher() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
    let modal = s.modal.as_ref().unwrap();
    match modal {
        Modal::ContextSwitcher(cs) => {
            assert_eq!(cs.entries.len(), 2);
            assert!(cs.entries[0].is_active);
        }
        _ => panic!("expected ContextSwitcher"),
    }
}

#[test]
fn context_switcher_enter_emits_intent() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    match r.intent {
        Some(PendingIntent::SwitchContext(name)) => assert_eq!(name, "prod"),
        other => panic!("expected SwitchContext, got {other:?}"),
    }
}

#[test]
fn context_switched_outcome_updates_version_and_closes_modal() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
    let outcome = Ok(IntentOutcome::ContextSwitched {
        new_context_name: "other-ctx".into(),
        new_version: Version::new(2, 7, 2),
        new_base_url: "https://other.nifi:8443".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);
    assert_eq!(s.detected_version, Version::new(2, 7, 2));
    assert_eq!(s.context_name, "other-ctx");
    assert!(s.modal.is_none());
    assert!(s.pending_worker_restart, "worker restart must be flagged");
}

#[test]
fn context_switched_clears_cluster_summary_and_history() {
    use crate::app::history::{HistoryEntry, SelectionAnchor};

    let mut s = fresh_state();
    let c = tiny_config();
    s.cluster_summary = ClusterSummary {
        connected_nodes: Some(3),
        total_nodes: Some(3),
    };
    s.history.push(HistoryEntry {
        tab: ViewId::Browser,
        anchor: Some(SelectionAnchor::ComponentId("stale-cid".into())),
    });

    let outcome = Ok(IntentOutcome::ContextSwitched {
        new_context_name: "other-ctx".into(),
        new_version: Version::new(2, 7, 2),
        new_base_url: "https://other.nifi:8443".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);

    assert_eq!(s.cluster_summary.connected_nodes, None);
    assert_eq!(s.cluster_summary.total_nodes, None);
    assert!(!s.history.can_go_back(), "history must be wiped");
    assert!(!s.history.can_go_forward());
}

#[test]
fn context_switched_clears_events_results() {
    use crate::client::ProvenanceEventSummary;
    use crate::view::events::state::EventsQueryStatus;
    use std::time::SystemTime;

    let mut s = fresh_state();
    let c = tiny_config();
    s.events.events.push(ProvenanceEventSummary {
        event_id: 1,
        event_time_iso: "2026-04-17T00:00:00Z".into(),
        event_type: "DROP".into(),
        component_id: "cid".into(),
        component_name: "CName".into(),
        component_type: "PROCESSOR".into(),
        group_id: "gid".into(),
        flow_file_uuid: "ff-1".into(),
        relationship: None,
        details: None,
    });
    s.events.selected_row = Some(0);
    s.events.status = EventsQueryStatus::Done {
        fetched_at: SystemTime::now(),
        truncated: false,
        took_ms: 42,
    };

    let outcome = Ok(IntentOutcome::ContextSwitched {
        new_context_name: "other-ctx".into(),
        new_version: Version::new(2, 7, 2),
        new_base_url: "https://other.nifi:8443".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);

    assert!(
        s.events.events.is_empty(),
        "stale provenance results must be cleared"
    );
    assert!(s.events.selected_row.is_none());
    assert!(matches!(s.events.status, EventsQueryStatus::Idle));
}

#[test]
fn intent_error_sets_banner() {
    let mut s = fresh_state();
    let c = tiny_config();
    let err = NifiLensError::WriteIntentRefused {
        intent_name: "StartProcessor",
    };
    update(&mut s, AppEvent::IntentOutcome(Err(err)), &c);
    assert!(s.status.banner.is_some());
    assert_eq!(
        s.status.banner.as_ref().unwrap().severity,
        BannerSeverity::Error
    );
}

#[test]
fn cross_link_open_in_browser_pushes_history() {
    let (mut s, c) = seeded_browser_state();
    s.current_tab = ViewId::Bulletins;
    let outcome = Ok(IntentOutcome::OpenInBrowserTarget {
        component_id: "gen".into(),
        group_id: "root".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);
    assert!(s.history.can_go_back(), "back stack should have an entry");
    assert_eq!(s.current_tab, ViewId::Browser);
}

#[test]
fn cross_link_tracer_landing_pushes_history() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Browser;
    let outcome = Ok(IntentOutcome::TracerLandingOn {
        component_id: "some-comp".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);
    assert!(s.history.can_go_back(), "back stack should have an entry");
    assert_eq!(s.current_tab, ViewId::Tracer);
}

#[test]
fn open_parameter_context_modal_cross_link_opens_modal_with_preselect() {
    // seeded_browser_state has a PG with id "ingest" and name "ingest".
    let (mut s, c) = seeded_browser_state();
    let outcome = Ok(IntentOutcome::OpenParameterContextModalTarget {
        pg_id: "ingest".into(),
        preselect: Some("kafka_bootstrap".into()),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);

    let modal = s
        .browser
        .parameter_modal
        .as_ref()
        .expect("modal should be open");
    assert_eq!(modal.originating_pg_id, "ingest");
    assert_eq!(modal.preselect.as_deref(), Some("kafka_bootstrap"));
}

#[test]
fn open_parameter_context_modal_cross_link_unknown_pg_falls_back_to_id() {
    // When the PG is not in the arena, pg_path falls back to the bare id.
    let (mut s, c) = seeded_browser_state();
    let outcome = Ok(IntentOutcome::OpenParameterContextModalTarget {
        pg_id: "pg-unknown".into(),
        preselect: None,
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);

    let modal = s
        .browser
        .parameter_modal
        .as_ref()
        .expect("modal should be open even for unknown pg");
    assert_eq!(modal.originating_pg_id, "pg-unknown");
    // originating_pg_path falls back to the bare id when name not found.
    assert_eq!(modal.originating_pg_path, "pg-unknown");
}

#[test]
fn shift_left_navigates_history_back_replaces_bracket() {
    // `[` is unmapped; history back is now Shift+Left via the central
    // InputEvent::History(Back) dispatch.
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Bulletins;
    s.history.push(crate::app::history::HistoryEntry {
        tab: ViewId::Bulletins,
        anchor: None,
    });
    s.current_tab = ViewId::Browser;

    update(&mut s, key(KeyCode::Left, KeyModifiers::SHIFT), &c);
    assert_eq!(s.current_tab, ViewId::Bulletins);
}

#[test]
fn shift_right_navigates_forward() {
    // `]` is unmapped; history forward is now Shift+Right via the central
    // InputEvent::History(Forward) dispatch.
    let mut s = fresh_state();
    let c = tiny_config();
    // Simulate: was on Bulletins, pushed history, moved to Browser,
    // then popped back. Forward stack should have Browser.
    s.history.push(crate::app::history::HistoryEntry {
        tab: ViewId::Bulletins,
        anchor: None,
    });
    s.current_tab = ViewId::Browser;
    // Pop back to Bulletins (populates forward with Browser).
    let current = crate::app::history::HistoryEntry {
        tab: ViewId::Browser,
        anchor: None,
    };
    let entry = s.history.pop_back(current);
    assert!(entry.is_some());
    s.current_tab = ViewId::Bulletins;
    assert!(s.history.can_go_forward());

    update(&mut s, key(KeyCode::Right, KeyModifiers::SHIFT), &c);
    assert_eq!(s.current_tab, ViewId::Browser);
}

#[test]
fn left_bracket_noop_when_history_empty() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Browser;
    update(&mut s, key(KeyCode::Char('['), KeyModifiers::NONE), &c);
    // Tab unchanged — no history.
    assert_eq!(s.current_tab, ViewId::Browser);
}

#[test]
fn new_state_has_empty_cluster_summary() {
    let state = fresh_state();
    assert_eq!(state.cluster_summary.connected_nodes, None);
    assert_eq!(state.cluster_summary.total_nodes, None);
}

#[test]
fn fuzzy_find_modal_f_key_is_captured_as_query_character() {
    // Regression: the FuzzyFind close arm used to include Char('f') which
    // ate every search starting with `f`. Only Esc closes the modal now.
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Browser;
    // Seed the flow index so the fuzzy find modal can actually open.
    s.flow_index = Some(crate::view::browser::state::FlowIndex { entries: vec![] });
    // Open the modal via Shift+F.
    update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
    assert!(
        matches!(s.modal, Some(Modal::FuzzyFind(_))),
        "Shift+F should open the FuzzyFind modal"
    );
    // Type 'f' again — this should append to the query, NOT close the modal.
    update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
    assert!(
        matches!(s.modal, Some(Modal::FuzzyFind(_))),
        "second f should be captured as query char, not close the modal"
    );
    if let Some(Modal::FuzzyFind(ref fs)) = s.modal {
        assert_eq!(fs.query, "f", "query buffer should contain 'f'");
    }
    // Esc closes it.
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.modal.is_none(), "Esc should close the modal");
}

#[test]
fn collect_hints_advertises_new_bracket_chords_not_alt_arrows() {
    let mut s = fresh_state();
    // Put something in history so the back/fwd hints are emitted.
    s.history.push(crate::app::history::HistoryEntry {
        tab: ViewId::Bulletins,
        anchor: None,
    });
    s.current_tab = ViewId::Browser;
    let hints = collect_hints(&s);
    let hint_text: String = hints
        .iter()
        .map(|h| format!("{} {}", h.key, h.action))
        .collect::<Vec<_>>()
        .join(" | ");
    assert!(
        !hint_text.contains("Alt+"),
        "hint bar must not advertise old Alt+ chords: {hint_text}"
    );
}

#[test]
fn severity_filter_hints_are_hidden_from_status_bar() {
    let mut s = fresh_state();
    s.current_tab = ViewId::Bulletins;
    let hints = collect_hints(&s);
    // The three 1/2/3 hints must not appear in the status-bar strip —
    // they're surfaced by the [E n] [W n] [I n] chips one row above.
    assert!(
        !hints.iter().any(|h| h.key == "1"),
        "key '1' must not be in status bar; got {:?}",
        hints.iter().map(|h| h.key.as_ref()).collect::<Vec<_>>(),
    );
    assert!(
        !hints.iter().any(|h| h.key == "2"),
        "key '2' must not be in status bar"
    );
    assert!(
        !hints.iter().any(|h| h.key == "3"),
        "key '3' must not be in status bar"
    );
    // Sanity: other Bulletins hints are still present.
    assert!(hints.iter().any(|h| h.key == "/"), "other hints unaffected");
}

#[test]
fn capital_k_with_shift_opens_context_switcher() {
    // ContextSwitcher is bound to Shift+K via the central
    // InputEvent::App(ContextSwitcher) dispatch. The legacy loose match
    // for K-without-SHIFT is no longer supported.
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
    assert!(
        matches!(s.modal, Some(Modal::ContextSwitcher(_))),
        "Shift+K should open the context switcher"
    );
}

fn build_test_sysdiag_with_two_nodes() -> crate::client::overview::SystemDiagSnapshot {
    use crate::client::overview::{
        GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
    };
    use std::time::Instant;

    let node = |address: &str| NodeDiagnostics {
        address: address.into(),
        heap_used_bytes: crate::bytes::FIXTURE_HEAP_USED,
        heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
        gc: vec![GcSnapshot {
            name: "G1 Young".into(),
            collection_count: 10,
            collection_millis: 50,
        }],
        load_average: Some(1.5),
        available_processors: Some(4),
        total_threads: 50,
        uptime: "1h".into(),
        content_repos: vec![RepoUsage {
            identifier: "content".into(),
            used_bytes: 60,
            total_bytes: 100,
            free_bytes: 40,
            utilization_percent: 60,
        }],
        flowfile_repo: Some(RepoUsage {
            identifier: "flowfile".into(),
            used_bytes: 30,
            total_bytes: 100,
            free_bytes: 70,
            utilization_percent: 30,
        }),
        provenance_repos: vec![RepoUsage {
            identifier: "provenance".into(),
            used_bytes: 20,
            total_bytes: 100,
            free_bytes: 80,
            utilization_percent: 20,
        }],
    };

    SystemDiagSnapshot {
        aggregate: SystemDiagAggregate {
            content_repos: vec![RepoUsage {
                identifier: "content".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "flowfile".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "provenance".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        },
        nodes: vec![node("node1:8080"), node("node2:8080")],
        fetched_at: Instant::now(),
    }
}

#[test]
fn sysdiag_redraw_populates_cluster_summary() {
    use crate::cluster::snapshot::{EndpointState, FetchMeta};
    use std::time::{Duration, Instant};

    let mut s = fresh_state();

    // Pre-condition: cluster_summary is empty placeholder.
    assert_eq!(s.cluster_summary.connected_nodes, None);
    assert_eq!(s.cluster_summary.total_nodes, None);

    // Seed the cluster snapshot and invoke `redraw_sysdiag`
    // directly. The main-loop `ClusterChanged` arm
    // (`src/app/mod.rs`) routes to this reducer; the reducer test
    // is the canonical coverage for the projection logic.
    s.cluster.snapshot.system_diagnostics = EndpointState::Ready {
        data: build_test_sysdiag_with_two_nodes(),
        meta: FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: Duration::from_secs(30),
        },
    };

    crate::view::overview::state::redraw_sysdiag(&mut s);

    assert_eq!(s.cluster_summary.total_nodes, Some(2));
    // NodeDiagnostics has no status field, so connected_nodes equals total.
    assert_eq!(s.cluster_summary.connected_nodes, Some(2));
}

#[test]
fn context_switcher_row_nav_uses_arrows_only_no_jk() {
    // Open the context switcher (2 entries via tiny_config).
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
    let before = match s.modal.as_ref().unwrap() {
        Modal::ContextSwitcher(cs) => cs.cursor,
        _ => panic!("expected ContextSwitcher"),
    };

    // j is a no-op inside the modal.
    update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
    let after_j = match s.modal.as_ref().unwrap() {
        Modal::ContextSwitcher(cs) => cs.cursor,
        _ => panic!("expected ContextSwitcher"),
    };
    assert_eq!(after_j, before, "j dropped");

    // Down still moves the cursor.
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    let after_down = match s.modal.as_ref().unwrap() {
        Modal::ContextSwitcher(cs) => cs.cursor,
        _ => panic!("expected ContextSwitcher"),
    };
    assert!(after_down > before, "Down still works");

    let before = after_down;
    // k is a no-op.
    update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
    let after_k = match s.modal.as_ref().unwrap() {
        Modal::ContextSwitcher(cs) => cs.cursor,
        _ => panic!("expected ContextSwitcher"),
    };
    assert_eq!(after_k, before, "k dropped");

    // Up still moves the cursor back.
    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    let after_up = match s.modal.as_ref().unwrap() {
        Modal::ContextSwitcher(cs) => cs.cursor,
        _ => panic!("expected ContextSwitcher"),
    };
    assert!(after_up < before, "Up still works");
}

#[test]
fn events_landing_on_seeds_filters_and_switches_tab() {
    let mut s = fresh_state();
    let c = tiny_config();
    let outcome = crate::event::IntentOutcome::EventsLandingOn {
        component_id: "proc-42".into(),
    };
    let r = update(&mut s, AppEvent::IntentOutcome(Ok(outcome)), &c);
    assert_eq!(s.current_tab, ViewId::Events);
    assert_eq!(s.events.filters.source, "proc-42");
    assert_eq!(s.events.filters.time, "last 15m");
    assert!(matches!(
        s.events.status,
        crate::view::events::state::EventsQueryStatus::Running { .. }
    ));
    assert!(matches!(
        r.intent,
        Some(PendingIntent::RunProvenanceQuery { .. })
    ));
}

#[test]
fn tab_switch_away_from_events_clears_failed_status() {
    use crate::event::{EventsPayload, ViewPayload};
    use crate::view::events::state::EventsQueryStatus;

    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Events;

    // Drive the events state into Running so QueryFailed applies.
    update(
        &mut s,
        AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
            query_id: "q-1".into(),
        })),
        &c,
    );
    update(
        &mut s,
        AppEvent::Data(ViewPayload::Events(EventsPayload::QueryFailed {
            query_id: Some("q-1".into()),
            error: "boom".into(),
        })),
        &c,
    );
    assert!(matches!(s.events.status, EventsQueryStatus::Failed { .. }));

    // Press F1 to switch to Overview.
    update(&mut s, key(KeyCode::F(1), KeyModifiers::NONE), &c);
    assert_eq!(s.current_tab, ViewId::Overview);
    assert!(
        matches!(s.events.status, EventsQueryStatus::Idle),
        "leaving Events must reset Failed to Idle"
    );
}

fn seed_one_bulletin(state: &mut AppState) {
    use crate::client::BulletinSnapshot;
    state.bulletins.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "ERROR".into(),
        message: "test-msg".into(),
        source_id: "src-42".into(),
        source_name: "Proc-42".into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: "2026-04-14T00:00:00Z".into(),
        timestamp_human: String::new(),
    });
}

#[test]
fn shift_left_navigates_history_back() {
    use crossterm::event::{KeyEvent, KeyModifiers};

    let mut s = fresh_state();
    let c = tiny_config();

    // Build a history: start on Overview, move to Bulletins, then
    // history back should return to Overview.
    s.history.push(crate::app::history::HistoryEntry {
        tab: ViewId::Overview,
        anchor: None,
    });
    s.current_tab = ViewId::Bulletins;

    let r = update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Left,
            KeyModifiers::SHIFT,
        ))),
        &c,
    );
    assert!(r.redraw);
    assert_eq!(s.current_tab, ViewId::Overview);
}

#[test]
fn g_from_bulletins_opens_goto_menu_then_enter_gotos_to_browser() {
    use crossterm::event::{KeyEvent, KeyModifiers};

    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Bulletins;
    seed_one_bulletin(&mut s);

    // Press `g` — maps to AppAction::Goto; with Browser + Events cross-links
    // available, a GotoMenu modal opens (no intent emitted yet).
    let r1 = update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
        ))),
        &c,
    );
    assert!(r1.intent.is_none());
    assert!(matches!(s.modal, Some(Modal::GotoMenu(_))));

    // Press Enter — selects index 0 = Browser (the first cross-link).
    let r2 = update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
        &c,
    );
    assert!(matches!(
        r2.intent,
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser { .. }))
    ));
}

#[test]
fn handle_verb_toggles_error_filter_after_port() {
    // After the Bulletins port (Phase 3 Task 12), handle_verb dispatches
    // directly — ToggleSeverity(Error) flips show_error immediately.
    use crate::input::{Severity, ViewVerb};

    let mut s = fresh_state();
    s.current_tab = ViewId::Bulletins;
    let before = s.bulletins.filters.show_error;
    let _ = bulletins::BulletinsHandler::handle_verb(
        &mut s,
        ViewVerb::Bulletins(crate::input::BulletinsVerb::ToggleSeverity(Severity::Error)),
    );
    assert_ne!(
        s.bulletins.filters.show_error, before,
        "handle_verb must toggle show_error after Bulletins port"
    );
}

#[test]
fn bare_e_does_not_open_error_detail() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut s = fresh_state();
    let c = tiny_config();
    s.status.banner = Some(Banner {
        severity: BannerSeverity::Error,
        message: "test".into(),
        detail: Some("detail".into()),
    });
    update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
        ))),
        &c,
    );
    assert!(
        !matches!(s.modal, Some(Modal::ErrorDetail)),
        "bare 'e' must not open error detail"
    );
}

#[test]
fn enter_on_error_banner_opens_detail() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut s = fresh_state();
    let c = tiny_config();
    s.status.banner = Some(Banner {
        severity: BannerSeverity::Error,
        message: "test".into(),
        detail: Some("detail".into()),
    });
    update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Enter,
            KeyModifiers::NONE,
        ))),
        &c,
    );
    assert!(matches!(s.modal, Some(Modal::ErrorDetail)));
}

#[test]
fn esc_dismisses_error_banner_via_ascend() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut s = fresh_state();
    let c = tiny_config();
    s.status.banner = Some(Banner {
        severity: BannerSeverity::Error,
        message: "test".into(),
        detail: None,
    });
    update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))),
        &c,
    );
    assert!(s.status.banner.is_none(), "Esc must dismiss the banner");
}

#[test]
fn overview_noisy_g_b_builds_open_in_browser() {
    use crate::view::overview::state::{NoisyComponent, OverviewFocus, Severity as OvSev};
    let mut s = fresh_state();
    s.current_tab = ViewId::Overview;
    s.overview.focus = OverviewFocus::Noisy;
    s.overview.noisy = vec![NoisyComponent {
        source_id: "proc-1".into(),
        group_id: "grp-1".into(),
        source_name: "MyProc".into(),
        count: 3,
        max_severity: OvSev::Error,
    }];
    s.overview.noisy_selected = 0;
    let link = build_go_crosslink(&s, crate::input::GoTarget::Browser);
    assert!(
        matches!(&link, Some(CrossLink::OpenInBrowser { component_id, group_id })
                if component_id == "proc-1" && group_id == "grp-1"),
        "got {link:?}"
    );
}

#[test]
fn overview_noisy_g_e_builds_goto_events() {
    use crate::view::overview::state::{NoisyComponent, OverviewFocus, Severity as OvSev};
    let mut s = fresh_state();
    s.current_tab = ViewId::Overview;
    s.overview.focus = OverviewFocus::Noisy;
    s.overview.noisy = vec![NoisyComponent {
        source_id: "proc-2".into(),
        group_id: "grp-2".into(),
        source_name: "OtherProc".into(),
        count: 1,
        max_severity: OvSev::Warning,
    }];
    s.overview.noisy_selected = 0;
    let link = build_go_crosslink(&s, crate::input::GoTarget::Events);
    assert!(
        matches!(&link, Some(CrossLink::GotoEvents { component_id })
                if component_id == "proc-2"),
        "got {link:?}"
    );
}

#[test]
fn overview_queues_g_b_builds_open_in_browser() {
    use crate::view::overview::state::{OverviewFocus, UnhealthyQueue};
    let mut s = fresh_state();
    s.current_tab = ViewId::Overview;
    s.overview.focus = OverviewFocus::Queues;
    s.overview.unhealthy = vec![UnhealthyQueue {
        id: "conn-1".into(),
        group_id: "grp-3".into(),
        name: "q1".into(),
        source_name: "A".into(),
        destination_name: "B".into(),
        fill_percent: 80,
        flow_files_queued: 800,
        bytes_queued: 0,
        queued_display: "800".into(),
    }];
    s.overview.queues_selected = 0;
    let link = build_go_crosslink(&s, crate::input::GoTarget::Browser);
    assert!(
        matches!(&link, Some(CrossLink::OpenInBrowser { component_id, group_id })
                if component_id == "grp-3" && group_id == "grp-3"),
        "got {link:?}"
    );
}

#[test]
fn overview_no_focus_g_b_returns_none() {
    use crate::view::overview::state::OverviewFocus;
    let mut s = fresh_state();
    s.current_tab = ViewId::Overview;
    s.overview.focus = OverviewFocus::None;
    assert!(build_go_crosslink(&s, crate::input::GoTarget::Browser).is_none());
}

#[test]
fn selection_cross_links_empty_on_folder_row() {
    use crate::client::{FolderKind, NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    let mut s = fresh_state();
    s.current_tab = ViewId::Browser;
    crate::view::browser::state::apply_tree_snapshot(
        &mut s.browser,
        RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ControllerService,
                    id: "cs".into(),
                    group_id: "root".into(),
                    name: "pool".into(),
                    status_summary: NodeStatusSummary::ControllerService {
                        state: "ENABLED".into(),
                    },
                },
            ],
            fetched_at: std::time::SystemTime::now(),
        },
    );
    let folder_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| matches!(n.kind, NodeKind::Folder(FolderKind::ControllerServices)))
        .unwrap();
    s.browser.selected = s
        .browser
        .visible
        .iter()
        .position(|&i| i == folder_arena)
        .unwrap();
    assert!(
        s.selection_cross_links().is_empty(),
        "folder row must not produce any cross-link targets"
    );
}

#[test]
fn g_from_bulletins_no_selection_is_noop() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Bulletins;
    // With no bulletin selected, cross-links are empty → no-op.
    let r = update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
    assert!(r.intent.is_none());
    assert!(s.modal.is_none(), "no modal when no cross-links");
}

#[test]
fn g_auto_gotos_directly_when_single_cross_link() {
    // Overview + Queues focus: only GoTarget::Browser is available (Events
    // returns None for Queues focus), so selection_cross_links() → [Browser].
    // The single-target arm must fire a JumpTo intent without opening a modal.
    use crate::view::overview::state::{OverviewFocus, UnhealthyQueue};
    use crossterm::event::{KeyEvent, KeyModifiers};

    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Overview;
    s.overview.focus = OverviewFocus::Queues;
    s.overview.unhealthy = vec![UnhealthyQueue {
        id: "conn-1".into(),
        group_id: "grp-3".into(),
        name: "q1".into(),
        source_name: "A".into(),
        destination_name: "B".into(),
        fill_percent: 80,
        flow_files_queued: 800,
        bytes_queued: 0,
        queued_display: "800".into(),
    }];
    s.overview.queues_selected = 0;

    // Verify precondition: exactly one cross-link available.
    assert_eq!(
        s.selection_cross_links(),
        vec![crate::input::GoTarget::Browser],
        "expected exactly [Browser] for Overview+Queues"
    );

    let r = update(
        &mut s,
        AppEvent::Input(Event::Key(KeyEvent::new(
            KeyCode::Char('g'),
            KeyModifiers::NONE,
        ))),
        &c,
    );

    assert!(
        matches!(
            r.intent,
            Some(PendingIntent::Goto(CrossLink::OpenInBrowser { .. }))
        ),
        "single cross-link must auto-goto without a modal; got intent={:?}",
        r.intent
    );
    assert!(
        s.modal.is_none(),
        "no modal should open for single-target auto-goto"
    );
}

// ── banner + modal helper methods ────────────────────────────────

#[test]
fn post_error_sets_error_severity_and_detail() {
    let mut s = fresh_state();
    s.post_error("boom".to_string(), Some("stack".to_string()));
    let b = s.status.banner.as_ref().expect("banner was set");
    assert_eq!(b.severity, BannerSeverity::Error);
    assert_eq!(b.message, "boom");
    assert_eq!(b.detail.as_deref(), Some("stack"));
}

#[test]
fn post_info_replaces_prior_banner() {
    let mut s = fresh_state();
    s.post_error("err".to_string(), Some("d".to_string()));
    s.post_info("copied: foo".to_string());
    let b = s.status.banner.as_ref().expect("banner was set");
    assert_eq!(b.severity, BannerSeverity::Info);
    assert_eq!(b.message, "copied: foo");
    assert!(
        b.detail.is_none(),
        "post_info must not carry over prior detail"
    );
}

#[test]
fn post_warning_sets_warning_severity() {
    let mut s = fresh_state();
    s.post_warning("clipboard: no display".to_string());
    let b = s.status.banner.as_ref().expect("banner was set");
    assert_eq!(b.severity, BannerSeverity::Warning);
    assert_eq!(b.message, "clipboard: no display");
    assert!(b.detail.is_none());
}

#[test]
fn open_banner_detail_is_noop_without_banner() {
    let mut s = fresh_state();
    assert!(!s.open_banner_detail());
    assert!(s.modal.is_none());
    assert!(s.error_detail.is_none());
}

#[test]
fn open_banner_detail_is_noop_when_banner_has_no_detail() {
    let mut s = fresh_state();
    s.post_info("copied".to_string());
    assert!(!s.open_banner_detail());
    assert!(s.modal.is_none());
    assert!(s.error_detail.is_none());
}

#[test]
fn open_banner_detail_copies_detail_and_sets_modal() {
    let mut s = fresh_state();
    s.post_error("boom".to_string(), Some("full chain".to_string()));
    assert!(s.open_banner_detail());
    assert!(matches!(s.modal, Some(Modal::ErrorDetail)));
    assert_eq!(s.error_detail.as_deref(), Some("full chain"));
}

#[test]
fn close_modal_clears_both_modal_and_error_detail() {
    let mut s = fresh_state();
    s.post_error("boom".to_string(), Some("d".to_string()));
    s.open_banner_detail();
    assert!(s.modal.is_some());
    assert!(s.error_detail.is_some());
    s.close_modal();
    assert!(s.modal.is_none());
    assert!(s.error_detail.is_none());
}

mod goto_subject_tests {
    use super::*;
    use crate::client::BulletinSnapshot;
    use crate::input::GoTarget;
    use crate::test_support::fresh_state;
    use crate::widget::goto_menu::GotoSubject;

    fn stock_bulletin() -> BulletinSnapshot {
        BulletinSnapshot {
            id: 1,
            level: "WARNING".into(),
            message: "boom".into(),
            source_id: "src-a".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "grp-a".into(),
            timestamp_iso: "2026-04-17T00:00:00Z".into(),
            timestamp_human: String::new(),
        }
    }

    fn stock_event() -> crate::client::ProvenanceEventSummary {
        crate::client::ProvenanceEventSummary {
            event_id: 7,
            event_time_iso: "2026-04-17T00:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "cid".into(),
            component_name: "CName".into(),
            component_type: "PROCESSOR".into(),
            group_id: "gid".into(),
            flow_file_uuid: "ff-42".into(),
            relationship: None,
            details: None,
        }
    }

    #[test]
    fn bulletins_browser_subject_is_component_with_name_and_id() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        s.bulletins.ring.push_front(stock_bulletin());
        s.bulletins.selected = 0;
        let subject = build_goto_subject(&s, GoTarget::Browser).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Component {
                name: "ProcA".into(),
                id: "src-a".into(),
            }
        );
    }

    #[test]
    fn events_tracer_subject_is_flowfile_uuid() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        s.events.events.push(stock_event());
        s.events.selected_row = Some(0);

        let subject = build_goto_subject(&s, GoTarget::Tracer).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Flowfile {
                uuid: "ff-42".into()
            }
        );
    }

    #[test]
    fn events_browser_subject_is_component_name_and_id() {
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        s.events.events.push(stock_event());
        s.events.selected_row = Some(0);

        let subject = build_goto_subject(&s, GoTarget::Browser).expect("subject");
        assert_eq!(
            subject,
            GotoSubject::Component {
                name: "CName".into(),
                id: "cid".into(),
            }
        );
    }

    #[test]
    fn no_selection_returns_none() {
        let s = fresh_state();
        // Default tab = Overview with no noisy/queue selection populated.
        assert!(build_goto_subject(&s, GoTarget::Browser).is_none());
    }
}
