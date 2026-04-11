//! Pure state for the Browser tab.
//!
//! Everything here is synchronous and no-I/O. The tokio worker in
//! `super::worker` is the only place that touches the network. Navigation
//! helpers, `apply_node_detail`, and the detail-dispatch side-channel
//! land in Tasks 9/10.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use tokio::sync::{mpsc, oneshot};

use crate::client::browser::{
    ConnectionDetail, ControllerServiceDetail, NodeKind, NodeStatusSummary, ProcessGroupDetail,
    ProcessorDetail, RawNode, RecursiveSnapshot,
};

#[derive(Debug, Default)]
pub struct BrowserState {
    pub nodes: Vec<TreeNode>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub expanded: HashSet<usize>,
    pub details: HashMap<usize, NodeDetail>,
    pub pending_detail: Option<usize>,
    pub last_tree_fetched_at: Option<SystemTime>,
    /// Populated by the `WorkerRegistry` when the Browser worker is
    /// spawned. Cleared back to `None` on tab-switch-out so reducer
    /// pushes become no-ops. Task 13 wires this.
    pub detail_tx: Option<mpsc::UnboundedSender<DetailRequest>>,
    /// One-shot force-tick channel; a fresh sender on every worker spawn.
    /// `r` pushes a unit on it; the worker wakes and fetches immediately.
    /// Task 15 wires this.
    pub force_tick_tx: Option<oneshot::Sender<()>>,
}

impl BrowserState {
    pub fn new() -> Self {
        Self::default()
    }
}

#[derive(Debug, Clone)]
pub struct TreeNode {
    pub parent: Option<usize>,
    pub children: Vec<usize>,
    pub kind: NodeKind,
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub status_summary: NodeStatusSummary,
}

#[derive(Debug, Clone)]
pub enum NodeDetail {
    ProcessGroup(ProcessGroupDetail),
    Processor(ProcessorDetail),
    Connection(ConnectionDetail),
    ControllerService(ControllerServiceDetail),
}

#[derive(Debug, Clone)]
pub struct DetailRequest {
    pub arena_idx: usize,
    pub kind: NodeKind,
    pub id: String,
}

/// Fold a full recursive tree snapshot into the state. Preserves
/// expansion and selection across arena rebuilds by matching on
/// `(id, kind)`. Drops detail entries for nodes that are gone.
pub fn apply_tree_snapshot(state: &mut BrowserState, snap: RecursiveSnapshot) {
    // 1) Record previous keys for retranslation.
    let prev_selected_key: Option<(String, NodeKind)> = state
        .visible
        .get(state.selected)
        .and_then(|&arena_idx| state.nodes.get(arena_idx))
        .map(|n| (n.id.clone(), n.kind));
    let prev_expanded_keys: HashSet<(String, NodeKind)> = state
        .expanded
        .iter()
        .filter_map(|&idx| state.nodes.get(idx))
        .map(|n| (n.id.clone(), n.kind))
        .collect();
    let prev_detail_keys: HashMap<(String, NodeKind), NodeDetail> = state
        .details
        .iter()
        .filter_map(|(idx, d)| {
            state
                .nodes
                .get(*idx)
                .map(|n| ((n.id.clone(), n.kind), d.clone()))
        })
        .collect();
    let is_first_snapshot = state.nodes.is_empty();

    // 2) Rebuild the arena.
    let mut nodes: Vec<TreeNode> = Vec::with_capacity(snap.nodes.len());
    for RawNode {
        parent_idx,
        kind,
        id,
        group_id,
        name,
        status_summary,
    } in snap.nodes.into_iter()
    {
        nodes.push(TreeNode {
            parent: parent_idx,
            children: Vec::new(),
            kind,
            id,
            group_id,
            name,
            status_summary,
        });
    }
    // Fill each parent's children list in arena order.
    for i in 0..nodes.len() {
        if let Some(p) = nodes[i].parent {
            nodes[p].children.push(i);
        }
    }

    // 3) Translate expansion set by (id, kind).
    let mut new_expanded: HashSet<usize> = HashSet::new();
    for (new_idx, n) in nodes.iter().enumerate() {
        if prev_expanded_keys.contains(&(n.id.clone(), n.kind)) {
            new_expanded.insert(new_idx);
        }
    }

    // 4) First-snapshot seed: auto-expand the root PG so its children
    //    become visible immediately.
    if is_first_snapshot && !nodes.is_empty() {
        new_expanded.insert(0);
    }

    // 5) Translate details.
    let mut new_details: HashMap<usize, NodeDetail> = HashMap::new();
    for (new_idx, n) in nodes.iter().enumerate() {
        if let Some(d) = prev_detail_keys.get(&(n.id.clone(), n.kind)) {
            new_details.insert(new_idx, d.clone());
        }
    }

    // 6) Commit the new arena, then rebuild visible + translate selection.
    state.nodes = nodes;
    state.expanded = new_expanded;
    state.details = new_details;
    state.pending_detail = None;
    state.last_tree_fetched_at = Some(snap.fetched_at);

    rebuild_visible(state);

    // 7) Translate selection. If the previously-selected key still
    //    exists and is visible, find its new visible index. Else
    //    selection falls back to 0 (which is root PG after step 4).
    let new_selected: Option<usize> = prev_selected_key.as_ref().and_then(|(id, kind)| {
        state.visible.iter().position(|&arena_idx| {
            let node = &state.nodes[arena_idx];
            node.id == *id && node.kind == *kind
        })
    });
    state.selected = new_selected.unwrap_or(0);
    if state.visible.is_empty() {
        state.selected = 0;
    } else if state.selected >= state.visible.len() {
        state.selected = state.visible.len() - 1;
    }
}

/// Rebuild `visible` by walking the arena in depth-first tree order,
/// including a PG's children only when the PG is in `expanded`.
pub fn rebuild_visible(state: &mut BrowserState) {
    state.visible.clear();
    // Root node(s): any node with no parent. In practice there is exactly
    // one (the root PG), but we tolerate multiples for robustness.
    let roots: Vec<usize> = (0..state.nodes.len())
        .filter(|&i| state.nodes[i].parent.is_none())
        .collect();
    for r in roots {
        push_visible_subtree(state, r);
    }
}

fn push_visible_subtree(state: &mut BrowserState, idx: usize) {
    state.visible.push(idx);
    let expanded = state.expanded.contains(&idx);
    if !expanded {
        return;
    }
    // Render children in arena order (walker in `client::browser`
    // already appends PGs/processors/connections/ports in the right
    // display order, so arena order == display order).
    let children = state.nodes[idx].children.clone();
    for c in children {
        push_visible_subtree(state, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::browser::{NodeKind, NodeStatusSummary, RawNode};
    use std::time::UNIX_EPOCH;

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
        // Root (0), gen (1), c1 (2), ingest (3) visible (ingest collapsed).
        assert_eq!(s.visible, vec![0, 1, 2, 3]);
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
        assert_eq!(s.visible, vec![0, 1, 2, 3, 4]);

        // Re-apply the same snapshot — indices stay the same, but the
        // retranslation path still runs.
        apply_tree_snapshot(&mut s, demo_snap());
        assert!(s.expanded.contains(&3));
        assert_eq!(s.visible, vec![0, 1, 2, 3, 4]);
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
}
