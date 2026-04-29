use super::*;
use crate::client::browser::{FolderKind, NodeKind, NodeStatusSummary, RawNode};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::mpsc;

fn pg(id: &str, parent: Option<usize>, running: u32) -> RawNode {
    RawNode {
        parent_idx: parent,
        kind: NodeKind::ProcessGroup,
        id: id.into(),
        group_id: id.into(),
        name: id.into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
    }
}

fn proc(id: &str, parent: usize, status: &str) -> RawNode {
    RawNode {
        parent_idx: Some(parent),
        kind: NodeKind::Processor,
        id: id.into(),
        group_id: format!("g-{parent}"),
        name: id.into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: status.into(),
        },
    }
}

fn conn(id: &str, parent: usize, fill: u32) -> RawNode {
    RawNode {
        parent_idx: Some(parent),
        kind: NodeKind::Connection,
        id: id.into(),
        group_id: format!("g-{parent}"),
        name: id.into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: fill,
            flow_files_queued: 10,
            queued_display: "10 / 1KB".into(),
            source_id: String::new(),
            source_name: String::new(),
            destination_id: String::new(),
            destination_name: String::new(),
        },
    }
}

fn rpg(id: &str, parent: usize) -> RawNode {
    RawNode {
        parent_idx: Some(parent),
        kind: NodeKind::RemoteProcessGroup,
        id: id.into(),
        group_id: format!("g-{parent}"),
        name: id.into(),
        status_summary: NodeStatusSummary::RemoteProcessGroup {
            transmission_status: "Transmitting".into(),
            active_threads: 0,
            flow_files_received: 0,
            flow_files_sent: 0,
            bytes_received: 0,
            bytes_sent: 0,
            target_uri: "https://remote.example.com/nifi".into(),
        },
    }
}

fn snap(nodes: Vec<RawNode>) -> RecursiveSnapshot {
    RecursiveSnapshot {
        nodes,
        fetched_at: UNIX_EPOCH,
    }
}

/// Root PG (0) with a processor (1) and a connection (2), and a child
/// PG (3) with one processor (4).
fn demo_snap() -> RecursiveSnapshot {
    snap(vec![
        pg("root", None, 2),
        proc("gen", 0, "Running"),
        conn("c1", 0, 30),
        pg("ingest", Some(0), 1),
        proc("upd", 3, "Running"),
    ])
}

#[test]
fn empty_tree_after_first_snapshot_only_root_visible() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap(vec![pg("root", None, 0)]));
    assert_eq!(s.nodes.len(), 1);
    assert_eq!(s.visible, vec![0]);
    assert_eq!(s.selected, 0);
    assert!(s.expanded.contains(&0));
}

#[test]
fn first_tree_snapshot_auto_expands_root_and_selects_first_child() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Root (0), gen (1), Queues folder (5, collapsed), ingest (3,
    // collapsed) visible. B2 folder synthesis inserts a Queues folder
    // under root to bucket `c1`, so `c1` (arena 2) is hidden behind
    // the collapsed folder and `root.children == [1, 5, 3]`.
    assert_eq!(s.visible, vec![0, 1, 5, 3]);
    // First snapshot: no prior selection key, so selection falls back
    // to visible index 0 (the root).
    assert_eq!(s.selected, 0);
}

#[test]
fn expanded_set_is_preserved_by_id_across_arena_rebuild() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Manually expand "ingest" (arena idx 3).
    s.expanded.insert(3);
    rebuild_visible(&mut s);
    // B2 folder synthesis: root.children == [1, 5, 3] (gen, Queues
    // folder, ingest). Folder stays collapsed so c1 (arena 2) is
    // hidden; upd (arena 4) is visible because ingest is expanded.
    assert_eq!(s.visible, vec![0, 1, 5, 3, 4]);

    // Re-apply the same snapshot — indices stay the same, but the
    // retranslation path still runs.
    apply_tree_snapshot(&mut s, demo_snap());
    assert!(s.expanded.contains(&3));
    assert_eq!(s.visible, vec![0, 1, 5, 3, 4]);
}

#[test]
fn selection_is_preserved_by_id_across_arena_rebuild() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.expanded.insert(3);
    rebuild_visible(&mut s);
    // Move selection to "upd" (arena 4, visible index 4).
    s.selected = 4;

    // Re-apply with "gen" removed so indices shift.
    let shifted = snap(vec![
        pg("root", None, 2),
        conn("c1", 0, 30),
        pg("ingest", Some(0), 1),
        proc("upd", 2, "Running"),
    ]);
    apply_tree_snapshot(&mut s, shifted);
    // Selected node is now at arena idx 3, visible idx ...
    let upd_arena = s
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Processor && n.id == "upd")
        .unwrap();
    let upd_visible = s.visible.iter().position(|&i| i == upd_arena).unwrap();
    assert_eq!(s.selected, upd_visible);
}

#[test]
fn selection_resets_to_zero_when_previously_selected_node_is_gone() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.selected = 1; // "gen"

    // Rebuild without "gen".
    apply_tree_snapshot(
        &mut s,
        snap(vec![
            pg("root", None, 2),
            conn("c1", 0, 30),
            pg("ingest", Some(0), 1),
            proc("upd", 2, "Running"),
        ]),
    );
    assert_eq!(s.selected, 0);
}

#[test]
fn detail_focus_is_preserved_when_selected_node_persists_across_arena_rebuild() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Move selection to "gen" (Processor leaf).
    let gen_arena = s
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Processor && n.id == "gen")
        .unwrap();
    s.selected = s.visible.iter().position(|&i| i == gen_arena).unwrap();

    // Simulate the user descending into the detail pane and scrolling
    // around: focus on Properties (idx 0), row cursor at 3, horizontal
    // offset 5 — the state every periodic ClusterChanged tick would
    // otherwise wipe.
    s.detail_focus = DetailFocus::Section {
        idx: 0,
        rows: [3, 0, 0, 0, 0],
        x_offsets: [5, 0, 0, 0, 0],
    };

    // Re-apply the same snapshot (mirrors a periodic
    // ClusterChanged(RootPgStatus) arena rebuild while Browser is the
    // active tab).
    apply_tree_snapshot(&mut s, demo_snap());

    let gen_arena_after = s
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Processor && n.id == "gen")
        .unwrap();
    assert_eq!(s.visible[s.selected], gen_arena_after);
    assert_eq!(
        s.detail_focus,
        DetailFocus::Section {
            idx: 0,
            rows: [3, 0, 0, 0, 0],
            x_offsets: [5, 0, 0, 0, 0],
        }
    );
}

#[test]
fn detail_focus_resets_to_tree_when_selected_node_is_gone_after_rebuild() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    let gen_arena = s
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::Processor && n.id == "gen")
        .unwrap();
    s.selected = s.visible.iter().position(|&i| i == gen_arena).unwrap();
    s.detail_focus = DetailFocus::Section {
        idx: 1,
        rows: [0, 2, 0, 0, 0],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    // Rebuild without "gen" — selection falls back to root (different
    // node), so any carried-over section idx would be meaningless.
    apply_tree_snapshot(
        &mut s,
        snap(vec![
            pg("root", None, 2),
            conn("c1", 0, 30),
            pg("ingest", Some(0), 1),
            proc("upd", 2, "Running"),
        ]),
    );
    assert_eq!(s.detail_focus, DetailFocus::Tree);
}

#[test]
fn details_are_dropped_when_their_node_leaves_the_arena() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Stuff a fake detail for "gen" (arena 1).
    s.details.insert(
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

    apply_tree_snapshot(
        &mut s,
        snap(vec![
            pg("root", None, 2),
            conn("c1", 0, 30),
            pg("ingest", Some(0), 1),
            proc("upd", 2, "Running"),
        ]),
    );
    // "gen" no longer in arena; detail gone.
    assert_eq!(
        s.details.len(),
        0,
        "detail for removed 'gen' node must be dropped"
    );
}

#[test]
fn rebuild_visible_respects_collapse() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Initially ingest is collapsed; visible excludes "upd".
    assert_eq!(s.visible.len(), 4);
    s.expanded.insert(3);
    rebuild_visible(&mut s);
    assert_eq!(s.visible.len(), 5);
    s.expanded.remove(&3);
    rebuild_visible(&mut s);
    assert_eq!(s.visible.len(), 4);
}

#[test]
fn move_down_advances_visible_index() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    assert_eq!(s.selected, 0);
    s.move_down();
    assert_eq!(s.selected, 1);
    s.move_down();
    s.move_down();
    s.move_down(); // beyond the last row: clamps.
    assert_eq!(s.selected, s.visible.len() - 1);
}

#[test]
fn move_up_at_zero_stays_at_zero() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.move_up();
    assert_eq!(s.selected, 0);
}

#[test]
fn enter_on_collapsed_pg_expands_and_moves_to_first_child() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Move selection to "ingest" (visible row 3, arena 3).
    s.selected = 3;
    s.enter_selection();
    assert!(s.expanded.contains(&3));
    // First child of ingest is "upd" at arena 4.
    let selected_arena = s.visible[s.selected];
    assert_eq!(s.nodes[selected_arena].id, "upd");
}

#[test]
fn enter_on_leaf_is_noop() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.selected = 1; // "gen" processor
    let before = s.selected;
    s.enter_selection();
    assert_eq!(s.selected, before);
}

#[test]
fn backspace_on_expanded_pg_collapses_subtree_in_place() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.expanded.insert(3);
    rebuild_visible(&mut s);
    s.selected = 3; // "ingest"
    s.backspace_selection();
    assert!(!s.expanded.contains(&3));
    let selected_arena = s.visible[s.selected];
    assert_eq!(s.nodes[selected_arena].id, "ingest");
}

#[test]
fn backspace_on_leaf_selects_parent() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.expanded.insert(3);
    rebuild_visible(&mut s);
    // Select "upd" (child of ingest).
    let upd_arena = s.nodes.iter().position(|n| n.id == "upd").unwrap();
    let upd_visible = s.visible.iter().position(|&i| i == upd_arena).unwrap();
    s.selected = upd_visible;

    s.backspace_selection();
    let new_arena = s.visible[s.selected];
    assert_eq!(s.nodes[new_arena].id, "ingest");
}

#[test]
fn page_down_moves_by_height() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.page_down(2);
    assert_eq!(s.selected, 2);
    s.page_down(100);
    assert_eq!(s.selected, s.visible.len() - 1);
}

#[test]
fn home_and_end_goto_first_and_last_visible() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.goto_last();
    assert_eq!(s.selected, s.visible.len() - 1);
    s.goto_first();
    assert_eq!(s.selected, 0);
}

#[test]
fn selection_change_sets_pending_detail_and_pushes_request() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    let (tx, mut rx) = mpsc::unbounded_channel();
    s.detail_tx = Some(tx);
    s.move_down(); // select arena 1 ("gen")
    s.emit_detail_request_for_current_selection();
    assert_eq!(s.pending_detail, Some(1));
    let req = rx.try_recv().expect("request pushed");
    assert_eq!(req.arena_idx, 1);
    assert_eq!(req.kind, NodeKind::Processor);
    assert_eq!(req.id, "gen");
}

#[test]
fn selecting_rpg_emits_detail_fetch_request() {
    // Arena: root PG (0) with one RPG child (1). Selecting the RPG row
    // must enqueue a `DetailRequest` on `detail_tx` with the matching
    // (kind, id), the same mechanism processor selection uses — the
    // worker arm dispatches on `NodeKind::RemoteProcessGroup`.
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap(vec![pg("root", None, 0), rpg("rpg-1", 0)]));
    let (tx, mut rx) = mpsc::unbounded_channel();
    s.detail_tx = Some(tx);

    // Move down to the RPG row (visible idx 1 = arena idx 1).
    s.move_down();
    s.emit_detail_request_for_current_selection();

    assert_eq!(s.pending_detail, Some(1));
    let req = rx.try_recv().expect("RPG detail request pushed");
    assert_eq!(req.arena_idx, 1);
    assert_eq!(req.kind, NodeKind::RemoteProcessGroup);
    assert_eq!(req.id, "rpg-1");
}

#[test]
fn selection_change_with_no_detail_tx_is_noop_but_sets_pending() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.move_down();
    s.emit_detail_request_for_current_selection();
    assert_eq!(s.pending_detail, Some(1));
}

#[test]
fn apply_node_detail_lands_rpg_payload_in_details_cache() {
    use crate::client::browser::RemoteProcessGroupDetail;
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap(vec![pg("root", None, 0), rpg("rpg-1", 0)]));

    let payload = NodeDetailSnapshot {
        arena_idx: 1,
        kind: NodeKind::RemoteProcessGroup,
        id: "rpg-1".into(),
        detail: NodeDetail::RemoteProcessGroup(RemoteProcessGroupDetail {
            id: "rpg-1".into(),
            name: "Sink".into(),
            parent_group_id: Some("root".into()),
            target_uri: "https://remote/nifi".into(),
            target_secure: true,
            transport_protocol: "HTTP".into(),
            transmission_status: "Transmitting".into(),
            validation_status: "VALID".into(),
            validation_errors: vec![],
            comments: String::new(),
            input_ports: vec![],
            output_ports: vec![],
            active_remote_input_port_count: 0,
            inactive_remote_input_port_count: 0,
            active_remote_output_port_count: 0,
            inactive_remote_output_port_count: 0,
        }),
    };
    apply_node_detail(&mut s, payload);
    // The arena-guard check (kind + id match against `state.nodes[idx]`)
    // is the only stale-emit defense; payloads land in `state.details`
    // keyed by arena index, the same shape the renderer reads.
    let detail = match s.details.get(&1) {
        Some(NodeDetail::RemoteProcessGroup(d)) => d,
        other => panic!("expected RemoteProcessGroup detail, got {other:?}"),
    };
    assert_eq!(detail.id, "rpg-1");
    assert_eq!(detail.name, "Sink");
}

#[test]
fn apply_node_detail_clears_pending_when_matching_index() {
    use crate::client::ProcessorDetail;
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.pending_detail = Some(1);
    let payload = NodeDetailSnapshot {
        arena_idx: 1,
        kind: NodeKind::Processor,
        id: "gen".into(),
        detail: NodeDetail::Processor(ProcessorDetail {
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
    };
    apply_node_detail(&mut s, payload);
    assert_eq!(s.pending_detail, None);
    assert!(matches!(s.details.get(&1), Some(NodeDetail::Processor(_))));
}

#[test]
fn apply_node_detail_accepts_stale_response_without_clearing_pending() {
    use crate::client::ProcessorDetail;
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    s.pending_detail = Some(2); // user moved on to arena 2
    let payload = NodeDetailSnapshot {
        arena_idx: 1,
        kind: NodeKind::Processor,
        id: "gen".into(),
        detail: NodeDetail::Processor(ProcessorDetail {
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
    };
    apply_node_detail(&mut s, payload);
    assert_eq!(s.pending_detail, Some(2));
    assert!(s.details.contains_key(&1));
}

#[test]
fn flow_index_is_rebuilt_fresh_on_every_snapshot() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    let idx1 = build_flow_index(&s);
    // B3: folders are filtered out of the fuzzy-find flow index, so
    // the synthesized Queues folder does not contribute an entry —
    // we see the 5 raw nodes only.
    assert_eq!(idx1.entries.len(), 5);
    let shifted = snap(vec![pg("root", None, 2), proc("only", 0, "Running")]);
    apply_tree_snapshot(&mut s, shifted);
    let idx2 = build_flow_index(&s);
    // No connections or CS, so no folders synthesized.
    assert_eq!(idx2.entries.len(), 2);
}

#[test]
fn flow_index_group_path_walks_parent_chain() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    let idx = build_flow_index(&s);
    let upd = idx.entries.iter().find(|e| e.id == "upd").unwrap();
    assert_eq!(upd.group_path, "root/ingest");
}

#[test]
fn build_flow_index_populates_structured_fields() {
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    let idx = build_flow_index(&s);
    // "upd" sits under root/ingest per demo_snap
    let upd = idx.entries.iter().find(|e| e.id == "upd").unwrap();
    assert_eq!(upd.name, "upd");
    assert_eq!(upd.group_path, "root/ingest");
    match &upd.state {
        StateBadge::Processor { glyph, .. } => assert_eq!(*glyph, '\u{25CF}'),
        _ => panic!("expected Processor state badge"),
    }
}

#[test]
fn build_flow_index_populates_cs_state_badge() {
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![1],
        kind: NodeKind::ProcessGroup,
        id: "root".into(),
        group_id: String::new(),
        name: "Root".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::ControllerService,
        id: "cs1".into(),
        group_id: "root".into(),
        name: "kafka-brokers".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });
    let idx = build_flow_index(&s);
    let cs = idx.entries.iter().find(|e| e.id == "cs1").unwrap();
    match &cs.state {
        StateBadge::Cs { label, .. } => assert_eq!(label, "ENABLED"),
        _ => panic!("expected Cs state badge"),
    }
}

#[test]
fn build_flow_index_populates_invalid_count_for_pg() {
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "root".into(),
        group_id: String::new(),
        name: "Root".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 1,
            stopped: 0,
            invalid: 2,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    let idx = build_flow_index(&s);
    let pg = idx.entries.iter().find(|e| e.id == "root").unwrap();
    match &pg.state {
        StateBadge::Pg { invalid } => assert_eq!(*invalid, 2),
        _ => panic!("expected Pg state badge"),
    }
}

#[test]
fn apply_node_detail_silently_drops_when_node_gone() {
    use crate::client::ProcessorDetail;
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, demo_snap());
    // Bogus arena_idx beyond current nodes length.
    let bogus = s.nodes.len();
    let payload = NodeDetailSnapshot {
        arena_idx: bogus,
        kind: NodeKind::Processor,
        id: "gone".into(),
        detail: NodeDetail::Processor(ProcessorDetail {
            id: "gone".into(),
            name: "Gone".into(),
            type_name: "x".into(),
            bundle: String::new(),
            run_status: String::new(),
            scheduling_strategy: String::new(),
            scheduling_period: String::new(),
            concurrent_tasks: 0,
            run_duration_ms: 0,
            penalty_duration: String::new(),
            yield_duration: String::new(),
            bulletin_level: String::new(),
            properties: vec![],
            validation_errors: vec![],
        }),
    };
    let before = s.details.len();
    apply_node_detail(&mut s, payload);
    assert_eq!(s.details.len(), before);
}

#[test]
fn breadcrumb_segments_at_root() {
    let mut state = BrowserState::default();
    state.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "root-id".into(),
        group_id: String::new(),
        name: "NiFi Flow".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    state.visible = vec![0];
    state.selected = 0;

    let segs = state.breadcrumb_segments();
    assert_eq!(segs.len(), 1);
    assert_eq!(segs[0].name, "NiFi Flow");
    assert_eq!(segs[0].arena_idx, 0);
}

#[test]
fn breadcrumb_segments_nested() {
    // Build Root > Pipeline > Generate
    let mut state = BrowserState::default();
    state.nodes.push(TreeNode {
        parent: None,
        children: vec![1],
        kind: NodeKind::ProcessGroup,
        id: "root-id".into(),
        group_id: String::new(),
        name: "Root".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    state.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![2],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: "root-id".into(),
        name: "Pipeline".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    state.nodes.push(TreeNode {
        parent: Some(1),
        children: vec![],
        kind: NodeKind::Processor,
        id: "proc-1".into(),
        group_id: "pg-1".into(),
        name: "Generate".into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });
    state.visible = vec![0, 1, 2];
    state.selected = 2;

    let segs = state.breadcrumb_segments();
    assert_eq!(segs.len(), 3);
    assert_eq!(segs[0].name, "Root");
    assert_eq!(segs[1].name, "Pipeline");
    assert_eq!(segs[2].name, "Generate");
}

#[test]
fn pg_path_returns_none_for_unknown_group_id() {
    let s = BrowserState::new();
    assert!(s.pg_path("nonexistent").is_none());
}

#[test]
fn pg_path_joins_ancestor_pg_names_excluding_root() {
    // Build a minimal tree: Root → noisy-pipeline → inner
    let fixture = snap(vec![
        pg("root-id", None, 0),
        RawNode {
            parent_idx: Some(0),
            kind: NodeKind::ProcessGroup,
            id: "noisy-pipeline".into(),
            group_id: "root-id".into(),
            name: "noisy-pipeline".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        },
        RawNode {
            parent_idx: Some(1),
            kind: NodeKind::ProcessGroup,
            id: "inner".into(),
            group_id: "noisy-pipeline".into(),
            name: "inner".into(),
            status_summary: NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
        },
    ]);
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, fixture);
    // PG path for the inner PG should be "noisy-pipeline / inner".
    assert_eq!(
        s.pg_path("inner").as_deref(),
        Some("noisy-pipeline / inner"),
    );
    // PG path for the noisy-pipeline itself is just "noisy-pipeline".
    assert_eq!(
        s.pg_path("noisy-pipeline").as_deref(),
        Some("noisy-pipeline"),
    );
    // Root PG has no path (root name is intentionally dropped).
    assert_eq!(s.pg_path("root-id"), None);
}

#[test]
fn pg_health_rollup_green_when_all_running() {
    // Root PG → one processor with RUNNING.
    let snap = RecursiveSnapshot {
        fetched_at: UNIX_EPOCH,

        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "root".into(),
                group_id: "root".into(),
                name: "Root".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 1,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "p1".into(),
                group_id: "root".into(),
                name: "P1".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "RUNNING".into(),
                },
            },
        ],
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert_eq!(s.pg_health_rollup(0), PgHealth::Green);
}

#[test]
fn pg_health_rollup_yellow_when_any_stopped() {
    let snap = RecursiveSnapshot {
        fetched_at: UNIX_EPOCH,

        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "root".into(),
                group_id: "root".into(),
                name: "Root".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 1,
                    stopped: 1,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "p1".into(),
                group_id: "root".into(),
                name: "P1".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "RUNNING".into(),
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "p2".into(),
                group_id: "root".into(),
                name: "P2".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "STOPPED".into(),
                },
            },
        ],
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert_eq!(s.pg_health_rollup(0), PgHealth::Yellow);
}

#[test]
fn pg_health_rollup_red_when_any_invalid_even_if_some_stopped() {
    let snap = RecursiveSnapshot {
        fetched_at: UNIX_EPOCH,

        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "root".into(),
                group_id: "root".into(),
                name: "Root".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 1,
                    invalid: 1,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "p1".into(),
                group_id: "root".into(),
                name: "P1".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "STOPPED".into(),
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Processor,
                id: "p2".into(),
                group_id: "root".into(),
                name: "P2".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "INVALID".into(),
                },
            },
        ],
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert_eq!(s.pg_health_rollup(0), PgHealth::Red);
}

#[test]
fn pg_health_rollup_recurses_into_child_pgs() {
    // Root PG → inner PG → processor INVALID.
    let snap = RecursiveSnapshot {
        fetched_at: UNIX_EPOCH,

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
                    invalid: 1,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::ProcessGroup,
                id: "inner".into(),
                group_id: "root".into(),
                name: "inner".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 0,
                    stopped: 0,
                    invalid: 1,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(1),
                kind: NodeKind::Processor,
                id: "p1".into(),
                group_id: "inner".into(),
                name: "P1".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "INVALID".into(),
                },
            },
        ],
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    // Rollup at root PG finds the invalid grandchild → Red.
    assert_eq!(s.pg_health_rollup(0), PgHealth::Red);
}

#[test]
fn pg_health_rollup_green_for_empty_pg() {
    let snap = RecursiveSnapshot {
        fetched_at: UNIX_EPOCH,

        nodes: vec![RawNode {
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
        }],
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert_eq!(s.pg_health_rollup(0), PgHealth::Green);
}

#[test]
fn child_process_groups_returns_direct_children_with_counts() {
    // Root → noisy (pg), healthy (pg), connection (not a pg).
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
                id: "noisy".into(),
                group_id: "root".into(),
                name: "noisy".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 3,
                    stopped: 2,
                    invalid: 1,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::ProcessGroup,
                id: "healthy".into(),
                group_id: "root".into(),
                name: "healthy".into(),
                status_summary: NodeStatusSummary::ProcessGroup {
                    running: 5,
                    stopped: 0,
                    invalid: 0,
                    disabled: 0,
                },
            },
            RawNode {
                parent_idx: Some(0),
                kind: NodeKind::Connection,
                id: "c1".into(),
                group_id: "root".into(),
                name: "conn".into(),
                status_summary: NodeStatusSummary::Connection {
                    fill_percent: 10,
                    flow_files_queued: 100,
                    queued_display: "100 / 1 KB".into(),
                    source_id: String::new(),
                    source_name: String::new(),
                    destination_id: String::new(),
                    destination_name: String::new(),
                },
            },
        ],
        fetched_at: UNIX_EPOCH,
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    let kids = s.child_process_groups("root");
    assert_eq!(kids.len(), 2);
    assert_eq!(kids[0].name, "noisy");
    assert_eq!(kids[0].running, 3);
    assert_eq!(kids[0].stopped, 2);
    assert_eq!(kids[0].invalid, 1);
    assert_eq!(kids[1].name, "healthy");
    assert_eq!(kids[1].running, 5);
}

#[test]
fn child_process_groups_returns_empty_for_unknown_pg() {
    let s = BrowserState::new();
    assert!(s.child_process_groups("nope").is_empty());
}

#[test]
fn child_process_groups_excludes_non_pg_children() {
    let snap = RecursiveSnapshot {
        nodes: vec![
            RawNode {
                parent_idx: None,
                kind: NodeKind::ProcessGroup,
                id: "pg1".into(),
                group_id: "pg1".into(),
                name: "pg1".into(),
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
                id: "p1".into(),
                group_id: "pg1".into(),
                name: "P1".into(),
                status_summary: NodeStatusSummary::Processor {
                    run_status: "RUNNING".into(),
                },
            },
        ],
        fetched_at: UNIX_EPOCH,
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert!(s.child_process_groups("pg1").is_empty());
}

#[test]
fn state_badge_processor_carries_icon_and_style() {
    use super::StateBadge;
    use crate::widget::run_icon::processor_run_icon;
    let (glyph, style) = processor_run_icon("RUNNING");
    let badge = StateBadge::Processor { glyph, style };
    match badge {
        StateBadge::Processor { glyph: g, style: s } => {
            assert_eq!(g, '\u{25CF}');
            assert_eq!(s, crate::theme::success());
        }
        _ => panic!("expected Processor variant"),
    }
}

#[test]
fn for_node_process_group_returns_three_sections() {
    use crate::client::NodeKind;
    let sections = DetailSections::for_node(NodeKind::ProcessGroup);
    assert_eq!(
        sections.0,
        &[
            DetailSection::ControllerServices,
            DetailSection::ChildGroups,
            DetailSection::RecentBulletins,
        ][..],
    );
    assert_eq!(sections.len(), 3);
}

#[test]
fn section_len_process_group_sections() {
    use crate::client::{
        BulletinSnapshot, ControllerServiceSummary, NodeKind, NodeStatusSummary, ProcessGroupDetail,
    };
    use std::collections::VecDeque;

    let mut s = BrowserState::new();
    // Minimal arena: one PG node at idx 0, visible.
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: String::new(),
        name: "pg-1".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.visible = vec![0];
    s.selected = 0;
    s.details.insert(
        0,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: "pg-1".into(),
            name: "pg-1".into(),
            parent_group_id: None,
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0 / 0 B".into(),
            controller_services: vec![
                ControllerServiceSummary {
                    id: "cs1".into(),
                    name: "cs1".into(),
                    type_short: "T".into(),
                    state: "ENABLED".into(),
                },
                ControllerServiceSummary {
                    id: "cs2".into(),
                    name: "cs2".into(),
                    type_short: "T".into(),
                    state: "ENABLED".into(),
                },
            ],
        }),
    );

    // No child PGs in the arena, one bulletin in the ring for this group.
    let mut ring: VecDeque<BulletinSnapshot> = VecDeque::new();
    ring.push_back(BulletinSnapshot {
        id: 1,
        level: "WARN".into(),
        message: "hi".into(),
        source_id: "p1".into(),
        source_name: "p1".into(),
        source_type: "PROCESSOR".into(),
        group_id: "pg-1".into(),
        timestamp_iso: "2026-04-14T10:00:00.000Z".into(),
        timestamp_human: "04/14/2026 10:00:00 UTC".into(),
    });

    assert_eq!(s.section_len(DetailSection::ControllerServices, &ring), 2);
    assert_eq!(s.section_len(DetailSection::ChildGroups, &ring), 0);
    assert_eq!(s.section_len(DetailSection::RecentBulletins, &ring), 1);
}

#[test]
fn focused_row_copy_value_pg_controller_services() {
    use crate::client::{
        ControllerServiceSummary, NodeKind, NodeStatusSummary, ProcessGroupDetail,
    };
    use std::collections::VecDeque;

    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: String::new(),
        name: "pg-1".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.visible = vec![0];
    s.selected = 0;
    s.details.insert(
        0,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: "pg-1".into(),
            name: "pg-1".into(),
            parent_group_id: None,
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            queued_display: "0 / 0 B".into(),
            controller_services: vec![
                ControllerServiceSummary {
                    id: "cs-a".into(),
                    name: "cs-a".into(),
                    type_short: "T".into(),
                    state: "ENABLED".into(),
                },
                ControllerServiceSummary {
                    id: "cs-b".into(),
                    name: "cs-b".into(),
                    type_short: "T".into(),
                    state: "ENABLED".into(),
                },
            ],
        }),
    );
    s.detail_focus = DetailFocus::Section {
        idx: 0, // ControllerServices is the first section for PG
        rows: [1, 0, 0, 0, 0],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let ring: VecDeque<crate::client::BulletinSnapshot> = VecDeque::new();
    assert_eq!(s.focused_row_copy_value(&ring).as_deref(), Some("cs-b"),);
}

#[test]
fn focused_row_source_id_pg_recent_bulletins_returns_nth_newest_source() {
    use crate::client::{BulletinSnapshot, NodeKind, NodeStatusSummary, ProcessGroupDetail};
    use std::collections::VecDeque;

    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: String::new(),
        name: "pg-1".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.visible = vec![0];
    s.selected = 0;
    s.details.insert(
        0,
        NodeDetail::ProcessGroup(ProcessGroupDetail {
            id: "pg-1".into(),
            name: "pg-1".into(),
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
    // Focus is PG's RecentBulletins section (idx 2).
    s.detail_focus = DetailFocus::Section {
        idx: 2,
        rows: [0, 0, 1, 0, 0],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    // Ring: older → newer. Newest-first iteration should be [b3, b2, b1].
    let mut ring: VecDeque<BulletinSnapshot> = VecDeque::new();
    for (i, src) in ["p1", "p2", "p3"].iter().enumerate() {
        ring.push_back(BulletinSnapshot {
            id: (10 + i) as i64,
            level: "INFO".into(),
            message: format!("m{i}"),
            source_id: (*src).into(),
            source_name: (*src).into(),
            source_type: "PROCESSOR".into(),
            group_id: "pg-1".into(),
            timestamp_iso: "".into(),
            timestamp_human: "".into(),
        });
    }
    // Row 1 newest-first → p2.
    assert_eq!(s.focused_row_source_id(&ring).as_deref(), Some("p2"),);
}

#[test]
fn drill_into_group_expands_ancestors_and_selects_child() {
    use crate::client::{NodeKind, NodeStatusSummary};

    // Arena: root (idx 0) → ingest (idx 1) → enrich (idx 2).
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![1],
        kind: NodeKind::ProcessGroup,
        id: "root".into(),
        group_id: String::new(),
        name: "root".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.nodes.push(TreeNode {
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
    s.nodes.push(TreeNode {
        parent: Some(1),
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "enrich".into(),
        group_id: "ingest".into(),
        name: "enrich".into(),
        status_summary: NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    // Start with only the root visible, selection on root, no expansion.
    s.visible = vec![0];
    s.selected = 0;
    s.detail_focus = DetailFocus::Section {
        idx: 1,
        rows: [0; MAX_DETAIL_SECTIONS],
        x_offsets: [0; MAX_DETAIL_SECTIONS],
    };

    let ok = s.drill_into_group("enrich");
    assert!(ok);

    // root and ingest must be expanded now; `visible` must contain
    // [root, ingest, enrich]; selected must point to enrich.
    assert!(s.expanded.contains(&0));
    assert!(s.expanded.contains(&1));
    assert_eq!(s.visible, vec![0, 1, 2]);
    assert_eq!(s.visible[s.selected], 2);
    // And detail focus must have reset.
    assert_eq!(s.detail_focus, DetailFocus::Tree);
}

#[test]
fn drill_into_group_missing_id_returns_false() {
    let mut s = BrowserState::new();
    assert!(!s.drill_into_group("nope"));
}

fn cs(id: &str, parent: Option<usize>, state: &str) -> RawNode {
    RawNode {
        parent_idx: parent,
        kind: NodeKind::ControllerService,
        id: id.into(),
        group_id: parent.map(|_| "root".to_string()).unwrap_or_default(),
        name: id.into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: state.into(),
        },
    }
}

#[test]
fn apply_tree_snapshot_inserts_folder_for_queues_and_cs() {
    let fetched = SystemTime::now();
    let snap = RecursiveSnapshot {
        nodes: vec![
            pg("root", None, 1),
            proc("p1", 0, "Running"),
            conn("c1", 0, 30),
            cs("cs1", Some(0), "ENABLED"),
        ],
        fetched_at: fetched,
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);

    // Expected arena:
    //   0: PG root
    //   1: Processor p1 (child of root)
    //   2: Connection c1 (reparented to Queues folder)
    //   3: CS cs1         (reparented to CS folder)
    //   4: Folder(Queues) (child of root)
    //   5: Folder(ControllerServices) (child of root)

    assert_eq!(s.nodes.len(), 6);
    assert!(matches!(
        s.nodes[4].kind,
        NodeKind::Folder(FolderKind::Queues)
    ));
    assert!(matches!(
        s.nodes[5].kind,
        NodeKind::Folder(FolderKind::ControllerServices)
    ));

    // Processor remains directly under root; folders appear after processors.
    assert_eq!(s.nodes[0].children, vec![1, 4, 5]);
    // Connection re-parented to the Queues folder.
    assert_eq!(s.nodes[4].children, vec![2]);
    assert_eq!(s.nodes[2].parent, Some(4));
    // CS re-parented to the CS folder.
    assert_eq!(s.nodes[5].children, vec![3]);
    assert_eq!(s.nodes[3].parent, Some(5));
}

#[test]
fn apply_tree_snapshot_skips_empty_folders() {
    let snap = RecursiveSnapshot {
        nodes: vec![pg("root", None, 1), proc("p1", 0, "Running")],
        fetched_at: SystemTime::now(),
    };
    let mut s = BrowserState::new();
    apply_tree_snapshot(&mut s, snap);
    assert_eq!(s.nodes.len(), 2);
    assert!(
        s.nodes
            .iter()
            .all(|n| !matches!(n.kind, NodeKind::Folder(_)))
    );
}

#[test]
fn is_uuid_shape_accepts_canonical_uuid() {
    assert!(super::is_uuid_shape("a1b2c3d4-e5f6-7890-abcd-ef1234567890"));
}

#[test]
fn is_uuid_shape_rejects_wrong_length() {
    assert!(!super::is_uuid_shape("too-short"));
    assert!(!super::is_uuid_shape(
        "a1b2c3d4-e5f6-7890-abcd-ef1234567890-extra"
    ));
}

#[test]
fn is_uuid_shape_rejects_missing_hyphens() {
    // 36 chars but hyphens in wrong positions.
    assert!(!super::is_uuid_shape(
        "a1b2c3d4e5f67890abcdef12345678901234"
    ));
}

#[test]
fn is_uuid_shape_rejects_non_hex() {
    assert!(!super::is_uuid_shape(
        "Z1b2c3d4-e5f6-7890-abcd-ef1234567890"
    ));
}

#[test]
fn is_uuid_shape_accepts_uppercase_hex() {
    assert!(super::is_uuid_shape("A1B2C3D4-E5F6-7890-ABCD-EF1234567890"));
}

#[test]
fn resolve_id_returns_none_for_non_uuid_string() {
    let s = BrowserState::new();
    assert!(s.resolve_id("not-a-uuid").is_none());
    assert!(s.resolve_id("").is_none());
    assert!(s.resolve_id("   ").is_none());
}

#[test]
fn resolve_id_returns_none_for_uuid_not_in_arena() {
    let s = BrowserState::new();
    assert!(
        s.resolve_id("a1b2c3d4-e5f6-7890-abcd-ef1234567890")
            .is_none()
    );
}

#[test]
fn resolve_id_returns_ref_for_known_node() {
    use crate::client::{NodeKind, NodeStatusSummary};
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ControllerService,
        id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".into(),
        group_id: "root-pg".into(),
        name: "pool".into(),
        status_summary: NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });
    let got = s
        .resolve_id("a1b2c3d4-e5f6-7890-abcd-ef1234567890")
        .expect("resolve_id returns Some");
    assert_eq!(got.arena_idx, 0);
    assert_eq!(got.kind, NodeKind::ControllerService);
    assert_eq!(got.name, "pool");
    assert_eq!(got.group_id, "root-pg");
}

#[test]
fn resolve_id_trims_whitespace() {
    use crate::client::{NodeKind, NodeStatusSummary};
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::Processor,
        id: "a1b2c3d4-e5f6-7890-abcd-ef1234567890".into(),
        group_id: "g".into(),
        name: "p".into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });
    assert!(
        s.resolve_id("   a1b2c3d4-e5f6-7890-abcd-ef1234567890   ")
            .is_some()
    );
}

#[test]
fn resolve_id_matches_remote_process_group_arena_entry() {
    use crate::client::{NodeKind, NodeStatusSummary};
    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::RemoteProcessGroup,
        id: "b2c3d4e5-f6a7-8901-bcde-f12345678901".into(),
        group_id: "pg-root".into(),
        name: "MyRemoteSink".into(),
        status_summary: NodeStatusSummary::RemoteProcessGroup {
            transmission_status: "Transmitting".into(),
            active_threads: 0,
            flow_files_received: 0,
            flow_files_sent: 0,
            bytes_received: 0,
            bytes_sent: 0,
            target_uri: "https://remote.example.com/nifi".into(),
        },
        parameter_context_ref: None,
    });
    let got = s
        .resolve_id("b2c3d4e5-f6a7-8901-bcde-f12345678901")
        .expect("RPG must resolve");
    assert_eq!(got.arena_idx, 0);
    assert!(matches!(got.kind, NodeKind::RemoteProcessGroup));
    assert_eq!(got.name, "MyRemoteSink");
    assert_eq!(got.group_id, "pg-root");
}

#[test]
fn connections_for_processor_splits_in_and_out_edges() {
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::view::browser::state::{EdgeDirection, TreeNode};

    let mut s = BrowserState::new();
    // arena: root PG (0), proc-A (1), proc-B (2),
    //        conn A→B (3), conn B→A (4)
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![1, 2, 3, 4],
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
    for (id, name) in [("proc-A", "A"), ("proc-B", "B")] {
        s.nodes.push(TreeNode {
            parent: Some(0),
            children: vec![],
            kind: NodeKind::Processor,
            id: id.into(),
            group_id: "root".into(),
            name: name.into(),
            status_summary: NodeStatusSummary::Processor {
                run_status: "Running".into(),
            },
            parameter_context_ref: None,
        });
    }
    s.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Connection,
        id: "conn-ab".into(),
        group_id: "root".into(),
        name: "A→B".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 3,
            queued_display: "3 / 1KB".into(),
            source_id: "proc-A".into(),
            source_name: "A".into(),
            destination_id: "proc-B".into(),
            destination_name: "B".into(),
        },
        parameter_context_ref: None,
    });
    s.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Connection,
        id: "conn-ba".into(),
        group_id: "root".into(),
        name: "B→A".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 5,
            queued_display: "5 / 2KB".into(),
            source_id: "proc-B".into(),
            source_name: "B".into(),
            destination_id: "proc-A".into(),
            destination_name: "A".into(),
        },
        parameter_context_ref: None,
    });

    let edges = s.connections_for_processor("proc-A");
    assert_eq!(edges.len(), 2);

    let out = edges
        .iter()
        .find(|e| e.direction == EdgeDirection::Out)
        .expect("A has outgoing edge A→B");
    assert_eq!(out.connection_id, "conn-ab");
    assert_eq!(out.opposite_id, "proc-B");
    assert_eq!(out.opposite_name, "B");
    assert_eq!(out.queued_display, "3 / 1KB");

    let inb = edges
        .iter()
        .find(|e| e.direction == EdgeDirection::In)
        .expect("A has incoming edge B→A");
    assert_eq!(inb.opposite_id, "proc-B");
    assert_eq!(inb.opposite_name, "B");
    assert_eq!(inb.queued_display, "5 / 2KB");
}

#[test]
fn connections_for_processor_empty_when_processor_has_no_edges() {
    let s = BrowserState::new();
    assert!(s.connections_for_processor("unknown-id").is_empty());
}

#[test]
fn connections_for_processor_falls_back_to_empty_group_id_for_unresolvable_opposite() {
    use crate::client::{NodeKind, NodeStatusSummary};
    use crate::view::browser::state::TreeNode;

    let mut s = BrowserState::new();
    // Only a connection — neither endpoint exists in the arena.
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::Connection,
        id: "conn-x".into(),
        group_id: "root".into(),
        name: "x".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: "0".into(),
            source_id: "proc-A".into(),
            source_name: "A".into(),
            destination_id: "proc-B".into(),
            destination_name: "B".into(),
        },
        parameter_context_ref: None,
    });
    let edges = s.connections_for_processor("proc-A");
    assert_eq!(edges.len(), 1);
    assert_eq!(edges[0].opposite_group_id, "");
}

#[test]
fn for_node_connection_returns_endpoints_section() {
    use crate::client::NodeKind;
    let s = DetailSections::for_node(NodeKind::Connection);
    assert_eq!(s.0, &[DetailSection::Endpoints][..]);
    assert_eq!(s.len(), 1);
}

#[test]
fn section_len_endpoints_is_always_two() {
    use crate::client::{ConnectionDetail, NodeKind, NodeStatusSummary};
    use crate::view::browser::state::NodeDetail;
    use std::collections::VecDeque;

    let mut s = BrowserState::new();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::Connection,
        id: "c".into(),
        group_id: "g".into(),
        name: "c".into(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: "0".into(),
            source_id: "s".into(),
            source_name: "S".into(),
            destination_id: "d".into(),
            destination_name: "D".into(),
        },
        parameter_context_ref: None,
    });
    crate::view::browser::state::rebuild_visible(&mut s);
    s.selected = 0;
    s.details.insert(
        0,
        NodeDetail::Connection(ConnectionDetail {
            id: "c".into(),
            name: "c".into(),
            source_id: "s".into(),
            source_name: "S".into(),
            source_type: "PROCESSOR".into(),
            source_group_id: "g".into(),
            destination_id: "d".into(),
            destination_name: "D".into(),
            destination_type: "PROCESSOR".into(),
            destination_group_id: "g".into(),
            selected_relationships: vec![],
            available_relationships: vec![],
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
    let ring: VecDeque<crate::client::BulletinSnapshot> = VecDeque::new();
    assert_eq!(s.section_len(DetailSection::Endpoints, &ring), 2);
}

#[test]
fn for_node_processor_now_includes_connections_section() {
    use crate::client::NodeKind;
    let s = DetailSections::for_node(NodeKind::Processor);
    assert_eq!(
        s.0,
        &[
            DetailSection::Properties,
            DetailSection::Connections,
            DetailSection::RecentBulletins,
        ][..]
    );
}

#[test]
fn for_node_detail_processor_with_validation_includes_connections_before_bulletins() {
    use crate::client::NodeKind;
    let s = DetailSections::for_node_detail(NodeKind::Processor, true);
    assert_eq!(
        s.0,
        &[
            DetailSection::Properties,
            DetailSection::ValidationErrors,
            DetailSection::Connections,
            DetailSection::RecentBulletins,
        ][..]
    );
}

#[test]
fn section_len_connections_counts_processor_edges() {
    use crate::client::{NodeKind, NodeStatusSummary, ProcessorDetail};
    use crate::view::browser::state::NodeDetail;
    use std::collections::VecDeque;

    let mut s = BrowserState::new();
    // root PG (0), proc (1), conn in→proc (2), conn proc→out (3).
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![1, 2, 3],
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
    s.nodes.push(TreeNode {
        parent: Some(0),
        children: vec![],
        kind: NodeKind::Processor,
        id: "proc".into(),
        group_id: "root".into(),
        name: "proc".into(),
        status_summary: NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });
    for (id, src, dst) in [("c-in", "src", "proc"), ("c-out", "proc", "dst")] {
        s.nodes.push(TreeNode {
            parent: Some(0),
            children: vec![],
            kind: NodeKind::Connection,
            id: id.into(),
            group_id: "root".into(),
            name: id.into(),
            status_summary: NodeStatusSummary::Connection {
                fill_percent: 0,
                flow_files_queued: 0,
                queued_display: "0".into(),
                source_id: src.into(),
                source_name: src.into(),
                destination_id: dst.into(),
                destination_name: dst.into(),
            },
            parameter_context_ref: None,
        });
    }
    // Expand the root PG so children appear in visible.
    s.expanded.insert(0);
    crate::view::browser::state::rebuild_visible(&mut s);
    // Select the processor (arena idx 1). Need the correct visible index.
    s.selected = s.visible.iter().position(|&i| i == 1).unwrap();
    // A minimal ProcessorDetail so section_len can dispatch.
    s.details.insert(
        1,
        NodeDetail::Processor(ProcessorDetail {
            id: "proc".into(),
            name: "proc".into(),
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
    let ring: VecDeque<crate::client::BulletinSnapshot> = VecDeque::new();
    assert_eq!(s.section_len(DetailSection::Connections, &ring), 2);
}

#[test]
fn properties_modal_state_defaults_selected_to_zero() {
    let s = PropertiesModalState::new(42);
    assert_eq!(s.arena_idx, 42);
    assert_eq!(s.selected, 0);
}

#[test]
fn property_rows_marks_uuid_values_that_resolve() {
    let mut s = BrowserState::new();
    // Seed a CS node at arena index 0 so its id can be a resolvable UUID.
    let cs_uuid = "7f3e1c22-1111-4444-8888-abcdef012345".to_string();
    s.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: crate::client::NodeKind::ControllerService,
        id: cs_uuid.clone(),
        group_id: "root".into(),
        name: "fixture-json-reader".into(),
        status_summary: crate::client::NodeStatusSummary::ControllerService {
            state: "ENABLED".into(),
        },
        parameter_context_ref: None,
    });

    let props = vec![
        ("Log Level".to_string(), "info".to_string()),
        ("Record Reader".to_string(), cs_uuid.clone()),
        ("Record Reader Alt".to_string(), "not-a-uuid".to_string()),
    ];

    let rows = property_rows(&s, "", &props);
    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].resolves_to, None);
    assert_eq!(rows[1].resolves_to, Some(0));
    assert_eq!(rows[2].resolves_to, None);
}

#[test]
fn rebuild_arena_from_cluster_uses_snapshot_root_pg() {
    // Task 6: Browser rebuilds its arena from `ClusterSnapshot`
    // instead of a dedicated tree fetch. A minimal snapshot with a
    // Ready `root_pg_status` (containing one root PG node) and a
    // default-empty `controller_services` must populate
    // `state.browser.nodes` with the root PG.
    use crate::cluster::snapshot::{ClusterSnapshot, EndpointState, FetchMeta};
    use std::time::{Duration, Instant};

    let meta = FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: Duration::from_millis(10),
        next_interval: Duration::from_secs(10),
    };
    let snap = ClusterSnapshot {
        root_pg_status: EndpointState::Ready {
            data: crate::test_support::tiny_root_pg_status(),
            meta,
        },
        controller_services: EndpointState::Ready {
            data: crate::client::ControllerServicesSnapshot::default(),
            meta,
        },
        ..Default::default()
    };

    let mut state = crate::test_support::fresh_state();
    rebuild_arena_from_cluster(&mut state, &snap);

    let nodes = &state.browser.nodes;
    assert_eq!(nodes.len(), 1, "arena must have exactly the root PG node");
    assert_eq!(nodes[0].id, "root");
    assert!(matches!(nodes[0].kind, NodeKind::ProcessGroup));
    assert!(state.flow_index.is_some(), "flow index must be rebuilt");
}

#[test]
fn rebuild_arena_on_loading_snapshot_preserves_prior_arena() {
    // A `Loading` root_pg_status slot (pre-first-fetch) must leave
    // the existing arena untouched — the Browser UI continues
    // rendering whatever it had last instead of blanking out.
    use crate::cluster::snapshot::{ClusterSnapshot, EndpointState, FetchMeta};
    use std::time::{Duration, Instant};

    let meta = FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: Duration::from_millis(10),
        next_interval: Duration::from_secs(10),
    };
    // Seed a Ready snapshot once so the arena has content.
    let seeded_snap = ClusterSnapshot {
        root_pg_status: EndpointState::Ready {
            data: crate::test_support::tiny_root_pg_status(),
            meta,
        },
        ..Default::default()
    };
    let mut state = crate::test_support::fresh_state();
    rebuild_arena_from_cluster(&mut state, &seeded_snap);
    let prior_len = state.browser.nodes.len();
    assert_eq!(prior_len, 1);

    // Now re-run against a default (all-Loading) snapshot.
    let empty = ClusterSnapshot::default();
    rebuild_arena_from_cluster(&mut state, &empty);
    assert_eq!(
        state.browser.nodes.len(),
        prior_len,
        "Loading snapshot must not clear existing arena"
    );
}

#[test]
fn rebuild_arena_is_idempotent_for_same_inputs() {
    // Two consecutive rebuilds against the same snapshot must
    // produce the same arena (no double-splicing of CS members, no
    // stale residue).
    use crate::cluster::snapshot::{ClusterSnapshot, EndpointState, FetchMeta};
    use std::time::{Duration, Instant};

    let meta = FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: Duration::from_millis(10),
        next_interval: Duration::from_secs(10),
    };
    let snap = ClusterSnapshot {
        root_pg_status: EndpointState::Ready {
            data: crate::test_support::tiny_root_pg_status(),
            meta,
        },
        controller_services: EndpointState::Ready {
            data: crate::test_support::tiny_controller_services(vec![
                crate::client::ControllerServiceMember {
                    id: "cs-1".into(),
                    name: "fixture-reader".into(),
                    state: "ENABLED".into(),
                    parent_group_id: "root".into(),
                },
            ]),
            meta,
        },
        ..Default::default()
    };

    let mut state = crate::test_support::fresh_state();
    rebuild_arena_from_cluster(&mut state, &snap);
    let first_len = state.browser.nodes.len();

    rebuild_arena_from_cluster(&mut state, &snap);
    assert_eq!(
        state.browser.nodes.len(),
        first_len,
        "second rebuild must not add duplicate CS rows"
    );
    // Exactly one CS row in the arena.
    let cs_count = state
        .browser
        .nodes
        .iter()
        .filter(|n| matches!(n.kind, NodeKind::ControllerService))
        .count();
    assert_eq!(cs_count, 1, "exactly one CS member spliced");
}

#[test]
fn version_control_for_returns_summary_when_present() {
    use crate::cluster::snapshot::{
        ClusterSnapshot, EndpointState, FetchMeta, VersionControlMap, VersionControlSummary,
    };
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

    let mut snap = ClusterSnapshot::default();
    let mut map = VersionControlMap::default();
    map.by_pg_id.insert(
        "pg-1".into(),
        VersionControlSummary {
            state: VersionControlInformationDtoState::LocallyModifiedAndStale,
            registry_name: Some("ops".into()),
            bucket_name: Some("flows".into()),
            branch: None,
            flow_id: Some("f-1".into()),
            flow_name: Some("ingest".into()),
            version: Some("3".into()),
            state_explanation: None,
        },
    );
    snap.version_control = EndpointState::Ready {
        data: map,
        meta: FetchMeta {
            fetched_at: std::time::Instant::now(),
            fetch_duration: std::time::Duration::from_millis(10),
            next_interval: std::time::Duration::from_secs(30),
        },
    };
    assert_eq!(
        BrowserState::version_control_for(&snap, "pg-1")
            .unwrap()
            .state,
        VersionControlInformationDtoState::LocallyModifiedAndStale
    );
    assert!(BrowserState::version_control_for(&snap, "pg-unknown").is_none());
}

#[test]
fn version_control_for_returns_none_when_endpoint_loading() {
    use crate::cluster::snapshot::ClusterSnapshot;
    let snap = ClusterSnapshot::default();
    assert!(BrowserState::version_control_for(&snap, "pg-1").is_none());
}

#[test]
fn redraw_version_control_stamps_pg_entries_only() {
    use crate::client::NodeKind;
    use crate::cluster::snapshot::{
        EndpointState, FetchMeta, VersionControlMap, VersionControlSummary,
    };
    use crate::view::browser::state::{TreeNode, build_flow_index};
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

    // Build an AppState and seed the browser arena with one PG and one
    // Processor so the flow index has both kinds.
    let mut s = crate::test_support::fresh_state();
    s.browser.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: String::new(),
        name: "ingest".into(),
        status_summary: crate::client::NodeStatusSummary::ProcessGroup {
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
        kind: NodeKind::Processor,
        id: "proc-a".into(),
        group_id: "pg-1".into(),
        name: "EnrichRecord".into(),
        status_summary: crate::client::NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });
    s.flow_index = Some(build_flow_index(&s.browser));

    // Build a cluster snapshot where pg-1 is Stale.
    let mut map = VersionControlMap::default();
    map.by_pg_id.insert(
        "pg-1".into(),
        VersionControlSummary {
            state: VersionControlInformationDtoState::Stale,
            registry_name: None,
            bucket_name: None,
            branch: None,
            flow_id: None,
            flow_name: None,
            version: None,
            state_explanation: None,
        },
    );
    let snap = crate::cluster::snapshot::ClusterSnapshot {
        version_control: EndpointState::Ready {
            data: map,
            meta: FetchMeta {
                fetched_at: std::time::Instant::now(),
                fetch_duration: std::time::Duration::from_millis(0),
                next_interval: std::time::Duration::from_secs(30),
            },
        },
        ..crate::cluster::snapshot::ClusterSnapshot::default()
    };

    crate::view::browser::state::redraw_version_control(&mut s, &snap);

    let idx = s.flow_index.as_ref().unwrap();
    let pg_entry = idx
        .entries
        .iter()
        .find(|e| e.kind == NodeKind::ProcessGroup)
        .unwrap();
    let proc_entry = idx
        .entries
        .iter()
        .find(|e| e.kind == NodeKind::Processor)
        .unwrap();
    assert_eq!(
        pg_entry.version_state,
        Some(VersionControlInformationDtoState::Stale)
    );
    assert_eq!(proc_entry.version_state, None);
}

#[test]
fn redraw_version_control_clears_when_endpoint_loading() {
    use crate::client::NodeKind;
    use crate::view::browser::state::{TreeNode, build_flow_index};

    let mut s = crate::test_support::fresh_state();
    s.browser.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::ProcessGroup,
        id: "pg-1".into(),
        group_id: String::new(),
        name: "ingest".into(),
        status_summary: crate::client::NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    s.flow_index = Some(build_flow_index(&s.browser));

    // Pre-stamp a value to prove redraw clears it on Loading.
    let entries = &mut s.flow_index.as_mut().unwrap().entries;
    entries[0].version_state =
        Some(nifi_rust_client::dynamic::types::VersionControlInformationDtoState::Stale);

    let snap = crate::cluster::snapshot::ClusterSnapshot::default();
    crate::view::browser::state::redraw_version_control(&mut s, &snap);

    let idx = s.flow_index.as_ref().unwrap();
    let pg_entry = &idx.entries[0];
    assert_eq!(pg_entry.version_state, None);
}

#[test]
fn redraw_version_control_no_op_when_flow_index_absent() {
    let mut s = crate::test_support::fresh_state();
    // No flow_index set yet; function must not panic.
    let snap = crate::cluster::snapshot::ClusterSnapshot::default();
    crate::view::browser::state::redraw_version_control(&mut s, &snap);
    assert!(s.flow_index.is_none());
}

fn seed_one_pg(state: &mut BrowserState, id: &str, name: &str) {
    state.nodes.push(crate::view::browser::state::TreeNode {
        parent: None,
        children: vec![],
        kind: crate::client::NodeKind::ProcessGroup,
        id: id.into(),
        group_id: String::new(),
        name: name.into(),
        status_summary: crate::client::NodeStatusSummary::ProcessGroup {
            running: 0,
            stopped: 0,
            invalid: 0,
            disabled: 0,
        },
        parameter_context_ref: None,
    });
    state.visible.push(0);
    state.selected = 0;
}

fn snapshot_with_versioned_pg(
    pg_id: &str,
    state: nifi_rust_client::dynamic::types::VersionControlInformationDtoState,
) -> crate::cluster::snapshot::ClusterSnapshot {
    use crate::cluster::snapshot::{
        ClusterSnapshot, EndpointState, FetchMeta, VersionControlMap, VersionControlSummary,
    };
    let mut map = VersionControlMap::default();
    map.by_pg_id.insert(
        pg_id.into(),
        VersionControlSummary {
            state,
            registry_name: Some("ops".into()),
            bucket_name: Some("flows".into()),
            branch: None,
            flow_id: Some("f-1".into()),
            flow_name: Some("ingest".into()),
            version: Some("3".into()),
            state_explanation: None,
        },
    );
    ClusterSnapshot {
        version_control: EndpointState::Ready {
            data: map,
            meta: FetchMeta {
                fetched_at: std::time::Instant::now(),
                fetch_duration: std::time::Duration::from_millis(0),
                next_interval: std::time::Duration::from_secs(30),
            },
        },
        ..ClusterSnapshot::default()
    }
}

#[test]
fn open_version_control_modal_captures_pg_id_and_identity() {
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

    let mut state = BrowserState::new();
    seed_one_pg(&mut state, "pg-1", "ingest");
    let snap = snapshot_with_versioned_pg("pg-1", VersionControlInformationDtoState::Stale);

    state.open_version_control_modal(&snap);

    let modal = state.version_modal.as_ref().unwrap();
    assert_eq!(modal.pg_id, "pg-1");
    assert_eq!(modal.pg_name, "ingest");
    assert!(matches!(
        modal.differences,
        crate::view::browser::state::VersionControlDifferenceLoad::Pending
    ));
    assert!(!modal.show_environmental);
    assert!(modal.identity.is_some());
    assert_eq!(
        modal.identity.as_ref().unwrap().state,
        VersionControlInformationDtoState::Stale
    );
}

#[test]
fn open_version_control_modal_no_op_on_unversioned_pg() {
    use crate::cluster::snapshot::ClusterSnapshot;
    let mut state = BrowserState::new();
    seed_one_pg(&mut state, "pg-1", "ingest");
    state.open_version_control_modal(&ClusterSnapshot::default());
    assert!(state.version_modal.is_none());
}

#[test]
fn close_version_control_modal_clears_state() {
    use crate::view::browser::state::VersionControlModalState;
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.close_version_control_modal();
    assert!(state.version_modal.is_none());
}

#[test]
fn modal_loaded_event_with_mismatched_pg_id_is_ignored() {
    use crate::client::FlowComparisonGrouped;
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.apply_version_control_modal_loaded(
        "pg-OTHER".into(),
        None,
        FlowComparisonGrouped::default(),
    );
    assert!(matches!(
        state.version_modal.as_ref().unwrap().differences,
        VersionControlDifferenceLoad::Pending
    ));
}

#[test]
fn modal_loaded_event_with_matching_pg_id_populates_diffs() {
    use crate::client::{ComponentDiffSection, FlowComparisonGrouped, RenderedDifference};
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "proc-a".into(),
            component_name: "UpdateRecord".into(),
            component_type: "Processor".into(),
            display_label: "UpdateRecord".into(),
            differences: vec![RenderedDifference {
                kind: "PROPERTY_CHANGED".into(),
                description: "Record Reader changed".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);
    let modal = state.version_modal.as_ref().unwrap();
    match &modal.differences {
        VersionControlDifferenceLoad::Loaded(sections) => {
            assert_eq!(sections.len(), 1);
            assert_eq!(sections[0].differences.len(), 1);
        }
        other => panic!("expected Loaded, got {:?}", other),
    }
}

#[test]
fn modal_failed_event_with_matching_pg_id_marks_diffs_failed() {
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.apply_version_control_modal_failed("pg-1".into(), "diff fetch HTTP 500".into());

    let modal = state.version_modal.as_ref().expect("modal still open");
    match &modal.differences {
        VersionControlDifferenceLoad::Failed(err) => {
            assert_eq!(err, "diff fetch HTTP 500");
        }
        other => panic!("expected Failed(...), got {:?}", other),
    }
    // Identity is unchanged — failure path doesn't touch it.
    // (Pending tests already cover the initial-identity-None state.)
}

#[test]
fn modal_failed_event_with_mismatched_pg_id_is_ignored() {
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.apply_version_control_modal_failed("pg-OTHER".into(), "ignored failure".into());

    // Mismatched pg_id: modal stays in Pending — failure was for a stale fetch.
    assert!(matches!(
        state.version_modal.as_ref().unwrap().differences,
        VersionControlDifferenceLoad::Pending
    ));
}

#[test]
fn modal_failed_after_identity_loaded_preserves_identity() {
    use crate::client::FlowComparisonGrouped;
    use crate::cluster::snapshot::VersionControlSummary;
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));

    // First a successful load establishes the identity.
    let identity = VersionControlSummary {
        state: VersionControlInformationDtoState::Stale,
        registry_name: Some("registry".into()),
        bucket_name: Some("bucket".into()),
        branch: Some("main".into()),
        flow_id: Some("flow-1".into()),
        flow_name: Some("ingest".into()),
        version: Some("1".into()),
        state_explanation: None,
    };
    state.apply_version_control_modal_loaded(
        "pg-1".into(),
        Some(identity),
        FlowComparisonGrouped::default(),
    );

    // Then a later failure event lands; identity should survive.
    state.apply_version_control_modal_failed("pg-1".into(), "refresh failed".into());

    let modal = state.version_modal.as_ref().unwrap();
    assert!(
        modal.identity.is_some(),
        "identity must survive a later failure"
    );
    assert!(matches!(
        modal.differences,
        VersionControlDifferenceLoad::Failed(_)
    ));
}

#[test]
fn toggle_environmental_flips_flag() {
    use crate::view::browser::state::VersionControlModalState;
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.toggle_environmental();
    assert!(state.version_modal.as_ref().unwrap().show_environmental);
    state.toggle_environmental();
    assert!(!state.version_modal.as_ref().unwrap().show_environmental);
}

#[test]
fn modal_search_open_initializes_state() {
    use crate::view::browser::state::VersionControlModalState;
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    state.version_modal_search_open();
    let modal = state.version_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert!(search.input_active);
    assert!(!search.committed);
    assert!(search.query.is_empty());
}

#[test]
fn modal_search_commit_with_query_advances_state() {
    use crate::client::{ComponentDiffSection, FlowComparisonGrouped, RenderedDifference};
    use crate::view::browser::state::VersionControlModalState;

    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    // Load some content so search has a body to match against.
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "proc-a".into(),
            component_name: "UpdateRecord".into(),
            component_type: "Processor".into(),
            display_label: "UpdateRecord".into(),
            differences: vec![RenderedDifference {
                kind: "PROPERTY_CHANGED".into(),
                description: "Record Reader changed".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);

    state.version_modal_search_open();
    state.version_modal_search_push('R');
    state.version_modal_search_push('e');
    state.version_modal_search_commit();

    let modal = state.version_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert!(!search.input_active);
    assert!(search.committed);
    assert_eq!(search.query, "Re");
    assert!(!search.matches.is_empty());
}

#[test]
fn close_modal_clears_handle_field() {
    use crate::view::browser::state::VersionControlModalState;

    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    // We don't actually spawn a task — the abort path is a no-op when
    // the handle is `None`. The contract being verified is that
    // `close_version_control_modal` clears both the modal and its
    // handle field, leaving the state ready for a fresh open.
    state.close_version_control_modal();
    assert!(state.version_modal.is_none());
    assert!(state.version_modal_handle.is_none());
}

#[test]
fn modal_search_push_appends_to_query_after_open() {
    use crate::client::{ComponentDiffSection, FlowComparisonGrouped, RenderedDifference};
    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "abcdabcd".into(),
            component_name: "X".into(),
            component_type: "Processor".into(),
            display_label: "X".into(),
            differences: vec![RenderedDifference {
                kind: "PROPERTY_CHANGED".into(),
                description: "Record Reader changed".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);

    state.version_modal_search_open();
    state.version_modal_search_push('R');
    state.version_modal_search_push('e');
    let modal = state.version_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert_eq!(search.query, "Re");
    assert!(search.input_active);
    assert!(!search.committed);
}

#[test]
fn modal_loaded_resolves_connection_label_from_arena() {
    use crate::client::{
        ComponentDiffSection, FlowComparisonGrouped, NodeKind, NodeStatusSummary,
        RenderedDifference,
    };
    use crate::view::browser::state::{
        TreeNode, VersionControlDifferenceLoad, VersionControlModalState,
    };

    let mut state = BrowserState::new();
    // Seed an arena with a connection whose source/destination names
    // we want resolved.
    state.nodes.push(TreeNode {
        parent: None,
        children: vec![],
        kind: NodeKind::Connection,
        id: "conn-xyz".into(),
        group_id: "pg-1".into(),
        name: String::new(),
        status_summary: NodeStatusSummary::Connection {
            fill_percent: 0,
            flow_files_queued: 0,
            queued_display: String::new(),
            source_id: "src-uuid".into(),
            source_name: "GenerateFlowFile".into(),
            destination_id: "dst-uuid".into(),
            destination_name: "LogAttribute".into(),
        },
        parameter_context_ref: None,
    });

    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "conn-xyz".into(),
            component_name: String::new(),
            component_type: "Connection".into(),
            display_label: String::new(),
            differences: vec![RenderedDifference {
                kind: "COMPONENT_REMOVED".into(),
                description: "Connection was removed".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);

    let modal = state.version_modal.as_ref().unwrap();
    match &modal.differences {
        VersionControlDifferenceLoad::Loaded(sections) => {
            assert_eq!(sections[0].display_label, "GenerateFlowFile → LogAttribute");
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn modal_loaded_falls_back_for_unresolvable_connection() {
    use crate::client::{ComponentDiffSection, FlowComparisonGrouped, RenderedDifference};
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};

    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "missing-conn".into(),
            component_name: String::new(),
            component_type: "Connection".into(),
            display_label: String::new(),
            differences: vec![RenderedDifference {
                kind: "COMPONENT_REMOVED".into(),
                description: "Connection was removed".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);

    let modal = state.version_modal.as_ref().unwrap();
    match &modal.differences {
        VersionControlDifferenceLoad::Loaded(sections) => {
            assert_eq!(sections[0].display_label, "(unnamed connection)");
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

#[test]
fn modal_loaded_falls_back_for_unnamed_processor() {
    use crate::client::{ComponentDiffSection, FlowComparisonGrouped, RenderedDifference};
    use crate::view::browser::state::{VersionControlDifferenceLoad, VersionControlModalState};

    let mut state = BrowserState::new();
    state.version_modal = Some(VersionControlModalState::pending(
        "pg-1".into(),
        "ingest".into(),
        None,
    ));
    let grouped = FlowComparisonGrouped {
        sections: vec![ComponentDiffSection {
            component_id: "proc-z".into(),
            component_name: String::new(),
            component_type: "Processor".into(),
            display_label: String::new(),
            differences: vec![RenderedDifference {
                kind: "COMPONENT_ADDED".into(),
                description: "Processor was added".into(),
                environmental: false,
            }],
        }],
    };
    state.apply_version_control_modal_loaded("pg-1".into(), None, grouped);

    let modal = state.version_modal.as_ref().unwrap();
    match &modal.differences {
        VersionControlDifferenceLoad::Loaded(sections) => {
            assert_eq!(sections[0].display_label, "(unnamed)");
        }
        other => panic!("expected Loaded, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// apply_parameter_context_bindings / parameter_context_ref_for
// ---------------------------------------------------------------------------

fn fresh_browser_with_two_pgs() -> BrowserState {
    let mut state = BrowserState::default();
    for (id, name) in [("pg-a", "alpha"), ("pg-b", "beta")] {
        state.nodes.push(crate::view::browser::state::TreeNode {
            parent: None,
            children: vec![],
            kind: crate::client::NodeKind::ProcessGroup,
            id: id.into(),
            group_id: String::new(),
            name: name.into(),
            status_summary: crate::client::NodeStatusSummary::ProcessGroup {
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
            },
            parameter_context_ref: None,
        });
    }
    state
}

#[test]
fn apply_parameter_context_bindings_stamps_pg_entries() {
    use crate::cluster::snapshot::{ParameterContextBindingsMap, ParameterContextRef};
    use std::collections::BTreeMap;

    let mut state = fresh_browser_with_two_pgs();
    // Sanity: no PG has a ref yet.
    assert!(state.parameter_context_ref_for("pg-a").is_none());
    assert!(state.parameter_context_ref_for("pg-b").is_none());

    let mut by_pg_id: BTreeMap<String, Option<ParameterContextRef>> = BTreeMap::new();
    by_pg_id.insert(
        "pg-a".to_string(),
        Some(ParameterContextRef {
            id: "ctx-1".into(),
            name: "ctx-prod".into(),
        }),
    );
    by_pg_id.insert("pg-b".to_string(), None);
    state.apply_parameter_context_bindings(&ParameterContextBindingsMap { by_pg_id });

    assert_eq!(
        state
            .parameter_context_ref_for("pg-a")
            .map(|r| r.name.clone()),
        Some("ctx-prod".into())
    );
    assert!(state.parameter_context_ref_for("pg-b").is_none());
}

#[test]
fn apply_parameter_context_bindings_does_not_touch_non_pg_nodes() {
    use crate::cluster::snapshot::{ParameterContextBindingsMap, ParameterContextRef};
    use std::collections::BTreeMap;

    let mut state = fresh_browser_with_two_pgs();
    // Push a Processor node. Its id ("proc-x") is deliberately included in
    // the bindings map below so that the only thing preventing a stamp is
    // the kind guard (`NodeKind::ProcessGroup`), not the absence of the id
    // from the map. If the kind guard were removed the test would fail.
    state.nodes.push(crate::view::browser::state::TreeNode {
        parent: Some(0),
        children: vec![],
        kind: crate::client::NodeKind::Processor,
        id: "proc-x".into(),
        group_id: "pg-a".into(),
        name: "Proc".into(),
        status_summary: crate::client::NodeStatusSummary::Processor {
            run_status: "Running".into(),
        },
        parameter_context_ref: None,
    });

    let mut by_pg_id: BTreeMap<String, Option<ParameterContextRef>> = BTreeMap::new();
    by_pg_id.insert(
        "pg-a".to_string(),
        Some(ParameterContextRef {
            id: "ctx-1".into(),
            name: "ctx-prod".into(),
        }),
    );
    // Also add an entry for "proc-x" so the kind guard is the sole
    // discriminant — without it the Processor would be stamped.
    by_pg_id.insert(
        "proc-x".to_string(),
        Some(ParameterContextRef {
            id: "ctx-2".into(),
            name: "ctx-dev".into(),
        }),
    );
    state.apply_parameter_context_bindings(&ParameterContextBindingsMap { by_pg_id });

    // The processor node must not have been stamped with a ref, because
    // `apply_parameter_context_bindings` only touches ProcessGroup nodes.
    let proc_node = state.nodes.iter().find(|n| n.id == "proc-x").unwrap();
    assert!(proc_node.parameter_context_ref.is_none());
}

#[test]
fn open_action_history_modal_sets_pending_state() {
    let mut state = BrowserState::default();
    state.open_action_history_modal("proc-1".into(), "FetchKafka".into());
    let m = state.action_history_modal.as_ref().expect("modal opened");
    assert_eq!(m.source_id, "proc-1");
    assert_eq!(m.component_label, "FetchKafka");
    assert!(m.loading);
    assert!(m.actions.is_empty());
}

#[test]
fn close_action_history_modal_clears_state_and_aborts_handle() {
    let mut state = BrowserState::default();
    state.open_action_history_modal("proc-1".into(), "X".into());
    // Simulate a worker handle by spawning a no-op task on a
    // current-thread runtime.
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
                state.action_history_modal_handle = Some(crate::app::worker::AbortOnDrop::new(h));
                state.close_action_history_modal();
                assert!(state.action_history_modal.is_none());
                assert!(state.action_history_modal_handle.is_none());
            })
            .await;
    });
}

#[test]
fn open_action_history_modal_replaces_open_modal() {
    let mut state = BrowserState::default();
    state.open_action_history_modal("proc-1".into(), "A".into());
    state.open_action_history_modal("proc-2".into(), "B".into());
    let m = state.action_history_modal.as_ref().unwrap();
    assert_eq!(m.source_id, "proc-2");
    assert_eq!(m.component_label, "B");
}

#[test]
fn open_sparkline_for_selection_replaces_state() {
    use crate::client::history::ComponentKind;
    let mut s = BrowserState::default();
    s.open_sparkline_for_selection(ComponentKind::Processor, "p-1".into());
    let sparkline = s.sparkline.as_ref().expect("opened");
    assert!(matches!(sparkline.kind, ComponentKind::Processor));
    assert_eq!(sparkline.id, "p-1");
    assert!(sparkline.series.is_none());

    // Replace with a different selection.
    s.open_sparkline_for_selection(ComponentKind::ProcessGroup, "pg-1".into());
    let sparkline = s.sparkline.as_ref().expect("replaced");
    assert!(matches!(sparkline.kind, ComponentKind::ProcessGroup));
    assert_eq!(sparkline.id, "pg-1");
}

#[test]
fn close_sparkline_clears_state_and_aborts_handle() {
    use crate::client::history::ComponentKind;
    let mut s = BrowserState::default();
    s.open_sparkline_for_selection(ComponentKind::Processor, "p-1".into());

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
                s.sparkline_handle = Some(crate::app::worker::AbortOnDrop::new(h));
                s.close_sparkline();
                assert!(s.sparkline.is_none());
                assert!(s.sparkline_handle.is_none());
            })
            .await;
    });
}

#[test]
fn open_sparkline_for_selection_aborts_previous_handle() {
    use crate::client::history::ComponentKind;
    let mut s = BrowserState::default();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                s.open_sparkline_for_selection(ComponentKind::Processor, "p-1".into());
                let h1 = tokio::task::spawn_local(async {
                    tokio::time::sleep(std::time::Duration::from_secs(60)).await;
                });
                s.sparkline_handle = Some(crate::app::worker::AbortOnDrop::new(h1));

                // Open a new selection — old handle must be aborted, slot cleared.
                s.open_sparkline_for_selection(ComponentKind::Processor, "p-2".into());
                assert!(
                    s.sparkline_handle.is_none(),
                    "open replacement must abort and clear the old handle"
                );
                assert_eq!(s.sparkline.as_ref().unwrap().id, "p-2");
            })
            .await;
    });
}
