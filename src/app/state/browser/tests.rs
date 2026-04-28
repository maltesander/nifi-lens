use super::super::tests::{fresh_state, key, seeded_browser_state, tiny_config};
use super::super::update;
use super::BrowserHandler;
use crate::app::state::{AppState, BannerSeverity, Modal, PendingIntent, ViewId, ViewKeyHandler};
use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
use crate::config::Config;
use crate::event::AppEvent;
use crate::intent::CrossLink;
use crate::view::browser::state::{
    FlowIndex, FlowIndexEntry, MAX_DETAIL_SECTIONS, PropertiesModalState,
};
use crossterm::event::{KeyCode, KeyModifiers};
use std::time::SystemTime;

#[test]
fn rebuild_arena_from_cluster_populates_browser_state_and_flow_index() {
    use crate::cluster::snapshot::{EndpointState, FetchMeta};
    use std::time::{Duration, Instant};

    let mut s = fresh_state();
    let meta = FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: Duration::from_millis(10),
        next_interval: Duration::from_secs(10),
    };
    let mut root_pg = crate::test_support::tiny_root_pg_status();
    // Mutate `nodes` to add one processor under the root PG so the
    // flow-index has two entries.
    root_pg.nodes.push(RawNode {
        parent_idx: Some(0),
        kind: NodeKind::Processor,
        id: "gen".into(),
        group_id: "root".into(),
        name: "Gen".into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
    });
    s.cluster.snapshot.root_pg_status = EndpointState::Ready {
        data: root_pg,
        meta,
    };
    let snap = s.cluster.snapshot.clone();
    crate::view::browser::state::rebuild_arena_from_cluster(&mut s, &snap);
    assert_eq!(s.browser.nodes.len(), 2);
    assert_eq!(s.browser.visible.len(), 2); // root expanded -> 1 child visible
    let idx = s.flow_index.as_ref().expect("FlowIndex built");
    assert_eq!(idx.entries.len(), 2);
}

#[test]
fn open_in_browser_target_switches_tab_and_expands_ancestors() {
    let mut s = fresh_state();
    let c = tiny_config();
    // Seed a small tree: root → ingest → upd.
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
            RawNode {
                parent_idx: Some(1),
                kind: NodeKind::Processor,
                id: "upd".into(),
                group_id: "ingest".into(),
                name: "UpdateAttribute".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "Running".into(),
                },
            },
        ],
        fetched_at: SystemTime::now(),
    };
    crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
    s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));

    // Jump to "upd".
    let outcome = Ok(crate::event::IntentOutcome::OpenInBrowserTarget {
        component_id: "upd".into(),
        group_id: "ingest".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);
    assert_eq!(s.current_tab, ViewId::Browser);
    let arena = s.browser.nodes.iter().position(|n| n.id == "upd").unwrap();
    let visible = s.browser.visible.iter().position(|&i| i == arena).unwrap();
    assert_eq!(s.browser.selected, visible);
    // Ancestor expanded: "ingest" (arena 1) ∈ expanded.
    assert!(s.browser.expanded.contains(&1));
}

#[test]
fn open_in_browser_target_warns_when_id_not_in_arena() {
    let mut s = fresh_state();
    let c = tiny_config();
    let outcome = Ok(crate::event::IntentOutcome::OpenInBrowserTarget {
        component_id: "ghost".into(),
        group_id: "root".into(),
    });
    update(&mut s, AppEvent::IntentOutcome(outcome), &c);
    assert_eq!(s.current_tab, ViewId::Browser);
    let banner = s.status.banner.as_ref().unwrap();
    assert_eq!(banner.severity, BannerSeverity::Warning);
    assert!(banner.message.contains("ghost"));
}

#[test]
fn on_browser_tab_down_moves_selection_down() {
    let (mut s, c) = seeded_browser_state();
    assert_eq!(s.browser.selected, 0);
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    assert_eq!(s.browser.selected, 1);
}

#[test]
fn m_on_unversioned_selection_is_silent_no_op() {
    // The verb is grayed out in the hint bar via the `enabled`
    // predicate, but the keymap still dispatches the key. The reducer
    // must silently no-op rather than posting a sticky warning banner.
    let (mut s, c) = seeded_browser_state();
    // The seeded tree has no version-controlled PGs.
    assert!(!s.browser_selection_is_versioned_pg());
    update(&mut s, key(KeyCode::Char('m'), KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_none(),
        "m on a non-versioned selection must not post a banner"
    );
    assert!(
        s.browser.version_modal.is_none(),
        "m on a non-versioned selection must not open the modal"
    );
}

#[test]
fn open_parameter_context_disabled_on_pg_with_no_binding() {
    // `p` on a PG with no bound parameter context must be a silent no-op.
    // The enabled() predicate returns false so the hint bar grays out the
    // verb; the reducer defensive guard catches any bypass.
    let (mut s, c) = seeded_browser_state();
    // seeded_browser_state has "ingest" (index 2) with no parameter context.
    s.browser.selected = 2;
    assert!(
        !s.browser_selection_pg_has_parameter_context_binding(),
        "seeded PG must have no binding"
    );
    update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
    assert!(
        s.browser.parameter_modal.is_none(),
        "p on a PG with no binding must not open the modal"
    );
    assert!(
        s.status.banner.is_none(),
        "p on a PG with no binding must not post a banner"
    );
}

#[test]
fn on_browser_tab_enter_on_collapsed_pg_drills_in() {
    let (mut s, c) = seeded_browser_state();
    // Move selection to "ingest" (visible row 2 in a seeded tree with
    // root expanded and "gen" as first child).
    s.browser.selected = 2;
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    assert!(s.browser.expanded.contains(&2));
}

#[test]
fn on_browser_tab_left_on_expanded_pg_collapses() {
    let (mut s, c) = seeded_browser_state();
    s.browser.expanded.insert(2);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = 2;
    update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
    assert!(!s.browser.expanded.contains(&2));
}

#[test]
fn on_browser_tab_r_force_notifies_cluster_endpoints() {
    // Task 6: Browser's arena is rebuilt from the cluster snapshot,
    // so `r` no longer consumes a per-worker oneshot. Instead it
    // calls `cluster.force(...)` for every endpoint the arena
    // depends on; each of those `Arc<Notify>`s wakes its sleeping
    // fetcher loop. We assert the three per-endpoint notifies fire
    // by registering a waiter on each and verifying it wakes.
    let local = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    local.block_on(async {
        use crate::cluster::ClusterEndpoint;
        let local_set = tokio::task::LocalSet::new();
        local_set
            .run_until(async {
                let (mut s, c) = seeded_browser_state();
                let notifies: Vec<_> = [
                    ClusterEndpoint::RootPgStatus,
                    ClusterEndpoint::ControllerServices,
                    ClusterEndpoint::ConnectionsByPg,
                ]
                .into_iter()
                .map(|ep| s.cluster.notify_for(ep))
                .collect();
                let flags: Vec<_> = (0..3)
                    .map(|_| std::rc::Rc::new(std::cell::Cell::new(false)))
                    .collect();
                let waiters: Vec<_> = notifies
                    .iter()
                    .zip(flags.iter())
                    .map(|(notify, flag)| {
                        let notify = notify.clone();
                        let flag = flag.clone();
                        tokio::task::spawn_local(async move {
                            notify.notified().await;
                            flag.set(true);
                        })
                    })
                    .collect();
                tokio::task::yield_now().await;
                update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
                tokio::task::yield_now().await;
                for (i, flag) in flags.iter().enumerate() {
                    assert!(flag.get(), "endpoint #{i} notify did not fire on r");
                }
                for w in waiters {
                    w.abort();
                }
            })
            .await;
    });
}

#[test]
fn f_with_no_index_shows_warning_banner_and_does_not_open_modal() {
    let mut s = fresh_state();
    let c = tiny_config();
    update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
    assert!(s.modal.is_none());
    assert!(
        s.status
            .banner
            .as_ref()
            .map(|b| b.severity == BannerSeverity::Warning)
            .unwrap_or(false)
    );
}

#[test]
fn f_with_index_opens_fuzzy_find_modal() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.flow_index = Some(FlowIndex {
        entries: vec![FlowIndexEntry {
            id: "p".into(),
            group_id: "root".into(),
            kind: NodeKind::Processor,
            name: "P".into(),
            group_path: "root".into(),
            state: crate::view::browser::state::StateBadge::Processor {
                glyph: '\u{25CF}',
                style: crate::theme::success(),
            },
            haystack: "p   processor   root".into(),
            version_state: None,
        }],
    });
    update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
    assert!(matches!(s.modal, Some(Modal::FuzzyFind(_))));
}

#[test]
fn fuzzy_find_modal_enter_emits_open_in_browser_intent() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.flow_index = Some(FlowIndex {
        entries: vec![FlowIndexEntry {
            id: "target".into(),
            group_id: "g".into(),
            kind: NodeKind::Processor,
            name: "PutKafka".into(),
            group_path: "root".into(),
            state: crate::view::browser::state::StateBadge::Processor {
                glyph: '\u{25CF}',
                style: crate::theme::success(),
            },
            haystack: "putkafka   processor   root".into(),
            version_state: None,
        }],
    });
    update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
    update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
    let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    match r.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser { component_id, .. })) => {
            assert_eq!(component_id, "target");
        }
        other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
    }
    assert!(s.modal.is_none());
}

#[test]
fn fuzzy_find_modal_esc_closes_without_goto() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.flow_index = Some(FlowIndex {
        entries: vec![FlowIndexEntry {
            id: "x".into(),
            group_id: "g".into(),
            kind: NodeKind::Processor,
            name: "X".into(),
            group_path: "root".into(),
            state: crate::view::browser::state::StateBadge::Processor {
                glyph: '\u{25CF}',
                style: crate::theme::success(),
            },
            haystack: "x   processor   root".into(),
            version_state: None,
        }],
    });
    update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
    let r = update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
    assert!(r.intent.is_none());
}

#[test]
fn p_on_processor_with_detail_opens_properties_modal() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::NodeDetail;

    let (mut s, c) = seeded_browser_state();
    // Seed detail for "gen" (arena 1).
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("k".into(), "v".into())],
            validation_errors: vec![],
        }),
    );
    s.browser.selected = 1; // visible row for arena 1
    update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
    assert!(matches!(s.modal, Some(Modal::Properties(_))));
}

#[test]
fn e_no_longer_opens_properties_modal() {
    // `e` used to open properties; now it is a no-op — use `p` instead.
    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 1;
    update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn p_on_processor_without_detail_is_noop() {
    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 1;
    update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn e_on_pg_is_noop() {
    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 0; // root PG
    update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn esc_closes_properties_modal() {
    let (mut s, c) = seeded_browser_state();
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(s.modal.is_none());
}

#[test]
fn properties_modal_ctrl_p_does_not_close() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::{NodeDetail, PropertiesModalState};
    let (mut s, c) = seeded_browser_state();
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("K".into(), "v".into())],
            validation_errors: vec![],
        }),
    );
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));

    update(&mut s, key(KeyCode::Char('p'), KeyModifiers::CONTROL), &c);

    assert!(
        matches!(s.modal, Some(Modal::Properties(_))),
        "Ctrl+P must NOT dismiss the Properties modal (it's the FuzzyFind Up chord; modifier must be guarded)"
    );
}

#[test]
fn t_is_no_longer_a_goto_events_shortcut() {
    // `t` used to emit GotoEvents; that shortcut is retired.
    // Users now navigate via `g` which opens the AppAction::Goto modal.
    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 1; // "gen" processor
    let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
    assert!(
        r.intent.is_none(),
        "t must no longer emit GotoEvents; got {r:?}"
    );
}

/// Build a 3-level tree: Root (PG) > Pipeline (PG) > Generate (Processor).
/// Root and Pipeline are expanded so all three are visible.
/// Returns (state, config) with `current_tab` set to Browser.
fn three_level_browser_state() -> (AppState, Config) {
    let mut s = fresh_state();
    let c = tiny_config();
    let snap = RecursiveSnapshot {
        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "root".into(),
                group_id: "root".into(),
                name: "Root".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::ProcessGroup,
                id: "pipeline".into(),
                group_id: "root".into(),
                name: "Pipeline".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(1),
                kind: NodeKind::Processor,
                id: "gen".into(),
                group_id: "pipeline".into(),
                name: "Generate".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "Running".into(),
                },
            },
        ],
        fetched_at: SystemTime::now(),
    };
    crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
    s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));
    // Expand Pipeline so Generate is visible.
    s.browser.expanded.insert(1);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.current_tab = ViewId::Browser;
    (s, c)
}

#[test]
fn b_is_no_longer_breadcrumb_activation() {
    // `b` used to enter breadcrumb mode; the interactive breadcrumb mode
    // has been removed entirely. Pressing `b` must be a no-op.
    let (mut s, c) = three_level_browser_state();
    s.browser.selected = 2;
    let before_selected = s.browser.selected;
    update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
    assert_eq!(s.browser.selected, before_selected, "b must be a no-op");
}

#[test]
fn b_at_root_is_noop() {
    // `b` is a no-op on both leaf and root nodes.
    let (mut s, c) = three_level_browser_state();
    s.browser.selected = 0;
    let before_selected = s.browser.selected;
    update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.selected, before_selected,
        "b must be a no-op at root"
    );
}

#[test]
fn left_key_collapses_expanded_pg_in_tree_focus() {
    // Left collapses an expanded PG (replaces old backspace/h behavior).
    let (mut s, c) = three_level_browser_state();
    // Pipeline (arena 1) is expanded. Select it.
    let pipeline_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "pipeline")
        .unwrap();
    let pipeline_vis = s
        .browser
        .visible
        .iter()
        .position(|&i| i == pipeline_arena)
        .unwrap();
    s.browser.selected = pipeline_vis;
    assert!(s.browser.expanded.contains(&pipeline_arena));
    update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
    assert!(
        !s.browser.expanded.contains(&pipeline_arena),
        "Left on expanded PG should collapse it"
    );
}

#[test]
fn right_key_expands_collapsed_pg_in_tree_focus() {
    // Right expands a collapsed PG (replaces old Enter/Right behavior).
    let (mut s, c) = three_level_browser_state();
    // Collapse Pipeline first.
    s.browser.expanded.remove(&1);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    let pipeline_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "pipeline")
        .unwrap();
    let pipeline_vis = s
        .browser
        .visible
        .iter()
        .position(|&i| i == pipeline_arena)
        .unwrap();
    s.browser.selected = pipeline_vis;
    let before = s.browser.visible.clone();
    update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
    assert_ne!(s.browser.visible, before, "Right should expand the PG");
}

#[test]
fn enter_on_collapsed_pg_expands_and_moves_to_child() {
    // Enter (Descend) on a collapsed PG expands and selects the first child.
    let (mut s, c) = three_level_browser_state();
    // Collapse Pipeline so Enter on it will expand.
    s.browser.expanded.remove(&1);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    let pipeline_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "pipeline")
        .unwrap();
    let pipeline_vis = s
        .browser
        .visible
        .iter()
        .position(|&i| i == pipeline_arena)
        .unwrap();
    s.browser.selected = pipeline_vis;
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    assert!(
        s.browser.expanded.contains(&pipeline_arena),
        "Enter should expand the PG"
    );
}

#[test]
fn esc_on_expanded_pg_in_tree_collapses_it() {
    // Ascend (Esc) in tree focus on an expanded PG collapses it.
    let (mut s, c) = three_level_browser_state();
    // Pipeline (arena 1) is expanded. Select it.
    let pipeline_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "pipeline")
        .unwrap();
    let pipeline_vis = s
        .browser
        .visible
        .iter()
        .position(|&i| i == pipeline_arena)
        .unwrap();
    s.browser.selected = pipeline_vis;
    assert!(s.browser.expanded.contains(&pipeline_arena));
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert!(
        !s.browser.expanded.contains(&pipeline_arena),
        "Esc (Ascend) on expanded PG should collapse it"
    );
}

#[test]
fn tree_nav_uses_arrows_only_no_jk() {
    let (mut s, c) = seeded_browser_state();

    // j is dropped — firing it leaves selection unchanged.
    let before = s.browser.selected;
    update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.selected, before,
        "j should no longer move the cursor"
    );

    // Down still works.
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    assert!(s.browser.selected > before, "Down should move the cursor");

    // k is dropped — firing it leaves selection unchanged.
    let before = s.browser.selected;
    update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.selected, before,
        "k should no longer move the cursor"
    );

    // Up still works.
    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    assert!(s.browser.selected < before, "Up should move the cursor");
}

// -----------------------------------------------------------------------
// Helpers for Task 11-14 tests
// -----------------------------------------------------------------------

/// AppState with current_tab = Browser, selection on the "gen" Processor
/// (visible row 1 in seeded_browser_state).
fn fresh_browser_on_processor() -> (AppState, crate::config::Config) {
    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 1; // "gen" Processor
    (s, c)
}

// -----------------------------------------------------------------------
// Task 11: Focus cycle — Enter=descend, Right/Left=section, Esc=ascend
// -----------------------------------------------------------------------

#[test]
fn enter_on_processor_enters_detail_focus_at_section_zero() {
    // Enter (Descend) on a leaf Processor enters detail focus at section 0.
    let (mut s, c) = fresh_browser_on_processor();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    match &s.browser.detail_focus {
        crate::view::browser::state::DetailFocus::Section { idx, .. } => {
            assert_eq!(*idx, 0)
        }
        crate::view::browser::state::DetailFocus::Tree => {
            panic!("expected Section focus, got Tree")
        }
    }
}

#[test]
fn right_is_noop_in_section_focus() {
    // Right is unmapped in section focus — use NextPane (Tab) to cycle sections.
    let (mut s, c) = fresh_browser_on_processor();
    // Enter detail focus (Section{0}), then press Right twice — idx must stay 0.
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
    match &s.browser.detail_focus {
        crate::view::browser::state::DetailFocus::Section { idx, .. } => {
            assert_eq!(*idx, 0, "Right must be a no-op in section focus")
        }
        _ => panic!("expected Section focus"),
    }
}

#[test]
fn esc_returns_to_tree_focus_from_detail() {
    // Esc (Ascend) in Section focus returns to Tree focus.
    let (mut s, c) = fresh_browser_on_processor();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.detail_focus,
        crate::view::browser::state::DetailFocus::Tree
    );
}

#[test]
fn moving_tree_selection_resets_detail_focus() {
    // Moving the tree cursor while in detail focus resets focus to Tree.
    let (mut s, c) = fresh_browser_on_processor();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    assert!(matches!(
        s.browser.detail_focus,
        crate::view::browser::state::DetailFocus::Section { .. }
    ));
    // Return to tree focus (Esc), then move down.
    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.detail_focus,
        crate::view::browser::state::DetailFocus::Tree
    );
}

#[test]
fn enter_on_pg_does_not_enter_section_focus() {
    // Enter (Descend) on a ProcessGroup expands it — it does NOT enter
    // detail section focus.
    let (mut s, c) = seeded_browser_state();
    // Confirm we're on a PG (root, selected=0), and it's already expanded.
    // Collapse it first so Enter will expand.
    s.browser.expanded.remove(&0);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = 0;
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    // Must still be in Tree focus — not Section focus.
    assert_eq!(
        s.browser.detail_focus,
        crate::view::browser::state::DetailFocus::Tree,
        "Enter on PG should expand, not enter section focus"
    );
    // And the PG should now be expanded.
    assert!(
        s.browser.expanded.contains(&0),
        "PG should be expanded after Enter"
    );
}

// -----------------------------------------------------------------------
// Task 12: Arrow-key row nav inside focused sections
// -----------------------------------------------------------------------

/// AppState with selection on the "gen" Processor (arena 1, visible 1)
/// and a NodeDetail::Processor seeded with 3 properties.
fn fresh_browser_on_processor_with_properties() -> (AppState, crate::config::Config) {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::NodeDetail;

    let (mut s, c) = seeded_browser_state();
    s.browser.selected = 1;
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![
                ("alpha".into(), "one".into()),
                ("beta".into(), "two".into()),
                ("gamma".into(), "three".into()),
            ],
            validation_errors: vec![],
        }),
    );
    (s, c)
}

#[test]
fn down_inside_focused_properties_advances_row() {
    let (mut s, c) = fresh_browser_on_processor_with_properties();
    // Enter detail focus on Properties (section 0) via Descend (Enter).
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    match &s.browser.detail_focus {
        crate::view::browser::state::DetailFocus::Section { idx, rows, .. } => {
            assert_eq!(*idx, 0);
            assert_eq!(rows[0], 1);
        }
        _ => panic!("expected Section focus"),
    }
}

#[test]
fn up_inside_focused_properties_clamps_at_zero() {
    let (mut s, c) = fresh_browser_on_processor_with_properties();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    match &s.browser.detail_focus {
        crate::view::browser::state::DetailFocus::Section { rows, .. } => {
            assert_eq!(rows[0], 0, "clamped at 0")
        }
        _ => panic!("expected Section focus"),
    }
}

#[test]
fn down_inside_focused_properties_clamps_at_max() {
    use crate::view::browser::state::DetailSection;

    let (mut s, c) = fresh_browser_on_processor_with_properties();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    for _ in 0..100 {
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    }
    match &s.browser.detail_focus {
        crate::view::browser::state::DetailFocus::Section { rows, .. } => {
            let max = s
                .browser
                .section_len(DetailSection::Properties, &s.bulletins.ring);
            assert_eq!(rows[0], max.saturating_sub(1), "clamped at max-1");
        }
        _ => panic!("expected Section focus"),
    }
}

// -----------------------------------------------------------------------
// Task 13: c copy in focused sections
// -----------------------------------------------------------------------

#[test]
fn c_in_focused_properties_copies_value_and_emits_banner() {
    let (mut s, c) = fresh_browser_on_processor_with_properties();
    // Enter detail focus on Properties (section 0) via Descend (Enter).
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);

    // The banner must start with "copied" (success) or "clipboard" (failure).
    // We can't assert the clipboard was set in a headless test, but we can
    // assert the reducer produced an Info or Warning banner.
    let banner = s.status.banner.as_ref().expect("banner set after c");
    assert!(
        banner.message.starts_with("copied") || banner.message.starts_with("clipboard"),
        "banner = {}",
        banner.message
    );
}

/// Regression test for the arboard X11 `Drop` teardown bug that
/// corrupted the ratatui alt-screen grid on every `c` keypress.
/// Verifies that `AppState::clipboard` starts as `None` (lazy
/// init), the reducer sets a banner on `c`, and the second `c`
/// press does not panic — the handle is reused if the first
/// succeeded, and re-attempted if the first failed (e.g. headless
/// CI with no X display).
#[test]
fn c_lazily_initializes_and_reuses_persistent_clipboard_handle() {
    let (mut s, c) = seeded_browser_state();

    // Before any c press, the clipboard handle is None.
    assert!(
        s.clipboard.is_none(),
        "clipboard should be lazily initialized, not eager",
    );

    // First c press: a banner should appear (Info on success or
    // Warning on failure). In headless CI with no X display the
    // arboard call may fail — that's fine, we're testing lazy
    // init and banner emission, not clipboard success.
    update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_some(),
        "banner should be set after first c press",
    );

    // Second c press: the reducer reuses the handle if the first
    // succeeded, retries lazy init if the first failed. Either
    // path must not panic and must still produce a banner.
    update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);
    assert!(
        s.status.banner.is_some(),
        "banner should still be set after second c press",
    );
}

// -----------------------------------------------------------------------
// Task 14: t cross-link on focused bulletin rows
// -----------------------------------------------------------------------

/// AppState with selection on "gen" Processor, a populated NodeDetail, and
/// one matching bulletin in the ring.
fn fresh_browser_on_processor_with_bulletins() -> (AppState, crate::config::Config) {
    use crate::client::BulletinSnapshot;

    let (mut s, c) = fresh_browser_on_processor_with_properties();
    s.bulletins.ring.push_back(BulletinSnapshot {
        id: 1,
        message: "test bulletin".into(),
        source_id: "gen".into(),
        source_name: "Gen".into(),
        group_id: "root".into(),
        source_type: "PROCESSOR".into(),
        level: "WARNING".into(),
        timestamp_iso: String::new(),
        timestamp_human: "00:00:00".into(),
    });
    (s, c)
}

#[test]
fn t_is_noop_in_focused_recent_bulletins() {
    // `t` is retired; it is now a no-op in both tree and detail focus.
    // Users use `g e` (GoTarget::Events) for cross-tab gotos instead.
    let (mut s, c) = fresh_browser_on_processor_with_bulletins();
    // Enter detail focus on Properties (section 0), then cycle to
    // RecentBulletins (section 1) via Right.
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);

    let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
    assert!(
        r.intent.is_none(),
        "t must be a no-op in detail focus; got {r:?}"
    );
}

#[test]
fn t_is_noop_in_focused_properties_section() {
    // `t` is retired; no intent emitted from Properties section focus either.
    let (mut s, c) = fresh_browser_on_processor_with_properties();
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
    assert!(
        r.intent.is_none(),
        "t must be a no-op in section focus; got {r:?}"
    );
}

#[test]
fn tree_drill_uses_enter_or_right_only_no_l_alias() {
    // Use three_level_browser_state: root(0) expanded, pipeline(1) expanded,
    // gen(2). Collapse pipeline so Enter on it will expand and change visible.
    let (mut s, c) = three_level_browser_state();
    s.browser.expanded.remove(&1);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    // Visible is now [0, 1] (root expanded, pipeline collapsed).
    s.browser.selected = 1; // pipeline (collapsed PG)
    let before_visible = s.browser.visible.clone();

    // `l` no longer drills in.
    update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.visible, before_visible,
        "l should no longer drill into a PG"
    );

    // Enter still drills in (expands pipeline, adds gen to visible).
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    assert_ne!(
        s.browser.visible, before_visible,
        "Enter should still drill in"
    );
}

#[test]
fn t_on_focused_pg_recent_bulletins_emits_crosslink_for_row_source() {
    use crate::client::{BulletinSnapshot, ProcessGroupDetail};
    use crate::view::browser::state::{DetailFocus, NodeDetail};

    let (mut s, c) = seeded_browser_state();
    // Put the tree cursor on `root` (arena idx 0).
    s.browser.selected = s
        .browser
        .visible
        .iter()
        .position(|&i| i == 0)
        .expect("root visible");
    // Inject a PG detail for root so focused_row_source_id resolves.
    s.browser.details.insert(
        0,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: "root".into(),
            name: "root".into(),
            parent_group_id: None,
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "".into(),
            controller_services: vec![],
        }),
    );
    // Focus PG's RecentBulletins section (idx 2 per for_node(PG)).
    s.browser.detail_focus = DetailFocus::Section {
        idx: 2,
        rows: [0, 0, 0, 0, 0],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    // Ring: newest at the back. Newest-first iteration → row 0 = p2.
    s.bulletins.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "WARN".into(),
        message: "old".into(),
        source_id: "p1".into(),
        source_name: "p1".into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: "".into(),
        timestamp_human: "".into(),
    });
    s.bulletins.ring.push_back(BulletinSnapshot {
        id: 2,
        level: "WARN".into(),
        message: "new".into(),
        source_id: "p2".into(),
        source_name: "p2".into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: "".into(),
        timestamp_human: "".into(),
    });

    let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
    // `t` is now a no-op; cross-tab goto is via `g e`.
    assert!(r.intent.is_none(), "t must be a no-op; got {r:?}");
}

#[test]
fn tree_drill_out_uses_left_only_no_h_alias() {
    // `h` and Backspace are retired; only Left (Ascend) collapses a PG.
    let (mut s, c) = three_level_browser_state();
    let pipeline_arena = s
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "pipeline")
        .unwrap();
    let pipeline_visible = s
        .browser
        .visible
        .iter()
        .position(|&i| i == pipeline_arena)
        .unwrap();
    s.browser.selected = pipeline_visible;
    let before_visible = s.browser.visible.clone();

    // `h` is a no-op.
    update(&mut s, key(KeyCode::Char('h'), KeyModifiers::NONE), &c);
    assert_eq!(
        s.browser.visible, before_visible,
        "h must no longer collapse a PG"
    );

    // Left collapses the expanded pipeline PG.
    update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
    assert_ne!(
        s.browser.visible, before_visible,
        "Left should collapse the PG"
    );
}

#[test]
fn enter_on_focused_pg_child_groups_drills_in() {
    use crate::client::ProcessGroupDetail;
    use crate::view::browser::state::{DetailFocus, NodeDetail};

    let (mut s, c) = seeded_browser_state();
    // Put the tree cursor on `root` (arena idx 0).
    s.browser.selected = s
        .browser
        .visible
        .iter()
        .position(|&i| i == 0)
        .expect("root visible");
    // Inject a PG detail for root so the handler can read `d.id`.
    s.browser.details.insert(
        0,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: "root".into(),
            name: "root".into(),
            parent_group_id: None,
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "".into(),
            controller_services: vec![],
        }),
    );
    // Focus PG's ChildGroups section, row 0 → `ingest`.
    s.browser.detail_focus = DetailFocus::Section {
        idx: 1,
        rows: [0, 0, 0, 0, 0],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
    assert!(r.redraw);
    // Tree cursor now on ingest (arena idx 2), not gen (arena idx 1).
    assert_eq!(s.browser.visible[s.browser.selected], 2);
    assert_eq!(s.browser.detail_focus, DetailFocus::Tree);
}

// -----------------------------------------------------------------------
// Task 14: New typed-verb / typed-focus tests
// -----------------------------------------------------------------------

/// Seed a browser with a single Processor (gen, arena 1) and set
/// `current_tab = Browser`. Mirrors `seeded_browser_state` but exposes
/// the state at arena index 1 as a Processor.
fn seed_browser_with_processor(s: &mut AppState) {
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use std::time::SystemTime;

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
        ],
        fetched_at: SystemTime::now(),
    };
    let _ = c; // config not required now that we bypass `update(...)`.
    crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
    s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));
    s.current_tab = ViewId::Browser;
    s.browser.selected = 1; // "gen" Processor
}

/// Seed a browser with a ProcessGroup child. The root (arena 0) has
/// one child PG named "ingest" (arena 1).
fn seed_browser_with_child_pg(s: &mut AppState) {
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use std::time::SystemTime;

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
    let _ = c; // config not required now that we bypass `update(...)`.
    crate::view::browser::state::apply_tree_snapshot(&mut s.browser, snap);
    s.flow_index = Some(crate::view::browser::state::build_flow_index(&s.browser));
    s.current_tab = ViewId::Browser;
    // Select root (arena 0) — the parent of the child PG.
    s.browser.selected = 0;
}

#[test]
fn p_opens_properties_modal() {
    use crate::client::ProcessorDetail;
    use crate::input::{BrowserVerb, ViewVerb};
    use crate::view::browser::state::NodeDetail;

    let mut s = fresh_state();
    s.current_tab = ViewId::Browser;
    seed_browser_with_processor(&mut s);
    // Seed detail so the OpenProperties path has data.
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![],
            validation_errors: vec![],
        }),
    );

    BrowserHandler::handle_verb(&mut s, ViewVerb::Browser(BrowserVerb::OpenProperties));
    assert!(matches!(s.modal, Some(Modal::Properties(_))));
}

#[test]
fn e_no_longer_opens_properties() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Browser;
    seed_browser_with_processor(&mut s);
    update(
        &mut s,
        AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('e'),
            KeyModifiers::NONE,
        ))),
        &c,
    );
    assert!(
        !matches!(s.modal, Some(Modal::Properties(_))),
        "e must no longer open Properties; p does"
    );
}

#[test]
fn descend_on_process_group_expands() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Browser;
    seed_browser_with_child_pg(&mut s);
    // Root (arena 0) is already expanded by seed; collapse it first.
    s.browser.expanded.remove(&0);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = 0;
    let before = s.browser.expanded.len();
    BrowserHandler::handle_focus(&mut s, FocusAction::Descend);
    assert!(
        s.browser.expanded.len() > before,
        "Descend on PG must expand it"
    );
}

#[test]
fn breadcrumb_mode_no_longer_triggered_by_b() {
    use crossterm::event::{KeyEvent, KeyModifiers};
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Browser;
    seed_browser_with_processor(&mut s);
    let before_selected = s.browser.selected;
    update(
        &mut s,
        AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
            KeyCode::Char('b'),
            KeyModifiers::NONE,
        ))),
        &c,
    );
    // Interactive breadcrumb mode has been removed; `b` must be a no-op.
    assert_eq!(s.browser.selected, before_selected, "b must be a no-op");
}

// -----------------------------------------------------------------------
// Task 7: NextPane/PrevPane cycle Tree → Section{0..n} → Tree
// -----------------------------------------------------------------------

#[test]
fn next_pane_in_tree_enters_first_section_for_processor() {
    use crate::input::FocusAction;
    use crate::view::browser::state::DetailFocus;
    let (mut s, _c) = fresh_browser_on_processor();
    // Selection is on the "gen" Processor which has sections.
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
    assert!(r.is_some(), "NextPane in Tree focus should return Some");
    match &s.browser.detail_focus {
        DetailFocus::Section { idx, .. } => {
            assert_eq!(*idx, 0, "NextPane from Tree should enter Section{{0}}")
        }
        DetailFocus::Tree => panic!("expected Section focus, got Tree"),
    }
}

#[test]
fn prev_pane_in_tree_enters_last_section_for_processor() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, DetailSections};
    let (mut s, _c) = fresh_browser_on_processor();
    let arena_idx = s.browser.visible[s.browser.selected];
    let kind = s.browser.nodes[arena_idx].kind;
    let section_count = DetailSections::for_node(kind).len();
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
    assert!(r.is_some(), "PrevPane in Tree focus should return Some");
    match &s.browser.detail_focus {
        DetailFocus::Section { idx, .. } => {
            assert_eq!(
                *idx,
                section_count - 1,
                "PrevPane from Tree should enter Section{{last}}"
            )
        }
        DetailFocus::Tree => panic!("expected Section focus, got Tree"),
    }
}

#[test]
fn next_pane_in_tree_enters_first_section_for_pg() {
    use crate::input::FocusAction;
    use crate::view::browser::state::DetailFocus;
    let (mut s, _c) = seeded_browser_state();
    // seeded_browser_state puts the root PG at selected=0 (arena 0).
    // ProcessGroup has 3 focusable sections (ControllerServices, ChildGroups, RecentBulletins).
    s.browser.selected = 0;
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
    assert!(r.is_some(), "NextPane on PG should return Some");
    match &s.browser.detail_focus {
        DetailFocus::Section { idx, .. } => {
            assert_eq!(
                *idx, 0,
                "NextPane from Tree on PG should enter Section{{0}}"
            )
        }
        DetailFocus::Tree => panic!("expected Section focus, got Tree"),
    }
}

#[test]
fn next_pane_in_section_advances_to_next_section() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
    let (mut s, _c) = fresh_browser_on_processor();
    // Enter section 0 manually.
    s.browser.detail_focus = DetailFocus::Section {
        idx: 0,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
    assert!(r.is_some(), "NextPane in Section focus should return Some");
    match &s.browser.detail_focus {
        DetailFocus::Section { idx, .. } => {
            assert_eq!(
                *idx, 1,
                "NextPane from Section{{0}} should go to Section{{1}}"
            )
        }
        DetailFocus::Tree => panic!("expected Section focus, got Tree"),
    }
}

#[test]
fn next_pane_from_last_section_wraps_to_tree() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, DetailSections, MAX_DETAIL_SECTIONS};
    let (mut s, _c) = fresh_browser_on_processor();
    let arena_idx = s.browser.visible[s.browser.selected];
    let kind = s.browser.nodes[arena_idx].kind;
    let last_idx = DetailSections::for_node(kind).len() - 1;
    s.browser.detail_focus = DetailFocus::Section {
        idx: last_idx,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
    assert!(r.is_some(), "NextPane from last section should return Some");
    assert_eq!(
        s.browser.detail_focus,
        DetailFocus::Tree,
        "NextPane from last section must wrap back to Tree"
    );
}

#[test]
fn prev_pane_from_section_zero_wraps_to_tree() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
    let (mut s, _c) = fresh_browser_on_processor();
    s.browser.detail_focus = DetailFocus::Section {
        idx: 0,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
    assert!(r.is_some(), "PrevPane from Section{{0}} should return Some");
    assert_eq!(
        s.browser.detail_focus,
        DetailFocus::Tree,
        "PrevPane from Section{{0}} must wrap to Tree"
    );
}

#[test]
fn prev_pane_in_section_goes_to_previous_section() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
    let (mut s, _c) = fresh_browser_on_processor();
    s.browser.detail_focus = DetailFocus::Section {
        idx: 1,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
    assert!(r.is_some(), "PrevPane from Section{{1}} should return Some");
    match &s.browser.detail_focus {
        DetailFocus::Section { idx, .. } => {
            assert_eq!(
                *idx, 0,
                "PrevPane from Section{{1}} should go to Section{{0}}"
            )
        }
        DetailFocus::Tree => panic!("expected Section focus, got Tree"),
    }
}

#[test]
fn left_right_scroll_in_section_focus() {
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
    let (mut s, _c) = fresh_browser_on_processor();
    s.browser.detail_focus = DetailFocus::Section {
        idx: 0,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    // Right increments x_offsets[idx].
    let r = BrowserHandler::handle_focus(&mut s, FocusAction::Right);
    assert!(r.is_some(), "Right must return Some in Section focus");
    assert!(
        matches!(
            s.browser.detail_focus,
            DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 1
        ),
        "Right must increment x_offsets[0]"
    );

    // Left decrements back to 0.
    BrowserHandler::handle_focus(&mut s, FocusAction::Left);
    assert!(
        matches!(
            s.browser.detail_focus,
            DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 0
        ),
        "Left must decrement x_offsets[0]"
    );

    // Left at 0 stays at 0 (saturating).
    BrowserHandler::handle_focus(&mut s, FocusAction::Left);
    assert!(
        matches!(
            s.browser.detail_focus,
            DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 0
        ),
        "Left at 0 must not underflow"
    );
}

#[test]
fn descend_on_folder_toggles_expansion() {
    use crate::client::{FolderKind, NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use crate::view::browser::state::apply_tree_snapshot;

    let mut s = crate::test_support::fresh_state();
    apply_tree_snapshot(
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

    // After apply_tree_snapshot: root is auto-expanded; the CS folder
    // sits under root. Visible = [root, Folder(CS)]. Selected = 0.
    // Move cursor onto the folder.
    s.browser.selected = 1;
    let arena = s.browser.visible[1];
    assert!(
        matches!(
            s.browser.nodes[arena].kind,
            NodeKind::Folder(FolderKind::ControllerServices)
        ),
        "cursor must land on the CS folder"
    );
    assert!(
        !s.browser.expanded.contains(&arena),
        "folder starts collapsed"
    );

    BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend on folder should be consumed");

    assert!(
        s.browser.expanded.contains(&arena),
        "folder must be expanded after Descend"
    );

    // Second Descend collapses.
    BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend on folder should be consumed again");
    assert!(
        !s.browser.expanded.contains(&arena),
        "folder must be collapsed after second Descend"
    );
}

#[test]
fn descend_on_referencing_component_emits_goto() {
    use crate::app::state::PendingIntent;
    use crate::client::{ControllerServiceDetail, ReferencingComponent, ReferencingKind};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail,
    };

    let mut s = crate::test_support::fresh_state();
    // Minimal browser arena with one CS node at index 1 under a root PG at 0.
    s.browser.nodes.clear();
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: None,
        children: vec![1],
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
        parameter_context_ref: None,
    });
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: "cs".into(),
        group_id: "root".into(),
        name: "pool".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });
    s.browser.expanded.insert(0);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = s.browser.visible.iter().position(|&i| i == 1).unwrap();

    let cs = ControllerServiceDetail {
        id: "cs".into(),
        name: "pool".into(),
        type_name: "T".into(),
        bundle: "".into(),
        state: "ENABLED".into(),
        parent_group_id: Some("root".into()),
        properties: vec![],
        validation_errors: vec![],
        bulletin_level: "INFO".into(),
        comments: "".into(),
        restricted: false,
        deprecated: false,
        persists_state: false,
        referencing_components: vec![ReferencingComponent {
            id: "proc-x".into(),
            name: "p".into(),
            kind: ReferencingKind::Processor,
            state: "RUNNING".into(),
            active_thread_count: 0,
            group_id: "child-group".into(),
        }],
    };
    s.browser
        .details
        .insert(1, NodeDetail::ControllerService(cs));

    // Focus the Referencing components section, first row.
    let sections = DetailSections::for_node(NodeKind::ControllerService);
    let ref_idx = sections
        .0
        .iter()
        .position(|sec| *sec == DetailSection::ReferencingComponents)
        .expect("CS sections must include ReferencingComponents");
    let mut rows = [0usize; MAX_DETAIL_SECTIONS];
    rows[ref_idx] = 0;
    s.browser.detail_focus = DetailFocus::Section {
        idx: ref_idx,
        rows,
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let r = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("descend must produce a result");

    // Intent must be a Goto(OpenInBrowser) carrying the referenced
    // processor's id and its group id.
    match r.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, "proc-x");
            assert_eq!(group_id, "child-group");
        }
        other => {
            panic!("Descend on ref-components row must emit Goto(OpenInBrowser), got {other:?}")
        }
    }
}

#[test]
fn descend_on_pg_controller_service_emits_goto() {
    use crate::app::state::PendingIntent;
    use crate::client::{ControllerServiceSummary, ProcessGroupDetail};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail,
    };

    let mut s = crate::test_support::fresh_state();
    // Minimal arena: root PG at 0, selected PG "ingest" at 1, owned CS at 2.
    s.browser.nodes.clear();
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: None,
        children: vec![1],
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
        parameter_context_ref: None,
    });
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: Some(0),
        children: vec![2],
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
        parameter_context_ref: None,
    });
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: Some(1),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: "cs1".into(),
        group_id: "ingest".into(),
        name: "pool".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });
    s.browser.expanded.insert(0);
    s.browser.expanded.insert(1);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = s.browser.visible.iter().position(|&i| i == 1).unwrap();

    let pg = ProcessGroupDetail {
        id: "ingest".into(),
        name: "ingest".into(),
        parent_group_id: Some("root".into()),
        running: 0,
        stopped: 0,
        invalid: 0,
        disabled: 0,
        active_threads: 0,
        flow_files_queued: 0,
        bytes_queued: 0,
        queued_display: "0".into(),
        controller_services: vec![ControllerServiceSummary {
            id: "cs1".into(),
            name: "pool".into(),
            type_short: "T".into(),
            state: "ENABLED".into(),
        }],
    };
    s.browser.details.insert(1, NodeDetail::ProcessGroup(pg));

    let sections = DetailSections::for_node(NodeKind::ProcessGroup);
    let cs_idx = sections
        .0
        .iter()
        .position(|sec| *sec == DetailSection::ControllerServices)
        .expect("PG sections must include ControllerServices");
    let mut rows = [0usize; MAX_DETAIL_SECTIONS];
    rows[cs_idx] = 0;
    s.browser.detail_focus = DetailFocus::Section {
        idx: cs_idx,
        rows,
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let r = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("descend must produce a result");

    match r.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, "cs1");
            assert_eq!(group_id, "ingest");
        }
        other => panic!(
            "Descend on PG controller-services row must emit Goto(OpenInBrowser), got {other:?}"
        ),
    }
}

#[test]
fn properties_hotkey_opens_modal_on_cs_tree_row() {
    use crate::app::state::ViewId;

    let mut s = crate::test_support::fresh_state();
    s.current_tab = ViewId::Browser;

    // Minimal browser arena with one CS node at index 1 under a root PG at 0.
    s.browser.nodes.clear();
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: None,
        children: vec![1],
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
        parameter_context_ref: None,
    });
    s.browser.nodes.push(crate::view::browser::state::TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: "cs".into(),
        group_id: "root".into(),
        name: "pool".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });
    s.browser.expanded.insert(0);
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    s.browser.selected = s.browser.visible.iter().position(|&i| i == 1).unwrap();

    assert!(
        s.browser_selection_has_properties(),
        "p (OpenProperties) must be enabled on a CS tree row"
    );
}

/// Build a minimal browser state with a single Connection node at arena
/// index 1 (child of root at 0) and a `ConnectionDetail` inserted, with
/// `detail_focus` set to the Endpoints section (idx 0).
fn browser_state_on_connection_endpoints(row: usize) -> (AppState, crate::config::Config) {
    use crate::client::{ConnectionDetail, NodeKind, NodeStatusSummary};
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail, TreeNode,
        rebuild_visible,
    };

    let mut s = crate::test_support::fresh_state();
    s.current_tab = ViewId::Browser;

    s.browser.nodes.clear();
    s.browser.nodes.push(TreeNode {
        parent: None,
        children: vec![1],
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
        parameter_context_ref: None,
    });
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Connection,
        id: "conn".into(),
        group_id: "root".into(),
        name: "src→dst".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: "0".into(),
            source_id: "src-proc".into(),
            source_name: "Source".into(),
            destination_id: "dst-proc".into(),
            destination_name: "Dest".into(),
        },
        parameter_context_ref: None,
    });
    s.browser.expanded.insert(0);
    rebuild_visible(&mut s.browser);
    s.browser.selected = s.browser.visible.iter().position(|&i| i == 1).unwrap();

    s.browser.details.insert(
        1,
        NodeDetail::Connection(ConnectionDetail {
            id: "conn".into(),
            name: "src→dst".into(),
            source_id: "src-proc".into(),
            source_name: "Source".into(),
            source_type: "PROCESSOR".into(),
            source_group_id: "root".into(),
            destination_id: "dst-proc".into(),
            destination_name: "Dest".into(),
            destination_type: "PROCESSOR".into(),
            destination_group_id: "root".into(),
            selected_relationships: vec!["success".into()],
            available_relationships: vec!["success".into()],
            back_pressure_object_threshold: 0,
            back_pressure_data_size_threshold: "".into(),
            flow_file_expiration: "".into(),
            load_balance_strategy: "".into(),
            fill_percent: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0".into(),
        }),
    );

    // Focus the Endpoints section (idx 0) with the requested row cursor.
    let sections = DetailSections::for_node(NodeKind::Connection);
    let ep_idx = sections
        .0
        .iter()
        .position(|sec| *sec == DetailSection::Endpoints)
        .expect("Connection sections must include Endpoints");
    let mut rows = [0usize; MAX_DETAIL_SECTIONS];
    rows[ep_idx] = row;
    s.browser.detail_focus = DetailFocus::Section {
        idx: ep_idx,
        rows,
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    (s, tiny_config())
}

#[test]
fn descend_on_focused_endpoints_row_zero_emits_open_in_browser_for_source() {
    use crate::app::state::PendingIntent;
    use crate::intent::CrossLink;

    let (mut s, _c) = browser_state_on_connection_endpoints(0);
    let r = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend on Endpoints must produce a result");
    match r.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, "src-proc");
            assert_eq!(group_id, "root");
        }
        other => panic!("expected Goto(OpenInBrowser {{ src-proc }}), got {other:?}"),
    }
}

#[test]
fn descend_on_focused_endpoints_row_one_emits_open_in_browser_for_destination() {
    use crate::app::state::PendingIntent;
    use crate::intent::CrossLink;

    let (mut s, _c) = browser_state_on_connection_endpoints(1);
    let r = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend on Endpoints must produce a result");
    match r.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, "dst-proc");
            assert_eq!(group_id, "root");
        }
        other => panic!("expected Goto(OpenInBrowser {{ dst-proc }}), got {other:?}"),
    }
}

#[test]
fn descend_on_focused_properties_row_with_resolvable_uuid_emits_goto() {
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail, TreeNode,
    };

    let (mut s, _c) = fresh_browser_on_processor_with_properties();
    let cs_uuid = "a1b2c3d4-e5f6-7890-abcd-ef1234567890";

    // Add a CS node to the arena so the UUID resolves.
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: cs_uuid.into(),
        group_id: "root".into(),
        name: "http-pool".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });

    // Rewrite the selected processor's detail to have our one property.
    if let Some(NodeDetail::Processor(p)) = s.browser.details.get_mut(&1) {
        p.properties = vec![("Record Reader".into(), cs_uuid.into())];
    }

    // Focus Properties section row 0. Resolve Properties' index so we
    // don't depend on it being 0 forever.
    let sections = DetailSections::for_node_detail(NodeKind::Processor, false);
    let props_idx = sections
        .0
        .iter()
        .position(|x| *x == DetailSection::Properties)
        .expect("Processor has Properties section");
    s.browser.detail_focus = DetailFocus::Section {
        idx: props_idx,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    use crate::app::state::PendingIntent;
    match result.intent {
        Some(PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, cs_uuid);
            assert_eq!(group_id, "root");
        }
        other => panic!("expected Goto(OpenInBrowser), got {other:?}"),
    }
}

#[test]
fn descend_on_focused_properties_row_with_non_resolvable_value_is_noop() {
    use crate::client::NodeKind;
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail,
    };

    let (mut s, _c) = fresh_browser_on_processor_with_properties();
    if let Some(NodeDetail::Processor(p)) = s.browser.details.get_mut(&1) {
        p.properties = vec![("Batch Size".into(), "500".into())];
    }

    let sections = DetailSections::for_node_detail(NodeKind::Processor, false);
    let props_idx = sections
        .0
        .iter()
        .position(|x| *x == DetailSection::Properties)
        .expect("Processor has Properties section");
    s.browser.detail_focus = DetailFocus::Section {
        idx: props_idx,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let result = super::BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    assert!(
        result.intent.is_none(),
        "Descend on non-resolvable property row must be a silent no-op, got {:?}",
        result.intent
    );
}

#[test]
fn descend_on_focused_connections_row_emits_open_in_browser_for_opposite() {
    use crate::client::{NodeKind, NodeStatusSummary, ProcessorDetail};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{
        DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail, TreeNode,
    };

    let (mut s, _c) = fresh_browser_on_processor();
    // Add an outbound connection from the "gen" processor (arena 1).
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Connection,
        id: "c-out".into(),
        group_id: "root".into(),
        name: "gen→sink".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: "0".into(),
            source_id: "gen".into(),
            source_name: "Gen".into(),
            destination_id: "sink".into(),
            destination_name: "Sink".into(),
        },
        parameter_context_ref: None,
    });
    // And a Processor node for the "sink" so the group_id resolves.
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Processor,
        id: "sink".into(),
        group_id: "sink-pg".into(),
        name: "Sink".into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });
    crate::view::browser::state::rebuild_visible(&mut s.browser);
    // Make sure we're still on gen (arena 1).
    s.browser.selected = s
        .browser
        .visible
        .iter()
        .position(|&i| i == 1)
        .expect("gen visible");

    // Seed a minimal ProcessorDetail.
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "".into(),
            bundle: "".into(),
            run_status: "RUNNING".into(),
            scheduling_strategy: "".into(),
            scheduling_period: "".into(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: "".into(),
            yield_duration: "".into(),
            bulletin_level: "".into(),
            properties: vec![],
            validation_errors: vec![],
        }),
    );

    // Focus the Connections section. Resolve its index so we don't
    // depend on the exact section order.
    let sections = DetailSections::for_node_detail(NodeKind::Processor, false);
    let conn_idx = sections
        .0
        .iter()
        .position(|x| *x == DetailSection::Connections)
        .expect("Processor has Connections section");
    s.browser.detail_focus = DetailFocus::Section {
        idx: conn_idx,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    match result.intent {
        Some(crate::app::state::PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            group_id,
        })) => {
            assert_eq!(component_id, "sink");
            assert_eq!(group_id, "sink-pg");
        }
        other => panic!("expected Goto(OpenInBrowser {{ sink }}), got {other:?}"),
    }
}

#[test]
fn properties_modal_down_arrow_advances_selection() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::{NodeDetail, PropertiesModalState};
    let (mut s, c) = seeded_browser_state();
    // Seed processor detail with 3 properties.
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![
                ("K1".into(), "v1".into()),
                ("K2".into(), "v2".into()),
                ("K3".into(), "v3".into()),
            ],
            validation_errors: vec![],
        }),
    );
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));

    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 1);

    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c); // past end — must clamp
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 2);

    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 1);
}

#[test]
fn properties_modal_page_and_home_end() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::{NodeDetail, PropertiesModalState};
    let (mut s, c) = seeded_browser_state();
    // Seed processor with 20 properties so PageDown actually moves a page.
    let props: Vec<(String, String)> = (0..20)
        .map(|i| (format!("K{i}"), format!("v{i}")))
        .collect();
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: props,
            validation_errors: vec![],
        }),
    );
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));

    update(&mut s, key(KeyCode::End, KeyModifiers::NONE), &c);
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 19);

    update(&mut s, key(KeyCode::Home, KeyModifiers::NONE), &c);
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 0);

    update(&mut s, key(KeyCode::PageDown, KeyModifiers::NONE), &c);
    let Some(Modal::Properties(ps)) = &s.modal else {
        panic!()
    };
    assert_eq!(ps.selected, 10);
}

#[test]
fn properties_modal_c_copies_selected_value() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::{NodeDetail, PropertiesModalState};
    let (mut s, c) = seeded_browser_state();
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("K1".into(), "v1".into()), ("K2".into(), "copy-me".into())],
            validation_errors: vec![],
        }),
    );
    let mut ps = PropertiesModalState::new(1);
    ps.selected = 1;
    s.modal = Some(Modal::Properties(ps));

    update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);

    // In CI there is no clipboard daemon — the banner will be a
    // warning ("clipboard: …") rather than the info toast we'd see
    // interactively. Either outcome proves the code path ran against
    // the selected row. What we assert is that the banner is set and
    // non-empty, and that the modal did not close.
    assert!(s.status.banner.is_some(), "`c` must emit a status banner");
    assert!(matches!(s.modal, Some(Modal::Properties(_))));
}

#[test]
fn properties_modal_descend_on_uuid_emits_open_in_browser_and_closes() {
    use crate::client::ProcessorDetail;
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::intent::CrossLink;
    use crate::view::browser::state::NodeDetail;
    use crate::view::browser::state::{PropertiesModalState, TreeNode};

    let (mut s, c) = seeded_browser_state();

    // Seed a CS node at arena index 2 whose id is a real UUID.
    let cs_uuid = "7f3e1c22-1111-4444-8888-abcdef012345".to_string();
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: cs_uuid.clone(),
        group_id: "root".into(),
        name: "fixture-json-reader".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });

    // Processor (arena 1) has a property whose value is that UUID.
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![
                ("Plain".into(), "not-a-uuid".into()),
                ("Record Reader".into(), cs_uuid.clone()),
            ],
            validation_errors: vec![],
        }),
    );
    let mut ps = PropertiesModalState::new(1);
    ps.selected = 1;
    s.modal = Some(Modal::Properties(ps));

    let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    // Modal closed.
    assert!(s.modal.is_none());
    // Intent is Goto(OpenInBrowser { component_id = uuid }).
    match r.intent {
        Some(crate::app::state::PendingIntent::Goto(CrossLink::OpenInBrowser {
            component_id,
            ..
        })) => assert_eq!(component_id, cs_uuid),
        other => panic!("expected Goto(OpenInBrowser {{ cs_uuid }}), got {other:?}"),
    }
}

#[test]
fn properties_modal_descend_on_non_uuid_is_noop() {
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::{NodeDetail, PropertiesModalState};

    let (mut s, c) = seeded_browser_state();
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("K".into(), "plain text".into())],
            validation_errors: vec![],
        }),
    );
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));

    let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    // Modal stays open, no intent.
    assert!(matches!(s.modal, Some(Modal::Properties(_))));
    assert!(r.intent.is_none());
}

// ── T16 parameter-reference cross-link tests ──────────────────────────────

/// Builds a minimal AppState with one root PG that has a bound parameter
/// context and one processor (arena 1) whose group_id is that PG.
/// Returns the AppState, the owning PG id, and the processor's arena index.
fn browser_with_bound_pg_and_processor(prop_value: &str) -> (AppState, String, usize) {
    use crate::client::ProcessorDetail;
    use crate::cluster::snapshot::ParameterContextRef;
    use crate::view::browser::state::NodeDetail;

    let (mut s, _c) = seeded_browser_state();
    let pg_id = "root".to_string();

    // Stamp a parameter context ref onto the root PG (arena 0).
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-1".into(),
        name: "my-context".into(),
    });

    // Rewrite the processor detail (arena 1) to have our test property.
    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("Bootstrap".into(), prop_value.into())],
            validation_errors: vec![],
        }),
    );
    (s, pg_id, 1)
}

/// Focus the Properties section row 0 of the node at `arena_idx`.
fn focus_properties_row_0(s: &mut AppState, arena_idx: usize) {
    use crate::view::browser::state::{DetailFocus, DetailSection, DetailSections};

    let kind = s.browser.nodes[arena_idx].kind;
    let sections = DetailSections::for_node_detail(kind, false);
    let props_idx = sections
        .0
        .iter()
        .position(|x| *x == DetailSection::Properties)
        .expect("node has Properties section");
    s.browser.selected = s
        .browser
        .visible
        .iter()
        .position(|&i| i == arena_idx)
        .expect("arena_idx is visible");
    s.browser.detail_focus = DetailFocus::Section {
        idx: props_idx,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };
}

#[test]
fn t16_single_param_ref_emits_open_parameter_context_modal_with_preselect() {
    // A single `#{kafka_bootstrap}` value on a processor whose owning PG has a
    // bound context should emit `OpenParameterContextModal { pg_id, preselect:
    // Some("kafka_bootstrap") }`.
    let (mut s, pg_id, arena_idx) = browser_with_bound_pg_and_processor("#{kafka_bootstrap}");
    focus_properties_row_0(&mut s, arena_idx);

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    match result.intent {
        Some(PendingIntent::Goto(CrossLink::OpenParameterContextModal {
            pg_id: got_pg,
            preselect,
        })) => {
            assert_eq!(got_pg, pg_id, "pg_id must match the owning PG");
            assert_eq!(
                preselect,
                Some("kafka_bootstrap".to_string()),
                "single ref must preselect the name"
            );
        }
        other => {
            panic!("expected Goto(OpenParameterContextModal {{ preselect: Some }}), got {other:?}")
        }
    }
}

#[test]
fn t16_multiple_param_refs_emit_open_parameter_context_modal_no_preselect() {
    // Multiple refs `#{a}#{b}` → `preselect: None` (user picks in the modal).
    let (mut s, pg_id, arena_idx) = browser_with_bound_pg_and_processor("#{host}:#{port}");
    focus_properties_row_0(&mut s, arena_idx);

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    match result.intent {
        Some(PendingIntent::Goto(CrossLink::OpenParameterContextModal {
            pg_id: got_pg,
            preselect,
        })) => {
            assert_eq!(got_pg, pg_id);
            assert!(
                preselect.is_none(),
                "multiple refs must produce preselect: None"
            );
        }
        other => {
            panic!("expected Goto(OpenParameterContextModal {{ preselect: None }}), got {other:?}")
        }
    }
}

#[test]
fn t16_escaped_param_ref_produces_no_annotation() {
    // `##{literal}` is the escape for a literal `#{literal}` — not a ref.
    // Descend must be a no-op (no intent emitted).
    let (mut s, _pg, arena_idx) = browser_with_bound_pg_and_processor("##{literal}");
    focus_properties_row_0(&mut s, arena_idx);

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    assert!(
        result.intent.is_none(),
        "escaped #{{literal}} must produce no annotation, got {:?}",
        result.intent
    );
}

#[test]
fn t16_no_bound_context_produces_no_annotation() {
    // A value with `#{ref}` but the owning PG has no parameter context —
    // the annotation must NOT fire. Existing UUID logic still applies but
    // `#{ref}` should be silent.
    use crate::client::ProcessorDetail;
    use crate::view::browser::state::NodeDetail;

    let (mut s, _c) = seeded_browser_state();
    // Ensure the root PG (arena 0) has NO parameter context (default).
    assert!(s.browser.nodes[0].parameter_context_ref.is_none());

    s.browser.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "gen".into(),
            name: "Gen".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: "Running".into(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 1,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![("Bootstrap".into(), "#{kafka_bootstrap}".into())],
            validation_errors: vec![],
        }),
    );
    focus_properties_row_0(&mut s, 1);

    let result = BrowserHandler::handle_focus(&mut s, crate::input::FocusAction::Descend)
        .expect("Descend must be handled");

    assert!(
        result.intent.is_none(),
        "no bound context → no annotation, got {:?}",
        result.intent
    );
}

#[test]
fn properties_modal_hint_spans_advertise_new_chords() {
    use crate::view::browser::state::PropertiesModalState;
    let (mut s, _c) = seeded_browser_state();
    s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));
    let spans = crate::app::state::modal_hints_for_test(&s);
    let keys: Vec<&str> = spans.iter().map(|h| h.key.as_ref()).collect();
    assert!(
        keys.iter()
            .any(|k| k.contains('↑') || k.contains('\u{2191}'))
    );
    assert!(keys.contains(&"Enter"));
    assert!(keys.contains(&"c"));
    assert!(keys.contains(&"Esc"));
}

// ── T17 parameter-context modal verb tests ──────────────────────────────────

/// Build an AppState with the parameter-context modal open in Loaded state.
fn state_with_pc_modal_loaded() -> AppState {
    use crate::client::parameter_context::{ParameterContextNode, ParameterEntry};
    use crate::cluster::snapshot::ParameterContextRef;
    use crate::view::browser::state::parameter_context_modal::ParameterContextLoad;

    let (mut s, _c) = seeded_browser_state();
    // Stamp a parameter context ref onto the root PG (arena 0).
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-1".into(),
        name: "my-context".into(),
    });
    // Open the modal in Loading state, then immediately install a chain.
    s.browser
        .open_parameter_context_modal("root".into(), "root".into(), None);
    let chain = vec![ParameterContextNode {
        id: "ctx-1".into(),
        name: "my-context".into(),
        parameters: vec![
            ParameterEntry {
                name: "host".into(),
                value: Some("localhost".into()),
                description: None,
                sensitive: false,
                provided: false,
            },
            ParameterEntry {
                name: "token".into(),
                value: None,
                description: None,
                sensitive: true,
                provided: false,
            },
        ],
        inherited_ids: vec![],
        fetch_error: None,
    }];
    s.browser
        .apply_parameter_context_modal_loaded("root".into(), chain);
    assert!(
        matches!(
            s.browser.parameter_modal.as_ref().map(|m| &m.load),
            Some(ParameterContextLoad::Loaded { .. })
        ),
        "modal must be Loaded"
    );
    s
}

#[test]
fn toggle_by_context_flips_modal_flag() {
    let mut s = state_with_pc_modal_loaded();
    assert!(!s.browser.parameter_modal.as_ref().unwrap().by_context_mode);

    let result = BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::ToggleByContext,
        ),
    );
    assert!(result.is_some());
    assert!(s.browser.parameter_modal.as_ref().unwrap().by_context_mode);

    // Second toggle flips back.
    BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::ToggleByContext,
        ),
    );
    assert!(!s.browser.parameter_modal.as_ref().unwrap().by_context_mode);
}

#[test]
fn close_modal_verb_clears_parameter_modal() {
    let mut s = state_with_pc_modal_loaded();
    assert!(s.browser.parameter_modal.is_some());

    BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::Close,
        ),
    );

    assert!(
        s.browser.parameter_modal.is_none(),
        "Close must clear parameter_modal"
    );
}

#[test]
fn close_modal_verb_cancels_active_search_first() {
    let mut s = state_with_pc_modal_loaded();
    // Open a search session.
    s.browser.parameter_modal_search_open();
    assert!(
        s.browser.parameter_modal.as_ref().unwrap().search.is_some(),
        "search should be open"
    );

    // Pressing Close while search is active should cancel search, not close the modal.
    BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::Close,
        ),
    );

    assert!(
        s.browser.parameter_modal.is_some(),
        "modal must remain open after cancelling search"
    );
    assert!(
        s.browser.parameter_modal.as_ref().unwrap().search.is_none(),
        "search must be cleared"
    );
}

#[test]
fn toggle_shadowed_flips_flag() {
    let mut s = state_with_pc_modal_loaded();
    assert!(!s.browser.parameter_modal.as_ref().unwrap().show_shadowed);

    BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::ToggleShadowed,
        ),
    );
    assert!(s.browser.parameter_modal.as_ref().unwrap().show_shadowed);
}

#[test]
fn toggle_used_by_flips_flag() {
    let mut s = state_with_pc_modal_loaded();
    assert!(!s.browser.parameter_modal.as_ref().unwrap().show_used_by);

    BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::ToggleUsedBy,
        ),
    );
    assert!(s.browser.parameter_modal.as_ref().unwrap().show_used_by);
}

#[test]
fn parameter_context_modal_copy_redacts_sensitive_values() {
    let s = state_with_pc_modal_loaded();
    let text = super::parameter_context_modal_copy_text(&s.browser)
        .expect("loaded modal yields copy text");

    // Non-sensitive row carries its raw value.
    assert!(
        text.contains("host=localhost"),
        "expected non-sensitive row, got: {text}"
    );

    // Sensitive row substitutes the placeholder, never the raw value.
    assert!(
        text.contains("token=(sensitive)"),
        "sensitive row must render as `name=(sensitive)`, got: {text}"
    );

    // Defensive: a future refactor could leak the wire value. The
    // fixture's sensitive entry has value=None, but pin the invariant
    // anyway in case the fixture grows a Some(...) value later.
    assert!(
        !text.contains("token=(sensitive)\nNone") && !text.contains("token=None"),
        "sensitive value must never appear raw, got: {text}"
    );
}

#[test]
fn refresh_verb_sets_modal_to_loading_and_emits_spawn_intent() {
    use crate::cluster::snapshot::ParameterContextRef;
    use crate::view::browser::state::parameter_context_modal::ParameterContextLoad;

    let mut s = state_with_pc_modal_loaded();
    // Ensure the PG has a bound context id.
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-1".into(),
        name: "my-context".into(),
    });

    let result = BrowserHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ParameterContextModal(
            crate::input::ParameterContextModalVerb::Refresh,
        ),
    )
    .expect("Refresh must produce a result");

    assert!(
        matches!(
            s.browser.parameter_modal.as_ref().map(|m| &m.load),
            Some(ParameterContextLoad::Loading)
        ),
        "Refresh must set modal back to Loading"
    );
    assert!(
        matches!(
            result.intent,
            Some(PendingIntent::SpawnParameterContextModalFetch { .. })
        ),
        "Refresh must emit SpawnParameterContextModalFetch, got {:?}",
        result.intent
    );
}

#[test]
fn open_parameter_context_verb_spawns_fetch_intent() {
    use crate::cluster::snapshot::ParameterContextRef;
    use crate::input::{BrowserVerb, ViewVerb};

    let (mut s, _c) = seeded_browser_state();
    // Arena 0 is the root PG. Stamp a bound context ref.
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-bound".into(),
        name: "bound-ctx".into(),
    });
    // Select the root PG.
    s.browser.selected = 0;

    let result =
        BrowserHandler::handle_verb(&mut s, ViewVerb::Browser(BrowserVerb::OpenParameterContext))
            .expect("verb must be handled");

    assert!(
        s.browser.parameter_modal.is_some(),
        "modal must be open after OpenParameterContext"
    );
    assert!(
        matches!(
            result.intent,
            Some(PendingIntent::SpawnParameterContextModalFetch {
                ref bound_context_id,
                ..
            }) if bound_context_id == "ctx-bound"
        ),
        "must spawn fetch with the bound context id, got {:?}",
        result.intent
    );
}

/// Pressing Enter (Descend) on the `ParameterContext` detail section of a PG
/// that has a bound context dispatches `OpenParameterContextModal`.
#[test]
fn descend_on_parameter_context_section_dispatches_cross_link() {
    use crate::client::ProcessGroupDetail;
    use crate::cluster::snapshot::ParameterContextRef;
    use crate::input::FocusAction;
    use crate::view::browser::state::{DetailFocus, NodeDetail};

    let (mut s, _c) = seeded_browser_state();
    // Stamp a binding on the root PG (arena 0).
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-bound".into(),
        name: "bound-ctx".into(),
    });
    s.browser.selected = 0;
    let arena_idx = s.browser.visible[s.browser.selected];

    // Inject a minimal ProcessGroup detail so the Descend handler can read pg.id.
    let pg_id = s.browser.nodes[arena_idx].id.clone();
    s.browser.details.insert(
        arena_idx,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: pg_id.clone(),
            name: "root".into(),
            parent_group_id: None,
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0 / 0 B".into(),
            controller_services: vec![],
        }),
    );

    // Section index 0 = ParameterContext when the PG has a binding.
    s.browser.detail_focus = DetailFocus::Section {
        idx: 0,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let result =
        BrowserHandler::handle_focus(&mut s, FocusAction::Descend).expect("must return Some");
    assert!(
        matches!(
            result.intent,
            Some(PendingIntent::Goto(CrossLink::OpenParameterContextModal {
                ref pg_id,
                preselect: None,
            })) if pg_id == "root"
        ),
        "Descend on ParameterContext section must dispatch OpenParameterContextModal, got {:?}",
        result.intent
    );
}

/// `Enter` in the parameter-context modal shifts focus from Sidebar to Body.
#[test]
fn enter_shifts_focus_from_sidebar_to_body() {
    use crate::input::ParameterContextModalVerb;
    use crate::view::browser::state::parameter_context_modal::ParameterContextPane;

    let mut s = state_with_pc_modal_loaded();
    // Default focused_pane is Sidebar.
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().focused_pane,
        ParameterContextPane::Sidebar,
        "modal must start with Sidebar focus"
    );

    let verb = crate::input::ViewVerb::ParameterContextModal(ParameterContextModalVerb::FocusBody);
    BrowserHandler::handle_verb(&mut s, verb).expect("FocusBody must be handled");

    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().focused_pane,
        ParameterContextPane::Body,
        "FocusBody must shift focus to Body"
    );
}

/// `Esc` when Body is focused unfocuses back to Sidebar (does not close modal).
#[test]
fn esc_unfocuses_body_back_to_sidebar() {
    use crate::input::ParameterContextModalVerb;
    use crate::view::browser::state::parameter_context_modal::ParameterContextPane;

    let mut s = state_with_pc_modal_loaded();
    // Manually set Body focus.
    s.browser.parameter_modal.as_mut().unwrap().focused_pane = ParameterContextPane::Body;

    let verb = crate::input::ViewVerb::ParameterContextModal(ParameterContextModalVerb::Close);
    BrowserHandler::handle_verb(&mut s, verb).expect("Close must be handled");

    // Modal must still be open.
    assert!(
        s.browser.parameter_modal.is_some(),
        "Esc in Body focus must not close the modal"
    );
    // Focus must have returned to Sidebar.
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().focused_pane,
        ParameterContextPane::Sidebar,
        "Esc in Body focus must return to Sidebar"
    );
}

/// Fixture with a 3-context inheritance chain for testing sidebar navigation.
fn state_with_pc_modal_loaded_three_contexts() -> AppState {
    use crate::client::parameter_context::{ParameterContextNode, ParameterEntry};
    use crate::cluster::snapshot::ParameterContextRef;

    let (mut s, _c) = seeded_browser_state();
    s.browser.nodes[0].parameter_context_ref = Some(ParameterContextRef {
        id: "ctx-leaf".into(),
        name: "leaf".into(),
    });
    s.browser
        .open_parameter_context_modal("root".into(), "root".into(), None);

    fn entry(name: &str, value: &str) -> ParameterEntry {
        ParameterEntry {
            name: name.into(),
            value: Some(value.into()),
            description: None,
            sensitive: false,
            provided: false,
        }
    }

    let chain = vec![
        ParameterContextNode {
            id: "ctx-leaf".into(),
            name: "leaf".into(),
            parameters: vec![entry("a", "1")],
            inherited_ids: vec!["ctx-mid".into()],
            fetch_error: None,
        },
        ParameterContextNode {
            id: "ctx-mid".into(),
            name: "mid".into(),
            parameters: vec![entry("b", "2")],
            inherited_ids: vec!["ctx-root".into()],
            fetch_error: None,
        },
        ParameterContextNode {
            id: "ctx-root".into(),
            name: "root".into(),
            parameters: vec![entry("c", "3")],
            inherited_ids: vec![],
            fetch_error: None,
        },
    ];
    s.browser
        .apply_parameter_context_modal_loaded("root".into(), chain);
    s
}

#[test]
fn pcm_chain_row_down_advances_sidebar_index_with_clamp() {
    let mut s = state_with_pc_modal_loaded_three_contexts();
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().sidebar_index,
        0,
        "starts at chain head"
    );

    let down = crate::input::ViewVerb::ParameterContextModal(
        crate::input::ParameterContextModalVerb::RowDown,
    );

    BrowserHandler::handle_verb(&mut s, down);
    assert_eq!(s.browser.parameter_modal.as_ref().unwrap().sidebar_index, 1);

    BrowserHandler::handle_verb(&mut s, down);
    assert_eq!(s.browser.parameter_modal.as_ref().unwrap().sidebar_index, 2);

    // At last index — RowDown clamps.
    BrowserHandler::handle_verb(&mut s, down);
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().sidebar_index,
        2,
        "RowDown at last index clamps"
    );
}

#[test]
fn pcm_chain_row_up_decrements_with_clamp_at_zero() {
    let mut s = state_with_pc_modal_loaded_three_contexts();
    let down = crate::input::ViewVerb::ParameterContextModal(
        crate::input::ParameterContextModalVerb::RowDown,
    );
    BrowserHandler::handle_verb(&mut s, down);
    BrowserHandler::handle_verb(&mut s, down);
    assert_eq!(s.browser.parameter_modal.as_ref().unwrap().sidebar_index, 2);

    let up = crate::input::ViewVerb::ParameterContextModal(
        crate::input::ParameterContextModalVerb::RowUp,
    );
    BrowserHandler::handle_verb(&mut s, up);
    assert_eq!(s.browser.parameter_modal.as_ref().unwrap().sidebar_index, 1);

    BrowserHandler::handle_verb(&mut s, up);
    assert_eq!(s.browser.parameter_modal.as_ref().unwrap().sidebar_index, 0);

    // At index 0 — RowUp clamps via saturating_sub.
    BrowserHandler::handle_verb(&mut s, up);
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().sidebar_index,
        0,
        "RowUp at index 0 clamps"
    );
}

#[test]
fn pcm_chain_page_up_jumps_to_head_page_down_to_tail() {
    let mut s = state_with_pc_modal_loaded_three_contexts();

    let page_down = crate::input::ViewVerb::ParameterContextModal(
        crate::input::ParameterContextModalVerb::PageDown,
    );
    BrowserHandler::handle_verb(&mut s, page_down);
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().sidebar_index,
        2,
        "PageDown jumps to last index"
    );

    let page_up = crate::input::ViewVerb::ParameterContextModal(
        crate::input::ParameterContextModalVerb::PageUp,
    );
    BrowserHandler::handle_verb(&mut s, page_up);
    assert_eq!(
        s.browser.parameter_modal.as_ref().unwrap().sidebar_index,
        0,
        "PageUp jumps to head"
    );
}

// ---------------------------------------------------------------------------
// browser_selection_supports_action_history
// ---------------------------------------------------------------------------

fn make_state_with_selected_kind(kind: NodeKind) -> AppState {
    use crate::view::browser::state::{TreeNode, rebuild_visible};

    let mut s = crate::test_support::fresh_state();
    s.current_tab = ViewId::Browser;

    s.browser.nodes.clear();
    // Root PG at arena index 0.
    s.browser.nodes.push(TreeNode {
        parent: None,
        children: vec![1],
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
        parameter_context_ref: None,
    });
    // Target node at arena index 1, child of root.
    let status_summary = match kind {
        NodeKind::ProcessGroup => NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        NodeKind::Processor => NodeStatusSummary::Processor {
            run_status: "RUNNING".into(),
        },
        NodeKind::Connection => NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: "0".into(),
            source_id: "src".into(),
            source_name: "Source".into(),
            destination_id: "dst".into(),
            destination_name: "Dest".into(),
        },
        NodeKind::ControllerService => NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        NodeKind::InputPort | NodeKind::OutputPort => NodeStatusSummary::Port,
        NodeKind::Folder(_) => NodeStatusSummary::Folder { count: 0 },
    };
    s.browser.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind,
        id: "target".into(),
        group_id: "root".into(),
        name: "target".into(),
        status_summary,
        parameter_context_ref: None,
    });
    s.browser.expanded.insert(0);
    rebuild_visible(&mut s.browser);
    s.browser.selected = s.browser.visible.iter().position(|&i| i == 1).unwrap();
    s
}

#[test]
fn action_history_enabled_for_processor_pg_connection_cs_port() {
    for kind in [
        NodeKind::Processor,
        NodeKind::ProcessGroup,
        NodeKind::Connection,
        NodeKind::ControllerService,
        NodeKind::InputPort,
        NodeKind::OutputPort,
    ] {
        let state = make_state_with_selected_kind(kind);
        assert!(
            state.browser_selection_supports_action_history(),
            "expected true for {kind:?}"
        );
    }
}

#[test]
fn action_history_disabled_for_folder_rows() {
    use crate::client::FolderKind;
    for fk in [FolderKind::Queues, FolderKind::ControllerServices] {
        let state = make_state_with_selected_kind(NodeKind::Folder(fk));
        assert!(
            !state.browser_selection_supports_action_history(),
            "expected false for Folder({fk:?})"
        );
    }
}

// ---------------------------------------------------------------------------
// OpenActionHistory dispatch arm
// ---------------------------------------------------------------------------

fn make_state_with_processor_selected(id: &str, name: &str) -> AppState {
    let mut s = make_state_with_selected_kind(NodeKind::Processor);
    let arena_idx = s.browser.visible[s.browser.selected];
    s.browser.nodes[arena_idx].id = id.to_string();
    s.browser.nodes[arena_idx].name = name.to_string();
    s
}

fn make_state_with_folder_selected() -> AppState {
    use crate::client::FolderKind;
    make_state_with_selected_kind(NodeKind::Folder(FolderKind::Queues))
}

#[test]
fn open_action_history_verb_opens_modal_and_emits_intent() {
    use crate::app::state::PendingIntent;
    use crate::input::{BrowserVerb, ViewVerb};

    let mut state = make_state_with_processor_selected("proc-1", "FetchKafka");
    let result = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::Browser(BrowserVerb::OpenActionHistory),
    )
    .expect("handled");
    assert!(result.redraw);
    let intent = result.intent.expect("intent emitted");
    match intent {
        PendingIntent::SpawnActionHistoryModalFetch { source_id, .. } => {
            assert_eq!(source_id, "proc-1");
        }
        other => panic!("wrong intent: {other:?}"),
    }
    let m = state
        .browser
        .action_history_modal
        .as_ref()
        .expect("modal opened");
    assert_eq!(m.source_id, "proc-1");
    assert_eq!(m.component_label, "FetchKafka");
}

#[test]
fn open_action_history_verb_noop_when_disabled() {
    use crate::input::{BrowserVerb, ViewVerb};
    // Folder rows: enabled() returns false → arm guards a noop.
    let mut state = make_state_with_folder_selected();
    let result = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::Browser(BrowserVerb::OpenActionHistory),
    )
    .expect("handled");
    assert!(state.browser.action_history_modal.is_none());
    assert!(result.intent.is_none());
}

// -----------------------------------------------------------------------
// Task 13: reducer arms for ActionHistoryPage / ActionHistoryError
// -----------------------------------------------------------------------

#[test]
fn apply_action_history_page_appends_actions() {
    use super::super::handle_browser_payload;
    use crate::event::BrowserPayload;
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());

    let mut action = nifi_rust_client::dynamic::types::ActionEntity::default();
    action.id = Some(7);
    let payload = BrowserPayload::ActionHistoryPage {
        source_id: "proc-1".into(),
        offset: 0,
        actions: vec![action],
        total: Some(1),
    };
    handle_browser_payload(&mut state, payload);

    let m = state.browser.action_history_modal.as_ref().unwrap();
    assert_eq!(m.actions.len(), 1);
    assert_eq!(m.total, Some(1));
    assert!(!m.loading);
}

#[test]
fn apply_action_history_page_drops_stale_source_id() {
    use super::super::handle_browser_payload;
    use crate::event::BrowserPayload;
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());

    let mut action = nifi_rust_client::dynamic::types::ActionEntity::default();
    action.id = Some(99);
    let payload = BrowserPayload::ActionHistoryPage {
        source_id: "OTHER".into(),
        offset: 0,
        actions: vec![action],
        total: Some(1),
    };
    handle_browser_payload(&mut state, payload);

    let m = state.browser.action_history_modal.as_ref().unwrap();
    assert!(m.actions.is_empty());
}

#[test]
fn apply_action_history_error_sets_error_field() {
    use super::super::handle_browser_payload;
    use crate::event::BrowserPayload;
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());
    let payload = BrowserPayload::ActionHistoryError {
        source_id: "proc-1".into(),
        err: "boom".into(),
    };
    handle_browser_payload(&mut state, payload);
    let m = state.browser.action_history_modal.as_ref().unwrap();
    assert_eq!(m.error.as_deref(), Some("boom"));
    assert!(!m.loading);
}

#[test]
fn apply_action_history_error_preserves_handle_when_source_id_stale() {
    // Regression test: a stale ActionHistoryError (for a source_id that no
    // longer matches the open modal) MUST NOT clear the handle slot —
    // doing so would orphan the worker for the currently-open modal.
    use super::super::handle_browser_payload;
    use crate::event::BrowserPayload;
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-2".into(), "B".into());
    // Install a pretend handle to simulate the post-spawn state.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let h = tokio::task::spawn_local(async {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                });
                state.browser.action_history_modal_handle = Some(h);
                // Stale error from the previously-open proc-1 modal.
                let payload = BrowserPayload::ActionHistoryError {
                    source_id: "proc-1".into(),
                    err: "stale".into(),
                };
                handle_browser_payload(&mut state, payload);
                let m = state.browser.action_history_modal.as_ref().unwrap();
                // Modal state untouched.
                assert!(m.error.is_none(), "stale error must not mutate the modal");
                // Handle preserved.
                assert!(
                    state.browser.action_history_modal_handle.is_some(),
                    "stale error must not clear the handle for the active modal"
                );
            })
            .await;
    });
}

#[test]
fn action_history_modal_close_clears_modal() {
    use crate::input::{ActionHistoryModalVerb, ViewVerb};
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());
    let _ = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::Close),
    );
    assert!(state.browser.action_history_modal.is_none());
}

#[test]
fn action_history_modal_close_cancels_search_first() {
    use crate::input::{ActionHistoryModalVerb, ViewVerb};
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());
    let modal = state.browser.action_history_modal.as_mut().unwrap();
    modal.search = Some(crate::widget::search::SearchState::default());
    let _ = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::Close),
    );
    // First Close cancels search; modal stays open.
    assert!(state.browser.action_history_modal.is_some());
    assert!(
        state
            .browser
            .action_history_modal
            .as_ref()
            .unwrap()
            .search
            .is_none()
    );
    // Second Close closes the modal.
    let _ = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::Close),
    );
    assert!(state.browser.action_history_modal.is_none());
}

#[test]
fn action_history_modal_refresh_resets_state_and_emits_intent() {
    use crate::input::{ActionHistoryModalVerb, ViewVerb};
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "FetchKafka".into());
    // Pre-populate with one action.
    let mut action = nifi_rust_client::dynamic::types::ActionEntity::default();
    action.id = Some(1);
    state
        .browser
        .action_history_modal
        .as_mut()
        .unwrap()
        .apply_page("proc-1", vec![action], Some(1));
    let result = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::Refresh),
    )
    .unwrap();
    assert!(
        matches!(
            result.intent,
            Some(PendingIntent::SpawnActionHistoryModalFetch { .. })
        ),
        "Refresh must emit SpawnActionHistoryModalFetch intent"
    );
    let m = state.browser.action_history_modal.as_ref().unwrap();
    assert!(m.actions.is_empty(), "actions must be cleared on refresh");
    assert!(m.loading, "loading must be set on refresh");
}

#[test]
fn action_history_modal_toggle_expand_uses_selected_row() {
    use crate::input::{ActionHistoryModalVerb, ViewVerb};
    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());
    // Set selection to row 3.
    state
        .browser
        .action_history_modal
        .as_mut()
        .unwrap()
        .selected = 3;
    let _ = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::ToggleExpand),
    );
    assert_eq!(
        state
            .browser
            .action_history_modal
            .as_ref()
            .unwrap()
            .expanded_index,
        Some(3)
    );
}

#[test]
fn action_history_modal_search_input_routes_chars_to_query() {
    use crate::input::{ActionHistoryModalVerb, ViewVerb};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    let mut state = fresh_state();
    state
        .browser
        .open_action_history_modal("proc-1".into(), "X".into());
    // Open search via the verb dispatch.
    let _ = BrowserHandler::handle_verb(
        &mut state,
        ViewVerb::ActionHistoryModal(ActionHistoryModalVerb::OpenSearch),
    );
    // input_active should be true now.
    assert!(
        BrowserHandler::is_text_input_focused(&state),
        "is_text_input_focused must return true with action-history search active"
    );
    // Push three characters via handle_text_input.
    for ch in ['e', 'r', 'r'] {
        let _ = BrowserHandler::handle_text_input(
            &mut state,
            KeyEvent::new(KeyCode::Char(ch), KeyModifiers::empty()),
        );
    }
    let m = state.browser.action_history_modal.as_ref().unwrap();
    let search = m.search.as_ref().expect("search active");
    assert_eq!(search.query, "err");
    // Backspace.
    let _ = BrowserHandler::handle_text_input(
        &mut state,
        KeyEvent::new(KeyCode::Backspace, KeyModifiers::empty()),
    );
    assert_eq!(
        state
            .browser
            .action_history_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .query,
        "er"
    );
    // Esc cancels search.
    let _ = BrowserHandler::handle_text_input(
        &mut state,
        KeyEvent::new(KeyCode::Esc, KeyModifiers::empty()),
    );
    assert!(
        state
            .browser
            .action_history_modal
            .as_ref()
            .unwrap()
            .search
            .is_none()
    );
}

#[test]
fn apply_sparkline_update_replaces_series_when_selection_matches() {
    use crate::client::history::{Bucket, ComponentKind, StatusHistorySeries};
    let mut state = fresh_state();
    state
        .browser
        .open_sparkline_for_selection(ComponentKind::Processor, "p-1".into());
    let series = StatusHistorySeries {
        buckets: vec![Bucket {
            timestamp: std::time::SystemTime::now(),
            in_count: 7,
            out_count: 6,
            queued_count: None,
            task_time_ns: Some(100),
        }],
        generated_at: std::time::SystemTime::now(),
    };
    let ev = AppEvent::SparklineUpdate {
        kind: ComponentKind::Processor,
        id: "p-1".into(),
        series,
    };
    let cfg = tiny_config();
    update(&mut state, ev, &cfg);
    let s = state.browser.sparkline.as_ref().unwrap();
    assert_eq!(s.series.as_ref().unwrap().buckets.len(), 1);
    assert_eq!(s.series.as_ref().unwrap().buckets[0].in_count, 7);
    assert!(s.last_fetched_at.is_some());
}

#[test]
fn apply_sparkline_update_drops_stale_emit() {
    use crate::client::history::{Bucket, ComponentKind, StatusHistorySeries};
    let mut state = fresh_state();
    state
        .browser
        .open_sparkline_for_selection(ComponentKind::Processor, "p-1".into());
    let series = StatusHistorySeries {
        buckets: vec![Bucket {
            timestamp: std::time::SystemTime::now(),
            in_count: 99,
            out_count: 99,
            queued_count: None,
            task_time_ns: None,
        }],
        generated_at: std::time::SystemTime::now(),
    };
    // Stale emit — id mismatch.
    let ev = AppEvent::SparklineUpdate {
        kind: ComponentKind::Processor,
        id: "OTHER".into(),
        series,
    };
    let cfg = tiny_config();
    update(&mut state, ev, &cfg);
    assert!(
        state.browser.sparkline.as_ref().unwrap().series.is_none(),
        "stale id mismatch must be dropped"
    );
}

#[test]
fn apply_sparkline_endpoint_missing_sets_sticky_flag() {
    use crate::client::history::ComponentKind;
    let mut state = fresh_state();
    state
        .browser
        .open_sparkline_for_selection(ComponentKind::Connection, "c-1".into());
    let ev = AppEvent::SparklineEndpointMissing {
        kind: ComponentKind::Connection,
        id: "c-1".into(),
    };
    let cfg = tiny_config();
    update(&mut state, ev, &cfg);
    assert!(state.browser.sparkline.as_ref().unwrap().endpoint_missing);
}

#[test]
fn refresh_sparkline_no_selection_returns_none() {
    let mut state = fresh_state();
    let intent = state.refresh_sparkline_for_selection();
    assert!(intent.is_none());
    assert!(state.browser.sparkline.is_none());
}

#[test]
fn refresh_sparkline_processor_selection_emits_intent_and_opens_state() {
    use crate::client::history::ComponentKind;
    let (mut state, _c) = seeded_browser_state();
    // seeded_browser_state has nodes; expand root and select the processor "gen".
    let root_idx = state
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "root")
        .unwrap();
    state.browser.expanded.insert(root_idx);
    crate::view::browser::state::rebuild_visible(&mut state.browser);
    let gen_pos = state
        .browser
        .visible
        .iter()
        .position(|&i| state.browser.nodes[i].id == "gen")
        .unwrap();
    state.browser.selected = gen_pos;

    let intent = state
        .refresh_sparkline_for_selection()
        .expect("intent emitted");
    match intent {
        PendingIntent::SpawnSparklineFetchLoop {
            kind,
            id,
            cadence: _,
        } => {
            assert!(matches!(kind, ComponentKind::Processor));
            assert_eq!(id, "gen");
        }
        _ => panic!("wrong intent"),
    }
    let sparkline = state.browser.sparkline.as_ref().expect("opened");
    assert!(matches!(sparkline.kind, ComponentKind::Processor));
}

#[test]
fn refresh_sparkline_same_selection_is_noop() {
    let (mut state, _c) = seeded_browser_state();
    let root_idx = state
        .browser
        .nodes
        .iter()
        .position(|n| n.id == "root")
        .unwrap();
    state.browser.expanded.insert(root_idx);
    crate::view::browser::state::rebuild_visible(&mut state.browser);
    let gen_pos = state
        .browser
        .visible
        .iter()
        .position(|&i| state.browser.nodes[i].id == "gen")
        .unwrap();
    state.browser.selected = gen_pos;
    let _first = state
        .refresh_sparkline_for_selection()
        .expect("first intent");
    let second = state.refresh_sparkline_for_selection();
    assert!(
        second.is_none(),
        "calling refresh again with same selection must not respawn the worker"
    );
}
