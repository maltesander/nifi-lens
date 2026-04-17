//! Pure state for the Browser tab.
//!
//! Everything here is synchronous and no-I/O. The tokio worker in
//! `super::worker` is the only place that touches the network. Navigation
//! helpers, `apply_node_detail`, and the detail-dispatch side-channel
//! land in Tasks 9/10.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use tokio::sync::{mpsc, oneshot};

use crate::app::navigation::ListNavigation;
use crate::client::browser::{
    ConnectionDetail, ControllerServiceDetail, FolderKind, NodeKind, NodeStatusSummary,
    ProcessGroupDetail, ProcessorDetail, RawNode, RecursiveSnapshot,
};

/// Rolled-up health color for a Process Group's tree marker glyph.
/// Red beats Yellow beats Green: any descendant processor with
/// `INVALID` run-state promotes to Red; else any with `STOPPED`
/// promotes to Yellow; else Green.
///
/// Ports, Connections, Controller Services do not contribute.
/// This is a shallow-semantic rollup driving the tree's
/// at-a-glance marker color; it does not consider bulletin
/// severity or validation errors on non-processor nodes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PgHealth {
    Green,
    Yellow,
    Red,
}

/// Summary of a direct-child Process Group for the PG detail
/// pane's "Child groups" section. Pulls pre-computed counts from
/// the arena's `NodeStatusSummary::ProcessGroup` variant — no
/// extra API calls required.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChildPgSummary {
    pub id: String,
    pub name: String,
    pub running: u32,
    pub stopped: u32,
    pub invalid: u32,
    pub disabled: u32,
}

/// Named detail sub-sections that can hold keyboard focus.
///
/// This is a closed set — adding a new variant requires updating
/// `DetailSections::for_node`, `DetailSections::for_node_detail`, and
/// the render leaves that draw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetailSection {
    Properties,
    ValidationErrors,
    RecentBulletins,
    ControllerServices,
    ChildGroups,
    ReferencingComponents,
    Endpoints,
}

/// Per-node-kind list of focusable sections, in cycle order.
///
/// Returning a `&'static` slice keeps the per-call cost zero and
/// makes the "no focusable sections" case an `.is_empty()` check.
#[derive(Debug, Clone, Copy)]
pub struct DetailSections(pub &'static [DetailSection]);

impl DetailSections {
    /// Base section list — does not include `ValidationErrors`.
    /// Use `for_node_detail` when the presence of validation errors is known.
    pub fn for_node(kind: crate::client::NodeKind) -> Self {
        use crate::client::NodeKind as NK;
        match kind {
            NK::Processor => {
                DetailSections(&[DetailSection::Properties, DetailSection::RecentBulletins])
            }
            NK::ControllerService => DetailSections(&[
                DetailSection::Properties,
                DetailSection::ReferencingComponents,
                DetailSection::RecentBulletins,
            ]),
            NK::ProcessGroup => DetailSections(&[
                DetailSection::ControllerServices,
                DetailSection::ChildGroups,
                DetailSection::RecentBulletins,
            ]),
            NK::InputPort | NK::OutputPort => DetailSections(&[DetailSection::RecentBulletins]),
            NK::Connection => DetailSections(&[DetailSection::Endpoints]),
            _ => DetailSections(&[]),
        }
    }

    /// Section list that conditionally includes `ValidationErrors` between
    /// `Properties` and `RecentBulletins` when `has_validation` is true.
    /// Use this for focus cycling so the section is only reachable when
    /// errors are present.
    pub fn for_node_detail(kind: crate::client::NodeKind, has_validation: bool) -> Self {
        use crate::client::NodeKind as NK;
        match (kind, has_validation) {
            (NK::Processor, true) => DetailSections(&[
                DetailSection::Properties,
                DetailSection::ValidationErrors,
                DetailSection::RecentBulletins,
            ]),
            (NK::ControllerService, true) => DetailSections(&[
                DetailSection::Properties,
                DetailSection::ValidationErrors,
                DetailSection::ReferencingComponents,
                DetailSection::RecentBulletins,
            ]),
            _ => Self::for_node(kind),
        }
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

/// Max number of focusable sections any node kind has — drives the
/// size of the per-section row-cursor array inside `DetailFocus`.
pub const MAX_DETAIL_SECTIONS: usize = 5;

/// Browser tab focus — the cursor is either in the tree (default)
/// or inside one of the detail pane's focusable sub-sections.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum DetailFocus {
    /// Focus is in the PG tree. Default state.
    #[default]
    Tree,
    /// Focus is inside the detail pane.
    Section {
        /// Index into `DetailSections::for_node(current_kind).0`.
        idx: usize,
        /// Row cursor per section index. Slots beyond the current
        /// node's `DetailSections::len()` are unused.
        rows: [usize; MAX_DETAIL_SECTIONS],
        /// Horizontal character offset per section index. Incremented by
        /// `FocusAction::Right`, decremented (saturating) by `FocusAction::Left`.
        x_offsets: [usize; MAX_DETAIL_SECTIONS],
    },
}

#[derive(Debug, Default)]
pub struct BrowserState {
    pub nodes: Vec<TreeNode>,
    pub visible: Vec<usize>,
    pub selected: usize,
    pub expanded: HashSet<usize>,
    pub details: HashMap<usize, NodeDetail>,
    pub pending_detail: Option<usize>,
    /// `true` when a detail request was set via `pending_detail` but
    /// `detail_tx` was `None` (worker not yet spawned). The app loop
    /// re-emits the request once the worker creates `detail_tx`.
    pub pending_detail_unsent: bool,
    pub last_tree_fetched_at: Option<SystemTime>,
    /// Populated by the `WorkerRegistry` when the Browser worker is
    /// spawned. Cleared back to `None` on tab-switch-out so reducer
    /// pushes become no-ops. Task 13 wires this.
    pub detail_tx: Option<mpsc::UnboundedSender<DetailRequest>>,
    /// One-shot force-tick channel; a fresh sender on every worker spawn.
    /// `r` pushes a unit on it; the worker wakes and fetches immediately.
    /// Task 15 wires this.
    pub force_tick_tx: Option<oneshot::Sender<()>>,
    /// Phase 7: which focusable sub-section (if any) holds input focus.
    /// Always reset to `Tree` by `reset_detail_focus`, called from every
    /// selection-mutating method on `BrowserState`.
    pub detail_focus: DetailFocus,
}

/// One segment in the breadcrumb path.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BreadcrumbSegment {
    pub name: String,
    pub arena_idx: usize,
}

/// Cheap canonical-UUID shape check: 36 chars, hyphens at positions 8,
/// 13, 18, 23, and the remaining 32 positions are hex. Case-insensitive.
/// Returns `true` only for RFC-4122-shaped strings; does not validate
/// version or variant bits.
pub(crate) fn is_uuid_shape(s: &str) -> bool {
    if s.len() != 36 {
        return false;
    }
    let bytes = s.as_bytes();
    for (i, b) in bytes.iter().enumerate() {
        let is_hyphen_pos = matches!(i, 8 | 13 | 18 | 23);
        if is_hyphen_pos {
            if *b != b'-' {
                return false;
            }
        } else if !b.is_ascii_hexdigit() {
            return false;
        }
    }
    true
}

/// Result of resolving a string to a known arena node.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedRef {
    pub arena_idx: usize,
    pub kind: crate::client::NodeKind,
    pub name: String,
    pub group_id: String,
}

/// Direction of a connection edge relative to a given processor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    In,
    Out,
}

/// A connection edge touching a specific processor, enriched with display data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectionEdge {
    pub connection_id: String,
    pub connection_name: String,
    pub direction: EdgeDirection,
    pub opposite_id: String,
    pub opposite_name: String,
    pub opposite_group_id: String,
    pub queued_display: String,
}

impl BrowserState {
    pub fn new() -> Self {
        Self::default()
    }

    /// Resolve a string to a known arena node.
    ///
    /// Returns `Some` only when the trimmed string is a canonical UUID
    /// (see `is_uuid_shape`) and matches a `TreeNode.id` present in
    /// `self.nodes`. Linear scan — O(n) on the arena. Called once per
    /// renderable row in `→`-annotated sections; cheap compared with
    /// rendering cost.
    pub fn resolve_id(&self, raw: &str) -> Option<ResolvedRef> {
        let s = raw.trim();
        if !is_uuid_shape(s) {
            return None;
        }
        let (arena_idx, node) = self.nodes.iter().enumerate().find(|(_, n)| n.id == s)?;
        Some(ResolvedRef {
            arena_idx,
            kind: node.kind,
            name: node.name.clone(),
            group_id: node.group_id.clone(),
        })
    }

    /// Called from every selection-changing entry point. Resets detail
    /// focus because the node under the cursor has (potentially) changed.
    fn reset_detail_focus(&mut self) {
        self.detail_focus = DetailFocus::Tree;
    }

    pub fn move_down(&mut self) {
        ListNavigation::move_down(self);
        self.reset_detail_focus();
    }

    pub fn move_up(&mut self) {
        ListNavigation::move_up(self);
        self.reset_detail_focus();
    }

    pub fn page_down(&mut self, by: usize) {
        ListNavigation::page_down(self, by);
        self.reset_detail_focus();
    }

    pub fn page_up(&mut self, by: usize) {
        ListNavigation::page_up(self, by);
        self.reset_detail_focus();
    }

    pub fn goto_first(&mut self) {
        ListNavigation::goto_first(self);
        self.reset_detail_focus();
    }

    pub fn goto_last(&mut self) {
        ListNavigation::goto_last(self);
        self.reset_detail_focus();
    }

    /// `Enter` / `→` behavior. On a collapsed PG, expands and moves
    /// selection to the first child. On an expanded PG, moves to the
    /// first child (drill-in). On a leaf, no-op.
    pub fn enter_selection(&mut self) {
        let Some(&arena_idx) = self.visible.get(self.selected) else {
            return;
        };
        let is_pg = matches!(self.nodes[arena_idx].kind, NodeKind::ProcessGroup);
        if !is_pg {
            return;
        }
        let was_expanded = self.expanded.contains(&arena_idx);
        if !was_expanded {
            self.expanded.insert(arena_idx);
            rebuild_visible(self);
        }
        // Move selection to the first child (if any).
        let first_child = self.nodes[arena_idx].children.first().copied();
        if let Some(child) = first_child
            && let Some(pos) = self.visible.iter().position(|&i| i == child)
        {
            self.selected = pos;
        }
        self.reset_detail_focus();
    }

    /// `Backspace` / `←` behavior. On an expanded PG with its row
    /// selected: collapses. On any other node: moves selection to the
    /// parent.
    pub fn backspace_selection(&mut self) {
        let Some(&arena_idx) = self.visible.get(self.selected) else {
            return;
        };
        let node = &self.nodes[arena_idx];
        let is_expanded_pg =
            matches!(node.kind, NodeKind::ProcessGroup) && self.expanded.contains(&arena_idx);
        if is_expanded_pg {
            self.expanded.remove(&arena_idx);
            rebuild_visible(self);
            // Keep selection on the PG row — find its new visible index.
            if let Some(pos) = self.visible.iter().position(|&i| i == arena_idx) {
                self.selected = pos;
            }
            self.reset_detail_focus();
            return;
        }
        // Otherwise, walk up to parent.
        let parent = node.parent;
        if let Some(p) = parent
            && let Some(pos) = self.visible.iter().position(|&i| i == p)
        {
            self.selected = pos;
        }
        self.reset_detail_focus();
    }

    /// Locate the PG with `group_id` in the arena, expand all ancestors,
    /// rebuild visible, and move selection to that PG's row. Also resets
    /// detail focus to `Tree`. Returns `true` on success, `false` if the
    /// id is unknown.
    ///
    /// Used by `Enter` on a focused Child-groups row in the PG detail pane.
    pub fn drill_into_group(&mut self, group_id: &str) -> bool {
        let Some(target_idx) = self
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.id == group_id)
        else {
            return false;
        };
        // Walk up and expand every ancestor.
        let mut cursor = self.nodes[target_idx].parent;
        while let Some(p) = cursor {
            self.expanded.insert(p);
            cursor = self.nodes[p].parent;
        }
        rebuild_visible(self);
        if let Some(pos) = self.visible.iter().position(|&i| i == target_idx) {
            self.selected = pos;
        }
        self.reset_detail_focus();
        true
    }

    /// Set `pending_detail` to the currently-selected arena index and
    /// push a `DetailRequest` on `detail_tx` when a sender exists.
    pub fn emit_detail_request_for_current_selection(&mut self) {
        let Some(&arena_idx) = self.visible.get(self.selected) else {
            return;
        };
        let node = &self.nodes[arena_idx];
        if matches!(node.kind, NodeKind::Folder(_)) {
            // Folders have no detail pane; mark nothing pending.
            self.pending_detail = None;
            self.pending_detail_unsent = false;
            return;
        }
        self.pending_detail = Some(arena_idx);
        if let Some(tx) = self.detail_tx.as_ref() {
            let _ = tx.send(DetailRequest {
                arena_idx,
                kind: node.kind,
                id: node.id.clone(),
            });
            self.pending_detail_unsent = false;
        } else {
            self.pending_detail_unsent = true;
        }
    }

    /// Build the breadcrumb path from root to the currently selected node.
    /// Returns an empty vec if no node is selected.
    pub fn breadcrumb_segments(&self) -> Vec<BreadcrumbSegment> {
        let Some(&arena_idx) = self.visible.get(self.selected) else {
            return Vec::new();
        };
        let mut segments = Vec::new();
        let mut cursor = Some(arena_idx);
        while let Some(i) = cursor {
            if !matches!(self.nodes[i].kind, NodeKind::Folder(_)) {
                segments.push(BreadcrumbSegment {
                    name: self.nodes[i].name.clone(),
                    arena_idx: i,
                });
            }
            cursor = self.nodes[i].parent;
        }
        segments.reverse();
        segments
    }

    /// Resolve a `group_id` (PG UUID) to a human-readable breadcrumb
    /// path by walking the flow arena upward, e.g. `"noisy-pipeline"` or
    /// `"healthy-pipeline / ingest"`. The root PG name is dropped from
    /// the output because every path would otherwise start with the
    /// same redundant prefix.
    ///
    /// Returns `None` when the PG is not present in the current
    /// snapshot (Browser tab not yet visited, stale id, etc.). Callers
    /// must render their own fallback in that case.
    pub fn pg_path(&self, group_id: &str) -> Option<String> {
        // Find the PG node whose `id` matches `group_id`. A non-PG node
        // with a matching id is a programming error — PGs have unique
        // ids in NiFi.
        let start = self
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.id == group_id)?;
        // Walk parents, collecting names, stopping before the root.
        let mut names: Vec<&str> = Vec::new();
        let mut cursor = Some(start);
        while let Some(idx) = cursor {
            let node = &self.nodes[idx];
            // Stop at the root: a node whose parent is None. Its name
            // is intentionally excluded.
            if node.parent.is_none() {
                break;
            }
            names.push(node.name.as_str());
            cursor = node.parent;
        }
        if names.is_empty() {
            return None;
        }
        names.reverse();
        Some(names.join(" / "))
    }

    /// Compute the rolled-up health color for the PG at `arena_idx`
    /// by walking its descendants in the arena.
    ///
    /// Returns `PgHealth::Red` if any descendant processor has
    /// `run_status == "INVALID"`, else `Yellow` if any has
    /// `"STOPPED"`, else `Green`.
    ///
    /// Safe on any arena index — non-PG indices return `Green`
    /// (a PG's rollup is only asked for PG nodes in practice).
    pub fn pg_health_rollup(&self, arena_idx: usize) -> PgHealth {
        let mut saw_stopped = false;
        // DFS over descendants.
        let mut stack: Vec<usize> = self
            .nodes
            .get(arena_idx)
            .map(|n| n.children.clone())
            .unwrap_or_default();
        while let Some(idx) = stack.pop() {
            let Some(node) = self.nodes.get(idx) else {
                continue;
            };
            if let NodeStatusSummary::Processor { run_status } = &node.status_summary {
                match run_status.to_ascii_uppercase().as_str() {
                    "INVALID" => return PgHealth::Red,
                    "STOPPED" => saw_stopped = true,
                    _ => {}
                }
            }
            stack.extend(node.children.iter().copied());
        }
        if saw_stopped {
            PgHealth::Yellow
        } else {
            PgHealth::Green
        }
    }

    /// Row count for a given detail section on the currently-selected node.
    ///
    /// Used to clamp arrow-key row navigation inside detail focus.
    /// Returns 0 when the section has no data (empty properties, no recent
    /// bulletins for this node) or when no node is selected / detail not
    /// yet loaded.
    pub fn section_len(
        &self,
        section: DetailSection,
        bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    ) -> usize {
        let Some(&arena_idx) = self.visible.get(self.selected) else {
            return 0;
        };
        let Some(detail) = self.details.get(&arena_idx) else {
            return 0;
        };
        match (section, detail) {
            (DetailSection::Properties, NodeDetail::Processor(p)) => p.properties.len(),
            (DetailSection::Properties, NodeDetail::ControllerService(cs)) => cs.properties.len(),
            (DetailSection::ValidationErrors, NodeDetail::Processor(p)) => {
                p.validation_errors.len()
            }
            (DetailSection::ValidationErrors, NodeDetail::ControllerService(cs)) => {
                cs.validation_errors.len()
            }
            (DetailSection::ReferencingComponents, NodeDetail::ControllerService(cs)) => {
                cs.referencing_components.len()
            }
            (DetailSection::RecentBulletins, NodeDetail::ControllerService(cs)) => {
                bulletins.iter().filter(|b| b.source_id == cs.id).count()
            }
            (DetailSection::RecentBulletins, NodeDetail::Processor(_)) => {
                let source_id = &self.nodes[arena_idx].id;
                bulletins
                    .iter()
                    .filter(|b| b.source_id == *source_id)
                    .count()
            }
            (DetailSection::ControllerServices, NodeDetail::ProcessGroup(d)) => {
                d.controller_services.len()
            }
            (DetailSection::ChildGroups, NodeDetail::ProcessGroup(d)) => {
                self.child_process_groups(&d.id).len()
            }
            (DetailSection::RecentBulletins, NodeDetail::ProcessGroup(d)) => {
                let group_id = &d.id;
                bulletins.iter().filter(|b| b.group_id == *group_id).count()
            }
            (DetailSection::RecentBulletins, NodeDetail::Port(p)) => {
                bulletins.iter().filter(|b| b.source_id == p.id).count()
            }
            (DetailSection::Endpoints, NodeDetail::Connection(_)) => 2,
            _ => 0,
        }
    }

    /// Returns the clipboard-ready string for the currently-focused detail
    /// row, or `None` if focus is in the tree, no row is selected, or the
    /// section is empty.
    ///
    /// - Properties rows return the raw value string.
    /// - RecentBulletins rows return the full bulletin message.
    pub fn focused_row_copy_value(
        &self,
        bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    ) -> Option<String> {
        let DetailFocus::Section { idx, rows, .. } = &self.detail_focus else {
            return None;
        };
        let arena_idx = *self.visible.get(self.selected)?;
        let detail = self.details.get(&arena_idx)?;
        let kind = self.nodes[arena_idx].kind;
        let has_validation = match detail {
            NodeDetail::Processor(p) => !p.validation_errors.is_empty(),
            NodeDetail::ControllerService(cs) => !cs.validation_errors.is_empty(),
            _ => false,
        };
        let sections = DetailSections::for_node_detail(kind, has_validation);
        let section = *sections.0.get(*idx)?;
        let row = rows[*idx];
        match (section, detail) {
            (DetailSection::Properties, NodeDetail::Processor(p)) => {
                p.properties.get(row).map(|(_k, v)| v.clone())
            }
            (DetailSection::Properties, NodeDetail::ControllerService(cs)) => {
                cs.properties.get(row).map(|(_k, v)| v.clone())
            }
            (DetailSection::ValidationErrors, NodeDetail::Processor(p)) => {
                p.validation_errors.get(row).cloned()
            }
            (DetailSection::ValidationErrors, NodeDetail::ControllerService(cs)) => {
                cs.validation_errors.get(row).cloned()
            }
            (DetailSection::RecentBulletins, NodeDetail::Processor(_)) => {
                let source_id = &self.nodes[arena_idx].id;
                bulletins
                    .iter()
                    .filter(|b| b.source_id == *source_id)
                    .nth(row)
                    .map(|b| b.message.clone())
            }
            (DetailSection::ControllerServices, NodeDetail::ProcessGroup(d)) => {
                d.controller_services.get(row).map(|cs| cs.id.clone())
            }
            (DetailSection::ChildGroups, NodeDetail::ProcessGroup(d)) => self
                .child_process_groups(&d.id)
                .get(row)
                .map(|k| k.id.clone()),
            (DetailSection::RecentBulletins, NodeDetail::ProcessGroup(d)) => {
                let group_id = &d.id;
                bulletins
                    .iter()
                    .rev()
                    .filter(|b| b.group_id == *group_id)
                    .nth(row)
                    .map(|b| b.message.clone())
            }
            _ => None,
        }
    }

    /// Return the `source_id` of the bulletin row under detail focus,
    /// or `None` if focus is not on a Recent bulletins row.
    ///
    /// Used by the `t` cross-link on the PG detail pane: the PG itself
    /// is not the bulletin source, so the handler must walk the ring to
    /// find the per-row source. For Processor nodes, the nth matching
    /// bulletin's `source_id` equals the processor id — the helper works
    /// there too, so the handler can use it unconditionally.
    pub fn focused_row_source_id(
        &self,
        bulletins: &std::collections::VecDeque<crate::client::BulletinSnapshot>,
    ) -> Option<String> {
        let DetailFocus::Section { idx, rows, .. } = &self.detail_focus else {
            return None;
        };
        let arena_idx = *self.visible.get(self.selected)?;
        let detail = self.details.get(&arena_idx)?;
        let kind = self.nodes[arena_idx].kind;
        let sections = DetailSections::for_node(kind);
        let section = *sections.0.get(*idx)?;
        if section != DetailSection::RecentBulletins {
            return None;
        }
        let row = rows[*idx];
        match detail {
            NodeDetail::Processor(_) => {
                let source_id = &self.nodes[arena_idx].id;
                bulletins
                    .iter()
                    .filter(|b| b.source_id == *source_id)
                    .nth(row)
                    .map(|b| b.source_id.clone())
            }
            NodeDetail::ProcessGroup(d) => bulletins
                .iter()
                .rev()
                .filter(|b| b.group_id == d.id)
                .nth(row)
                .map(|b| b.source_id.clone()),
            _ => None,
        }
    }

    /// List the direct child Process Groups of the PG with the
    /// given `group_id`, in arena order. Non-PG children are
    /// excluded. Returns an empty vec if the PG is not present
    /// in the current snapshot.
    pub fn child_process_groups(&self, group_id: &str) -> Vec<ChildPgSummary> {
        let Some(parent_idx) = self
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.id == group_id)
        else {
            return Vec::new();
        };
        self.nodes[parent_idx]
            .children
            .iter()
            .filter_map(|&child_idx| {
                let child = self.nodes.get(child_idx)?;
                if !matches!(child.kind, NodeKind::ProcessGroup) {
                    return None;
                }
                let (running, stopped, invalid, disabled) = match &child.status_summary {
                    NodeStatusSummary::ProcessGroup {
                        running,
                        stopped,
                        invalid,
                        disabled,
                    } => (*running, *stopped, *invalid, *disabled),
                    _ => (0, 0, 0, 0),
                };
                Some(ChildPgSummary {
                    id: child.id.clone(),
                    name: child.name.clone(),
                    running,
                    stopped,
                    invalid,
                    disabled,
                })
            })
            .collect()
    }

    /// Return every connection edge touching `processor_id`, split into
    /// inbound (processor is the connection's destination) and outbound
    /// (processor is the connection's source). Names for the opposite
    /// endpoint come from the status snapshot; `opposite_group_id` is
    /// resolved via an arena lookup on `opposite_id` and falls back to
    /// the empty string when the opposite endpoint isn't in the current
    /// arena (e.g. remote process group). The `OpenInBrowser` reducer
    /// does not use `group_id`, so the empty fallback is safe.
    pub fn connections_for_processor(&self, processor_id: &str) -> Vec<ConnectionEdge> {
        use crate::client::{NodeKind, NodeStatusSummary};
        let mut edges = Vec::new();
        for node in &self.nodes {
            if !matches!(node.kind, NodeKind::Connection) {
                continue;
            }
            let NodeStatusSummary::Connection {
                source_id,
                source_name,
                destination_id,
                destination_name,
                queued_display,
                ..
            } = &node.status_summary
            else {
                continue;
            };
            let (direction, opposite_id, opposite_name) = if source_id == processor_id {
                (EdgeDirection::Out, destination_id, destination_name)
            } else if destination_id == processor_id {
                (EdgeDirection::In, source_id, source_name)
            } else {
                continue;
            };
            let opposite_group_id = self
                .nodes
                .iter()
                .find(|n| n.id == *opposite_id)
                .map(|n| n.group_id.clone())
                .unwrap_or_default();
            edges.push(ConnectionEdge {
                connection_id: node.id.clone(),
                connection_name: node.name.clone(),
                direction,
                opposite_id: opposite_id.clone(),
                opposite_name: opposite_name.clone(),
                opposite_group_id,
                queued_display: queued_display.clone(),
            });
        }
        edges
    }
}

impl ListNavigation for BrowserState {
    fn list_len(&self) -> usize {
        self.visible.len()
    }

    fn selected(&self) -> Option<usize> {
        if self.visible.is_empty() {
            None
        } else {
            Some(self.selected)
        }
    }

    fn set_selected(&mut self, index: Option<usize>) {
        self.selected = index.unwrap_or(0);
        self.reset_detail_focus();
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
    Port(crate::client::PortDetail),
}

#[derive(Debug, Clone)]
pub struct DetailRequest {
    pub arena_idx: usize,
    pub kind: NodeKind,
    pub id: String,
}

/// Envelope the worker wraps around a fetched detail before pushing it
/// back to the UI task via `AppEvent::Data(ViewPayload::Browser(
/// BrowserPayload::Detail(...)))`. Task 11 adds the event plumbing.
#[derive(Debug, Clone)]
pub struct NodeDetailSnapshot {
    pub arena_idx: usize,
    pub kind: NodeKind,
    pub id: String,
    pub detail: NodeDetail,
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

    // 2b) Synthesize folder nodes per PG that has Connection and/or
    // ControllerService children. Re-parents the leaves onto the folder.
    // Folders are UI-only: they never come from the client.
    let pg_indices: Vec<usize> = (0..nodes.len())
        .filter(|&i| matches!(nodes[i].kind, NodeKind::ProcessGroup))
        .collect();
    for pg_idx in pg_indices {
        let children = nodes[pg_idx].children.clone();
        let mut processors: Vec<usize> = Vec::new();
        let mut queues: Vec<usize> = Vec::new();
        let mut cs_kids: Vec<usize> = Vec::new();
        let mut rest: Vec<usize> = Vec::new();
        for c in children {
            match nodes[c].kind {
                NodeKind::Processor => processors.push(c),
                NodeKind::Connection => queues.push(c),
                NodeKind::ControllerService => cs_kids.push(c),
                _ => rest.push(c),
            }
        }

        let mut new_children: Vec<usize> = processors;

        if !queues.is_empty() {
            let folder_idx = nodes.len();
            nodes.push(TreeNode {
                parent: Some(pg_idx),
                children: queues.clone(),
                kind: NodeKind::Folder(FolderKind::Queues),
                id: format!("{}::folder::queues", nodes[pg_idx].id),
                group_id: nodes[pg_idx].id.clone(),
                name: format!("Queues ({})", queues.len()),
                status_summary: NodeStatusSummary::Folder {
                    count: queues.len() as u32,
                },
            });
            for q in &queues {
                nodes[*q].parent = Some(folder_idx);
            }
            new_children.push(folder_idx);
        }

        if !cs_kids.is_empty() {
            let folder_idx = nodes.len();
            nodes.push(TreeNode {
                parent: Some(pg_idx),
                children: cs_kids.clone(),
                kind: NodeKind::Folder(FolderKind::ControllerServices),
                id: format!("{}::folder::cs", nodes[pg_idx].id),
                group_id: nodes[pg_idx].id.clone(),
                name: format!("Controller services ({})", cs_kids.len()),
                status_summary: NodeStatusSummary::Folder {
                    count: cs_kids.len() as u32,
                },
            });
            for c in &cs_kids {
                nodes[*c].parent = Some(folder_idx);
            }
            new_children.push(folder_idx);
        }

        new_children.extend(rest);
        nodes[pg_idx].children = new_children;
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
    state.detail_focus = DetailFocus::Tree;
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

/// Insert or update the cached detail for the node at the payload's
/// arena index. Drops the payload if the arena no longer contains that
/// index (tree refreshed between request and response), or if the node
/// at that index changed. Only clears `pending_detail` when the payload
/// matches the current pending index.
pub fn apply_node_detail(state: &mut BrowserState, payload: NodeDetailSnapshot) {
    if payload.arena_idx >= state.nodes.len() {
        return;
    }
    let node = &state.nodes[payload.arena_idx];
    if node.id != payload.id || node.kind != payload.kind {
        return; // stale: node at that index changed
    }
    state.details.insert(payload.arena_idx, payload.detail);
    if state.pending_detail == Some(payload.arena_idx) {
        state.pending_detail = None;
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

/// Type-specific state chip rendered in the fuzzy find State column.
///
/// Pre-computed at `build_flow_index` time from each arena node's
/// `NodeStatusSummary` so the fuzzy renderer never touches the
/// original DTO shape.
#[derive(Debug, Clone)]
pub enum StateBadge {
    /// Processor run-state icon (`●` running, `◌` stopped, `⚠` invalid,
    /// `⌀` disabled, `◐` validating). `style` carries the theme color.
    Processor {
        glyph: char,
        style: ratatui::style::Style,
    },
    /// Controller service state word (`ENABLED`, `DISABLED`, ...) with
    /// theme style.
    Cs {
        label: String,
        style: ratatui::style::Style,
    },
    /// Process group rollup; renders `⚠N` when `invalid>0`, else blank.
    Pg { invalid: u32 },
    /// Connection queue fill; renders `N%` in muted style.
    Conn { fill_percent: u32 },
    /// Input or output port; renders blank (ports have no run state).
    Port,
}

/// Fuzzy-find haystack shared between Browser and the f-key modal.
/// Rebuilt on every tree snapshot.
#[derive(Debug, Clone)]
pub struct FlowIndex {
    pub entries: Vec<FlowIndexEntry>,
}

#[derive(Debug, Clone)]
pub struct FlowIndexEntry {
    pub id: String,
    pub group_id: String,
    pub kind: NodeKind,
    /// Display name shown in the Name column.
    pub name: String,
    /// Group path for the Path column — e.g. `"root/ingest/enrich"`,
    /// or `"(root)"` for the root PG.
    pub group_path: String,
    /// Type-specific state chip for the State column.
    pub state: StateBadge,
    /// Lowercased `"{name}   {kind_label}   {group_path}"` — the
    /// haystack nucleo searches against. Highlight positions from the
    /// matcher index into this string.
    pub haystack: String,
}

#[derive(Debug)]
pub struct PropertiesModalState {
    /// Arena index of the node whose properties we're showing. The
    /// renderer re-resolves the property list from `BrowserState.details`
    /// on every frame; if the node is gone after a tree refresh, the
    /// modal will close itself at render time.
    pub arena_idx: usize,
    pub scroll: usize,
}

impl PropertiesModalState {
    pub fn new(arena_idx: usize) -> Self {
        Self {
            arena_idx,
            scroll: 0,
        }
    }

    pub fn scroll_down(&mut self, max: usize) {
        if self.scroll + 1 < max {
            self.scroll += 1;
        }
    }

    pub fn scroll_up(&mut self) {
        if self.scroll > 0 {
            self.scroll -= 1;
        }
    }

    pub fn page_down(&mut self, by: usize, max: usize) {
        self.scroll = (self.scroll + by).min(max.saturating_sub(1));
    }

    pub fn page_up(&mut self, by: usize) {
        self.scroll = self.scroll.saturating_sub(by);
    }
}

/// Build a fresh `FlowIndex` from the arena. Walks parent pointers to
/// produce each node's group path (e.g. `"root/ingest/enrich"`). PGs,
/// processors, connections, ports, and controller services are all
/// included.
pub fn build_flow_index(state: &BrowserState) -> FlowIndex {
    fn path_to_root(nodes: &[TreeNode], idx: usize) -> String {
        let mut names: Vec<&str> = Vec::new();
        let mut cursor = Some(idx);
        while let Some(i) = cursor {
            names.push(&nodes[i].name);
            cursor = nodes[i].parent;
        }
        names.reverse();
        names.join("/")
    }
    let entries = state
        .nodes
        .iter()
        .filter(|n| !matches!(n.kind, NodeKind::Folder(_)))
        .map(|n| {
            let kind_label = match n.kind {
                NodeKind::ProcessGroup => "PG",
                NodeKind::Processor => "Processor",
                NodeKind::Connection => "Connection",
                NodeKind::InputPort => "InputPort",
                NodeKind::OutputPort => "OutputPort",
                NodeKind::ControllerService => "CS",
                NodeKind::Folder(_) => "Folder",
            };
            let group_path = match n.parent {
                Some(p) => path_to_root(&state.nodes, p),
                None => "(root)".to_string(),
            };
            let haystack = format!("{}   {}   {}", n.name, kind_label, group_path).to_lowercase();
            let state_badge = match &n.status_summary {
                NodeStatusSummary::Processor { run_status } => {
                    let (glyph, style) = crate::widget::run_icon::processor_run_icon(run_status);
                    StateBadge::Processor { glyph, style }
                }
                NodeStatusSummary::ControllerService { state } => {
                    let style = match state.to_ascii_uppercase().as_str() {
                        "ENABLED" => crate::theme::success(),
                        "DISABLED" => crate::theme::disabled(),
                        "ENABLING" | "DISABLING" => crate::theme::info(),
                        _ => crate::theme::muted(),
                    };
                    StateBadge::Cs {
                        label: state.clone(),
                        style,
                    }
                }
                NodeStatusSummary::ProcessGroup { invalid, .. } => {
                    StateBadge::Pg { invalid: *invalid }
                }
                NodeStatusSummary::Connection { fill_percent, .. } => StateBadge::Conn {
                    fill_percent: *fill_percent,
                },
                NodeStatusSummary::Port => StateBadge::Port,
                NodeStatusSummary::Folder { .. } => StateBadge::Port,
            };
            FlowIndexEntry {
                id: n.id.clone(),
                group_id: n.group_id.clone(),
                kind: n.kind,
                name: n.name.clone(),
                group_path,
                state: state_badge,
                haystack,
            }
        })
        .collect();
    FlowIndex { entries }
}

#[cfg(test)]
mod tests {
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

    fn snap(nodes: Vec<RawNode>) -> RecursiveSnapshot {
        RecursiveSnapshot {
            nodes,
            fetched_at: UNIX_EPOCH,
            cs_fetch_error: None,
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
    fn selection_change_with_no_detail_tx_is_noop_but_sets_pending() {
        let mut s = BrowserState::new();
        apply_tree_snapshot(&mut s, demo_snap());
        s.move_down();
        s.emit_detail_request_for_current_selection();
        assert_eq!(s.pending_detail, Some(1));
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
            BulletinSnapshot, ControllerServiceSummary, NodeKind, NodeStatusSummary,
            ProcessGroupDetail,
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
            cs_fetch_error: None,
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
            cs_fetch_error: None,
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
        });
        assert!(
            s.resolve_id("   a1b2c3d4-e5f6-7890-abcd-ef1234567890   ")
                .is_some()
        );
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
}
