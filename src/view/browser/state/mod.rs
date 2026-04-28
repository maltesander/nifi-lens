//! Pure state for the Browser tab.
//!
//! Everything here is synchronous and no-I/O. The tokio worker in
//! `super::worker` is the only place that touches the network. Navigation
//! helpers, `apply_node_detail`, and the detail-dispatch side-channel
//! land in Tasks 9/10.

use std::collections::{HashMap, HashSet};
use std::time::SystemTime;

use tokio::sync::mpsc;

use crate::app::navigation::ListNavigation;
use crate::client::browser::{
    ConnectionDetail, ControllerServiceDetail, FolderKind, NodeKind, NodeStatusSummary,
    ProcessGroupDetail, ProcessorDetail, RawNode, RecursiveSnapshot,
};
use crate::client::status::{ControllerServiceState, ProcessorStatus};

pub mod action_history_modal;
pub mod parameter_context_modal;
pub use parameter_context_modal::{
    ParameterContextLoad, ParameterContextModalState, ResolvedParameter, resolve,
};
pub mod version_control_modal;
pub use version_control_modal::{VersionControlDifferenceLoad, VersionControlModalState};

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
    Connections,
    /// Focusable single-row section in the PG detail pane. Pressing Enter
    /// opens the parameter-context modal (same as the `p` verb). Only
    /// present in the section list when the PG has a bound context.
    ParameterContext,
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
            NK::Processor => DetailSections(&[
                DetailSection::Properties,
                DetailSection::Connections,
                DetailSection::RecentBulletins,
            ]),
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

    /// Variant for ProcessGroup nodes that conditionally includes a
    /// `ParameterContext` section at index 0 when `has_param_ctx` is true.
    /// Use this (rather than `for_node`) when the binding state is known.
    pub fn for_pg_node(has_param_ctx: bool) -> Self {
        if has_param_ctx {
            DetailSections(&[
                DetailSection::ParameterContext,
                DetailSection::ControllerServices,
                DetailSection::ChildGroups,
                DetailSection::RecentBulletins,
            ])
        } else {
            DetailSections(&[
                DetailSection::ControllerServices,
                DetailSection::ChildGroups,
                DetailSection::RecentBulletins,
            ])
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
                DetailSection::Connections,
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
    /// Phase 7: which focusable sub-section (if any) holds input focus.
    /// Always reset to `Tree` by `reset_detail_focus`, called from every
    /// selection-mutating method on `BrowserState`.
    pub detail_focus: DetailFocus,
    /// Open version-control modal state, if any. `None` while the
    /// modal is closed. Captured at open time from the cluster
    /// snapshot for identity, populated asynchronously with diff data
    /// by the view-local worker (Task 19).
    pub version_modal: Option<VersionControlModalState>,
    /// Live worker handle for the version-control modal's diff fetch.
    /// Aborted on `Close` and on `Refresh` (which spawns a new one).
    /// Cleared by the loaded / failed event handlers.
    pub version_modal_handle: Option<tokio::task::JoinHandle<()>>,
    /// Open parameter-context modal state, if any. `None` while the
    /// modal is closed. Populated asynchronously with chain data by
    /// the view-local worker (Task 17).
    pub parameter_modal: Option<ParameterContextModalState>,
    /// Live worker handle for the parameter-context modal's chain fetch.
    /// Aborted on `Close` and on `Refresh` (which spawns a new one).
    /// Cleared by the loaded / failed event handlers.
    pub parameter_modal_handle: Option<tokio::task::JoinHandle<()>>,
    /// Open action-history modal state, if any. `None` when the modal
    /// is closed. Captured at open time, populated asynchronously by
    /// the view-local worker (Task 11).
    pub action_history_modal: Option<action_history_modal::ActionHistoryModalState>,
    /// Live worker handle for the action-history modal's paginator.
    /// Aborted on close, refresh, tab switch, or selection change.
    pub action_history_modal_handle: Option<tokio::task::JoinHandle<()>>,
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

    /// Look up the `VersionControlSummary` for a PG id from the cluster
    /// snapshot. Returns `None` for unversioned PGs (absent from the map),
    /// for non-PG ids, and while the endpoint is still `Loading`.
    ///
    /// Static method (no `&self`) because the cluster snapshot is a peer
    /// of `BrowserState` on `AppState`, not a child. Call site:
    /// `BrowserState::version_control_for(&state.cluster.snapshot, &pg_id)`.
    pub fn version_control_for<'a>(
        snapshot: &'a crate::cluster::snapshot::ClusterSnapshot,
        pg_id: &str,
    ) -> Option<&'a crate::cluster::snapshot::VersionControlSummary> {
        snapshot
            .version_control
            .latest()
            .and_then(|m| m.by_pg_id.get(pg_id))
    }

    /// Re-stamp every PG node in the arena with its bound parameter context
    /// from the cluster snapshot. Non-PG nodes are untouched. PGs absent
    /// from the map have their field cleared to `None`.
    pub fn apply_parameter_context_bindings(
        &mut self,
        map: &crate::cluster::snapshot::ParameterContextBindingsMap,
    ) {
        for node in self.nodes.iter_mut() {
            if let crate::client::NodeKind::ProcessGroup = node.kind {
                node.parameter_context_ref = map.by_pg_id.get(&node.id).cloned().flatten();
            }
        }
    }

    /// Look up the name of a PG by id. Returns `None` when the id is not
    /// in the arena or the node is not a process group. Used as a
    /// display-only label in the parameter-context modal header.
    pub fn pg_name_for(&self, pg_id: &str) -> Option<&str> {
        self.nodes.iter().find_map(|n| {
            (n.id == pg_id && n.kind == NodeKind::ProcessGroup).then_some(n.name.as_str())
        })
    }

    /// when the PG is unbound, the id is not in the arena, or the node
    /// is not a PG.
    pub fn parameter_context_ref_for(
        &self,
        pg_id: &str,
    ) -> Option<&crate::cluster::snapshot::ParameterContextRef> {
        self.nodes.iter().find_map(|n| {
            (n.id == pg_id)
                .then_some(n.parameter_context_ref.as_ref())
                .flatten()
        })
    }

    /// Open the version-control modal on the currently-selected node.
    /// No-op when the selection is not a PG, when no PG is selected,
    /// or when the PG is not under version control.
    pub fn open_version_control_modal(
        &mut self,
        snapshot: &crate::cluster::snapshot::ClusterSnapshot,
    ) {
        let Some(&arena) = self.visible.get(self.selected) else {
            return;
        };
        let Some(node) = self.nodes.get(arena) else {
            return;
        };
        if !matches!(node.kind, crate::client::NodeKind::ProcessGroup) {
            return;
        }
        let identity = Self::version_control_for(snapshot, &node.id).cloned();
        if identity.is_none() {
            // Defensive: keymap should have grayed out the verb. If we got
            // here, the cluster snapshot raced ahead of the keypress.
            return;
        }
        self.version_modal = Some(VersionControlModalState::pending(
            node.id.clone(),
            node.name.clone(),
            identity,
        ));
    }

    /// Close the version-control modal. Idempotent. Aborts any
    /// in-flight worker — the worker's payload-emit will be cancelled
    /// before it lands on the channel, so a stale `…Loaded` event
    /// can't reopen state on a freshly-closed modal.
    pub fn close_version_control_modal(&mut self) {
        if let Some(h) = self.version_modal_handle.take() {
            h.abort();
        }
        self.version_modal = None;
    }

    /// Apply a successful diff fetch to the open modal. Mismatched
    /// `pg_id` is ignored (the user navigated since dispatch).
    pub fn apply_version_control_modal_loaded(
        &mut self,
        pg_id: String,
        identity: Option<crate::cluster::snapshot::VersionControlSummary>,
        mut differences: crate::client::FlowComparisonGrouped,
    ) {
        // Resolve connection labels from the arena BEFORE taking the
        // mutable borrow on `self.version_modal`. NiFi's
        // local-modifications endpoint leaves `componentName` empty
        // for connections (especially for `COMPONENT_REMOVED`, where
        // the connection is already gone). Resolving against
        // `self.nodes` gives us the source→destination pair so the
        // section header is meaningful.
        for section in &mut differences.sections {
            if section.component_type.eq_ignore_ascii_case("Connection") {
                if let Some(label) = self.resolve_connection_display_label(&section.component_id) {
                    section.display_label = label;
                } else if section.display_label.is_empty() {
                    section.display_label = "(unnamed connection)".to_string();
                }
            } else if section.display_label.is_empty() {
                section.display_label = "(unnamed)".to_string();
            }
        }

        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        if modal.pg_id != pg_id {
            return;
        }
        if let Some(id) = identity {
            modal.identity = Some(id);
        }
        modal.differences = VersionControlDifferenceLoad::Loaded(differences.sections);
        // Drop any in-flight search; it indexed the previous body.
        modal.search = None;
    }

    /// Look up a connection in the arena by id, returning
    /// `"{source_name} → {destination_name}"` when found. Returns
    /// `None` when the connection is not in the current arena (typical
    /// for `COMPONENT_REMOVED` whose backing connection has already
    /// been deleted from the live flow).
    fn resolve_connection_display_label(&self, conn_id: &str) -> Option<String> {
        let node = self
            .nodes
            .iter()
            .find(|n| n.id == conn_id && matches!(n.kind, NodeKind::Connection))?;
        if let NodeStatusSummary::Connection {
            source_name,
            destination_name,
            ..
        } = &node.status_summary
        {
            Some(format!("{source_name} → {destination_name}"))
        } else {
            None
        }
    }

    /// Apply a failed diff fetch to the open modal. Mismatched
    /// `pg_id` is ignored.
    pub fn apply_version_control_modal_failed(&mut self, pg_id: String, err: String) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        if modal.pg_id != pg_id {
            return;
        }
        modal.differences = VersionControlDifferenceLoad::Failed(err);
    }

    /// Open the parameter-context modal for the given PG. Creates a
    /// `Loading` placeholder immediately; the chain is populated
    /// asynchronously by the view-local worker (Task 17).
    pub fn open_parameter_context_modal(
        &mut self,
        pg_id: String,
        pg_path: String,
        preselect: Option<String>,
    ) {
        self.parameter_modal = Some(ParameterContextModalState::pending(
            pg_id, pg_path, preselect,
        ));
    }

    /// Close the parameter-context modal. Idempotent. Aborts any
    /// in-flight worker so a stale `…Loaded` event can't update a
    /// freshly-closed (or re-opened) modal.
    pub fn close_parameter_context_modal(&mut self) {
        if let Some(h) = self.parameter_modal_handle.take() {
            h.abort();
        }
        self.parameter_modal = None;
    }

    /// Open the action-history modal scoped to `(source_id, label)`.
    /// Replaces any previously-open action-history modal. Aborts the
    /// previous worker handle if present.
    pub fn open_action_history_modal(&mut self, source_id: String, component_label: String) {
        if let Some(h) = self.action_history_modal_handle.take() {
            h.abort();
        }
        self.action_history_modal = Some(action_history_modal::ActionHistoryModalState::pending(
            source_id,
            component_label,
        ));
    }

    /// Close the action-history modal and abort any in-flight worker.
    pub fn close_action_history_modal(&mut self) {
        if let Some(h) = self.action_history_modal_handle.take() {
            h.abort();
        }
        self.action_history_modal = None;
    }

    /// Apply a successful chain fetch to the open modal. Mismatched
    /// `pg_id` is ignored (the user navigated since dispatch).
    pub fn apply_parameter_context_modal_loaded(
        &mut self,
        pg_id: String,
        chain: Vec<crate::client::parameter_context::ParameterContextNode>,
    ) {
        if let Some(modal) = self.parameter_modal.as_mut()
            && modal.originating_pg_id == pg_id
        {
            modal.load = ParameterContextLoad::Loaded { chain };
        }
    }

    /// Apply a failed chain fetch to the open modal. Mismatched
    /// `pg_id` is ignored.
    pub fn apply_parameter_context_modal_failed(&mut self, pg_id: String, err: String) {
        if let Some(modal) = self.parameter_modal.as_mut()
            && modal.originating_pg_id == pg_id
        {
            modal.load = ParameterContextLoad::Error { message: err };
        }
    }

    /// Toggle the `show_environmental` flag on the open modal.
    /// Invalidates any active search because the body composition
    /// depends on the flag.
    pub fn toggle_environmental(&mut self) {
        if let Some(modal) = self.version_modal.as_mut() {
            modal.show_environmental = !modal.show_environmental;
            // Search body changes on toggle; invalidate the search index.
            modal.search = None;
        }
    }

    // Search reducer methods — mirror BulletinsState::modal_search_*
    // (src/view/bulletins/state/mod.rs:790-905). Each operates against
    // version_modal.search and the body returned by
    // VersionControlModalState::searchable_body.

    /// Open a fresh live-input search session on the modal.
    pub fn version_modal_search_open(&mut self) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        modal.search = Some(crate::widget::search::SearchState {
            query: String::new(),
            input_active: true,
            committed: false,
            matches: Vec::new(),
            current: None,
        });
    }

    /// Append a character to the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn version_modal_search_push(&mut self, ch: char) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.push(ch);
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Remove the last character from the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn version_modal_search_pop(&mut self) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.pop();
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Commit the current query. If the query is empty, closes search
    /// (sets `modal.search = None`). Otherwise flips `input_active` to
    /// false and `committed` to true.
    pub fn version_modal_search_commit(&mut self) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if search.query.is_empty() {
            modal.search = None;
            return;
        }
        search.input_active = false;
        search.committed = true;
    }

    /// Cancel search and clear all search state from the modal.
    pub fn version_modal_search_cancel(&mut self) {
        if let Some(modal) = self.version_modal.as_mut() {
            modal.search = None;
        }
    }

    /// Advance to the next match, wrapping around. No-op unless search
    /// is committed and has at least one match.
    pub fn version_modal_search_cycle_next(&mut self) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some((cur + 1) % search.matches.len());
    }

    /// Move to the previous match, wrapping around. No-op unless search
    /// is committed and has at least one match.
    pub fn version_modal_search_cycle_prev(&mut self) {
        let Some(modal) = self.version_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some(if cur == 0 {
            search.matches.len() - 1
        } else {
            cur - 1
        });
    }

    // Search reducer methods for the parameter-context modal. Mirror the
    // version_modal_search_* methods above; each operates against
    // parameter_modal.search and the body from
    // ParameterContextModalState::searchable_body.

    /// Open a fresh live-input search session on the parameter-context modal.
    pub fn parameter_modal_search_open(&mut self) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        modal.search = Some(crate::widget::search::SearchState {
            query: String::new(),
            input_active: true,
            committed: false,
            matches: Vec::new(),
            current: None,
        });
    }

    /// Append a character to the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn parameter_modal_search_push(&mut self, ch: char) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.push(ch);
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Remove the last character from the live search query and recompute matches.
    /// No-op if no modal or no active search input.
    pub fn parameter_modal_search_pop(&mut self) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.pop();
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Commit the current query. If the query is empty, closes search.
    /// Otherwise flips `input_active` to false and `committed` to true.
    pub fn parameter_modal_search_commit(&mut self) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if search.query.is_empty() {
            modal.search = None;
            return;
        }
        search.input_active = false;
        search.committed = true;
    }

    /// Cancel search and clear all search state from the parameter-context modal.
    pub fn parameter_modal_search_cancel(&mut self) {
        if let Some(modal) = self.parameter_modal.as_mut() {
            modal.search = None;
        }
    }

    /// Advance to the next match in the parameter-context modal. No-op
    /// unless search is committed and has at least one match.
    pub fn parameter_modal_search_cycle_next(&mut self) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some((cur + 1) % search.matches.len());
    }

    /// Move to the previous match in the parameter-context modal. No-op
    /// unless search is committed and has at least one match.
    pub fn parameter_modal_search_cycle_prev(&mut self) {
        let Some(modal) = self.parameter_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.committed || search.matches.is_empty() {
            return;
        }
        let cur = search.current.unwrap_or(0);
        search.current = Some(if cur == 0 {
            search.matches.len() - 1
        } else {
            cur - 1
        });
    }

    // Search reducer methods for the action-history modal. Mirror the
    // version_modal_search_* and parameter_modal_search_* methods above;
    // each operates against action_history_modal.search and the body from
    // ActionHistoryModalState::searchable_body.

    /// Push a character into the action-history modal's live search query
    /// and recompute matches. No-op if no modal or no active search input.
    pub fn action_history_modal_search_push(&mut self, ch: char) {
        let Some(modal) = self.action_history_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.push(ch);
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Remove the last character from the action-history modal's live search query.
    pub fn action_history_modal_search_pop(&mut self) {
        let Some(modal) = self.action_history_modal.as_mut() else {
            return;
        };
        let body = modal.searchable_body();
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if !search.input_active {
            return;
        }
        search.query.pop();
        search.matches = crate::widget::search::compute_matches(&body, &search.query);
        search.current = if search.matches.is_empty() {
            None
        } else {
            Some(0)
        };
    }

    /// Commit the current query. Empty query closes search; otherwise flips
    /// `input_active` to false and `committed` to true.
    pub fn action_history_modal_search_commit(&mut self) {
        let Some(modal) = self.action_history_modal.as_mut() else {
            return;
        };
        let Some(search) = modal.search.as_mut() else {
            return;
        };
        if search.query.is_empty() {
            modal.search = None;
            return;
        }
        search.input_active = false;
        search.committed = true;
    }

    /// Cancel search and clear all search state from the action-history modal.
    pub fn action_history_modal_search_cancel(&mut self) {
        if let Some(modal) = self.action_history_modal.as_mut() {
            modal.search = None;
        }
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
                match ProcessorStatus::from_wire(run_status) {
                    ProcessorStatus::Invalid => return PgHealth::Red,
                    ProcessorStatus::Stopped => saw_stopped = true,
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
            (DetailSection::Connections, NodeDetail::Processor(_)) => {
                let source_id = &self.nodes[arena_idx].id;
                self.connections_for_processor(source_id).len()
            }
            // ParameterContext is a single-row section; the row is the binding
            // line itself. Returns 0 when no binding (focus can't land here
            // because `for_pg_node(false)` omits the section).
            (DetailSection::ParameterContext, NodeDetail::ProcessGroup(d)) => {
                if self.parameter_context_ref_for(&d.id).is_some() {
                    1
                } else {
                    0
                }
            }
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
    /// Bound parameter context for this PG. `None` for non-PG nodes,
    /// unbound PGs, and while the endpoint is still loading. Re-stamped
    /// on every `ClusterChanged(ParameterContextBindings)` event via
    /// `BrowserState::apply_parameter_context_bindings`.
    pub parameter_context_ref: Option<crate::cluster::snapshot::ParameterContextRef>,
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

/// Task 6 of the central-cluster-store refactor: rebuild the Browser
/// arena from `AppState.cluster.snapshot` instead of from the retired
/// `browser_tree` worker fetch. Called from the `ClusterChanged` arm of
/// the main loop whenever `RootPgStatus`, `ControllerServices`, or
/// `ConnectionsByPg` updates arrive.
///
/// - Reads the flat node list from `snap.root_pg_status.latest()`.
/// - Attaches CS rows from `snap.controller_services.latest()?.members`
///   using each member's `parent_group_id` to pick the owning PG.
/// - Backfills connection endpoint ids from `snap.connections_by_pg`
///   (NiFi's recursive status leaves `sourceId`/`destinationId` null on
///   `ConnectionStatusSnapshotDto`; the per-PG `/connections` fan-out
///   publishes them into the cluster snapshot).
///
/// When `snap.root_pg_status` hasn't delivered a successful value yet,
/// the existing arena is left in place — the Browser UI continues
/// rendering whatever it had last (Loading placeholder if this is the
/// very first frame) instead of blanking out mid-frame.
pub fn rebuild_arena_from_cluster(
    state: &mut crate::app::state::AppState,
    snap: &crate::cluster::snapshot::ClusterSnapshot,
) {
    let Some(root_pg) = snap.root_pg_status.latest() else {
        // Pre-first-fetch: leave the existing arena untouched.
        return;
    };
    // Clone the flat nodes from the snapshot so we can mutate them
    // (backfill connection endpoint ids). One shallow walk; the
    // allocation is cheap relative to the arena rebuild.
    let mut nodes: Vec<RawNode> = root_pg.nodes.clone();

    // Backfill connection endpoint ids from `connections_by_pg`. For
    // each PG's successful fetch, fill in every matching connection's
    // source/destination ids in `NodeStatusSummary::Connection`. PGs
    // with no successful fetch are silently skipped — the affected
    // connections just render without the `→` cross-link marker.
    let conns: std::collections::HashMap<String, &crate::client::ConnectionEndpointIds> = snap
        .connections_by_pg
        .values()
        .filter_map(crate::cluster::snapshot::EndpointState::latest)
        .flat_map(|ce| ce.by_connection.iter().map(|(k, v)| (k.clone(), v)))
        .collect();
    if !conns.is_empty() {
        for node in nodes.iter_mut() {
            if !matches!(node.kind, NodeKind::Connection) {
                continue;
            }
            let Some(endpoints) = conns.get(&node.id) else {
                continue;
            };
            if let NodeStatusSummary::Connection {
                source_id,
                destination_id,
                ..
            } = &mut node.status_summary
            {
                if !endpoints.source_id.is_empty() {
                    *source_id = endpoints.source_id.clone();
                }
                if !endpoints.destination_id.is_empty() {
                    *destination_id = endpoints.destination_id.clone();
                }
            }
        }
    }

    // Attach CS rows from the controller-services snapshot. Each member
    // is appended to the flat node list with `parent_idx` pointing at
    // the PG that owns it; the folder synthesizer in
    // `apply_tree_snapshot` buckets all CS children into a synthetic
    // `Folder(ControllerServices)` node regardless of their position in
    // the flat arena. Members whose `parent_group_id` doesn't match any
    // PG in the arena are silently dropped — they'd have no valid
    // parent row to anchor to.
    if let Some(cs_snap) = snap.controller_services.latest() {
        // Index PG id → arena position once up front. The map owns its
        // keys so the immutable borrow of `nodes` above drops before we
        // mutate it in the splice loop.
        let pg_index: std::collections::HashMap<String, usize> = nodes
            .iter()
            .enumerate()
            .filter(|(_, n)| matches!(n.kind, NodeKind::ProcessGroup))
            .map(|(i, n)| (n.id.clone(), i))
            .collect();
        for m in &cs_snap.members {
            let Some(&pg_idx) = pg_index.get(&m.parent_group_id) else {
                continue;
            };
            nodes.push(RawNode {
                parent_idx: Some(pg_idx),
                kind: NodeKind::ControllerService,
                id: m.id.clone(),
                group_id: m.parent_group_id.clone(),
                name: m.name.clone(),
                status_summary: NodeStatusSummary::ControllerService {
                    state: m.state.clone(),
                },
            });
        }
    }

    // `apply_tree_snapshot` owns the arena-construction logic — folder
    // synthesis, selection preservation, expanded-set translation. We
    // stay on top of it by handing it a throwaway `RecursiveSnapshot`
    // built from the assembled flat node list.
    apply_tree_snapshot(
        &mut state.browser,
        RecursiveSnapshot {
            nodes,
            fetched_at: SystemTime::now(),
        },
    );
    state.flow_index = Some(build_flow_index(&state.browser));
    // The freshly-built FlowIndex starts with `version_state: None` on
    // every entry. Re-stamp from the snapshot we already have so the
    // fuzzy-find drift filters (`:drift` / `:modified` / `:stale` /
    // `:syncerr`) keep returning matches across `RootPgStatus` ticks.
    // Without this re-stamp the drift filters only matched in the brief
    // window between a `VersionControl` tick and the next `RootPgStatus`
    // arena rebuild.
    redraw_version_control(state, snap);
    // Re-stamp parameter_context_ref from the snapshot we already hold.
    // `apply_tree_snapshot` constructs fresh TreeNode literals with
    // `parameter_context_ref: None`, so without this call every arena
    // rebuild (triggered by `ClusterChanged(RootPgStatus)` etc.) would
    // silently clear the field until the next
    // `ClusterChanged(ParameterContextBindings)` — typically up to 30 s
    // of stale-`None` between events.
    if let Some(map) = snap.parameter_context_bindings.latest() {
        state.browser.apply_parameter_context_bindings(map);
    }
}

/// Re-stamp `FlowIndexEntry.version_state` for ProcessGroup entries from
/// the cluster snapshot. O(|entries|) walk; no full FlowIndex rebuild.
/// Called from the main loop on every `ClusterChanged(VersionControl)` event.
///
/// When the endpoint hasn't fetched successfully yet (or has just failed
/// without `last_ok`), every entry's `version_state` is cleared to `None`
/// — keeps the field honest after context switch.
pub fn redraw_version_control(
    state: &mut crate::app::state::AppState,
    snapshot: &crate::cluster::snapshot::ClusterSnapshot,
) {
    let Some(idx) = state.flow_index.as_mut() else {
        return;
    };
    let map = snapshot.version_control.latest();
    for e in &mut idx.entries {
        if matches!(e.kind, NodeKind::ProcessGroup) {
            e.version_state = map.and_then(|m| m.by_pg_id.get(&e.id)).map(|s| s.state);
        } else {
            e.version_state = None;
        }
    }
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
            parameter_context_ref: None,
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
                parameter_context_ref: None,
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
                parameter_context_ref: None,
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
    //    `detail_focus` is taken aside (defaulting to Tree) so we can
    //    re-apply it in step 8 only when the selection lands back on
    //    the same node — otherwise the carried-over section index
    //    would point at unrelated content on a different node.
    let prev_detail_focus = std::mem::take(&mut state.detail_focus);
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
    let selection_preserved = new_selected.is_some();
    state.selected = new_selected.unwrap_or(0);
    if state.visible.is_empty() {
        state.selected = 0;
    } else if state.selected >= state.visible.len() {
        state.selected = state.visible.len() - 1;
    }

    // 8) Restore detail focus when the selection landed on the same
    //    (id, kind). Section list is keyed off node kind, which is
    //    pinned by the (id, kind) match — so the carried-over `idx`
    //    still indexes the same section. Without this, the user's
    //    right-pane focus, per-section row cursor, and horizontal
    //    scroll were wiped on every periodic cluster tick.
    if selection_preserved {
        state.detail_focus = prev_detail_focus;
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
    /// Per-PG version-control state, re-stamped from the cluster
    /// snapshot by `redraw_version_control`. Always `None` for non-PG
    /// kinds; `None` for unversioned PGs and while the endpoint is
    /// `Loading`.
    pub version_state: Option<nifi_rust_client::dynamic::types::VersionControlInformationDtoState>,
}

#[derive(Debug)]
pub struct PropertiesModalState {
    /// Arena index of the node whose properties we're showing. The
    /// renderer re-resolves the property list from `BrowserState.details`
    /// on every frame; if the node is gone after a tree refresh, the
    /// modal will close itself at render time.
    pub arena_idx: usize,
    /// Selected row in the property list. Clamped at render time
    /// against the live `props.len()`.
    pub selected: usize,
}

impl PropertiesModalState {
    pub fn new(arena_idx: usize) -> Self {
        Self {
            arena_idx,
            selected: 0,
        }
    }
}

/// One row of the properties modal table. Computed per frame from
/// the processor or CS detail's `properties` list.
#[derive(Debug, Clone)]
pub struct PropertyRow<'a> {
    pub key: &'a str,
    pub value: &'a str,
    /// `Some(arena_idx)` when the value is a UUID-shaped string that
    /// resolves to a known arena node (renderable `→`). `None`
    /// otherwise.
    pub resolves_to: Option<usize>,
    /// Parameter reference annotation for this row. `Some(preselect)`
    /// when the value contains `#{name}` reference(s) and the owning
    /// PG has a bound parameter context. `preselect` is `Some(name)`
    /// for a single ref, `None` for multiple refs. When `resolves_to`
    /// is also `Some`, UUID annotation takes precedence and
    /// `param_ref` should be ignored by renderers.
    pub param_ref: Option<Option<String>>,
}

/// Classify each property row against the browser arena. The
/// `owning_pg_id` is used to gate parameter-reference annotations:
/// only PGs that have a bound parameter context can produce
/// cross-links. Callers (renderer + reducer) share this so the `→`
/// marker and the Descend cross-link agree on annotation state.
pub fn property_rows<'a>(
    state: &BrowserState,
    owning_pg_id: &str,
    props: &'a [(String, String)],
) -> Vec<PropertyRow<'a>> {
    let has_ctx = state.parameter_context_ref_for(owning_pg_id).is_some();
    props
        .iter()
        .map(|(k, v)| {
            let resolves_to = state.resolve_id(v).map(|r| r.arena_idx);
            let param_ref = if resolves_to.is_none() && has_ctx {
                use crate::view::browser::render::{ParamRefScan, scan_param_refs};
                match scan_param_refs(v.as_str()) {
                    ParamRefScan::None => None,
                    ParamRefScan::Single { name } => Some(Some(name)),
                    ParamRefScan::Multiple => Some(None),
                }
            } else {
                None
            };
            PropertyRow {
                key: k.as_str(),
                value: v.as_str(),
                resolves_to,
                param_ref,
            }
        })
        .collect()
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
                    let style = ControllerServiceState::from_wire(state).style();
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
                version_state: None,
            }
        })
        .collect();
    FlowIndex { entries }
}

#[cfg(test)]
mod tests;
