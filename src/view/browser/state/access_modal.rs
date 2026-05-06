//! State and types for the per-component Access modal.
//!
//! Five user-facing audit axes (view / modify component, view / modify
//! data, view / modify policies). Each axis maps to one
//! `(action, resource)` pair on `/policies/{action}/{resource}`. Drill-in
//! reuses the inline `accessPolicies` on `UserDto` / `UserGroupDto`, so a
//! single `tenants/{...}/{id}` round-trip is sufficient.

use crate::client::NodeKind;

/// One of the five user-facing audit axes. Each axis maps to an
/// `(action, resource)` pair on NiFi's `/policies/{action}/{resource}`
/// endpoint; the resource path also embeds the component kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Axis {
    ViewComponent,
    ModifyComponent,
    ViewData,
    Operate,
    ManagePolicies,
}

impl Axis {
    /// All five axes in display order.
    pub const ALL: [Axis; 5] = [
        Axis::ViewComponent,
        Axis::ModifyComponent,
        Axis::ViewData,
        Axis::Operate,
        Axis::ManagePolicies,
    ];

    /// Short header label for the matrix column.
    pub fn header(self) -> &'static str {
        match self {
            Self::ViewComponent => "view",
            Self::ModifyComponent => "mod",
            Self::ViewData => "data",
            Self::Operate => "oper",
            Self::ManagePolicies => "pol",
        }
    }

    /// NiFi action string for this axis: `"read"` or `"write"`.
    pub fn action(self) -> &'static str {
        match self {
            Self::ViewComponent | Self::ViewData => "read",
            Self::ModifyComponent | Self::Operate | Self::ManagePolicies => "write",
        }
    }

    /// Leading path segment inserted before the component-kind slug;
    /// empty for view/modify which target the bare resource.
    pub(crate) fn resource_prefix(self) -> &'static str {
        match self {
            Self::ViewComponent | Self::ModifyComponent => "",
            Self::ViewData => "/data",
            Self::Operate => "/operate",
            Self::ManagePolicies => "/policies",
        }
    }

    /// Whether this axis applies to the given component kind.
    /// Empty axes render `—` in every cell instead of being fetched.
    pub fn applies_to(self, kind: NodeKind) -> bool {
        use NodeKind::*;
        if matches!(kind, Folder(_)) {
            return false;
        }
        match self {
            Self::ViewComponent | Self::ModifyComponent | Self::ManagePolicies => true,
            Self::ViewData => {
                matches!(
                    kind,
                    ProcessGroup | Processor | InputPort | OutputPort | Connection
                )
            }
            Self::Operate => {
                matches!(
                    kind,
                    ProcessGroup | Processor | InputPort | OutputPort | RemoteProcessGroup
                )
            }
        }
    }
}

/// Resource path segment for a component kind (the `{resource}` slot
/// in `/policies/{action}/{resource}/{id}`).
pub(crate) fn node_kind_resource(kind: NodeKind) -> &'static str {
    use NodeKind::*;
    match kind {
        ProcessGroup => "process-groups",
        Processor => "processors",
        ControllerService => "controller-services",
        InputPort => "input-ports",
        OutputPort => "output-ports",
        RemoteProcessGroup => "remote-process-groups",
        Connection => "connections",
        // Folder is excluded by the verb's enabled() predicate.
        Folder(_) => "",
    }
}

/// Build the full resource slug for an axis × kind × id triple.
/// Returns `None` when the axis does not apply to the kind.
pub fn axis_resource(axis: Axis, kind: NodeKind, id: &str) -> Option<String> {
    if !axis.applies_to(kind) {
        return None;
    }
    Some(format!(
        "{}/{}/{}",
        axis.resource_prefix(),
        node_kind_resource(kind),
        id
    ))
}

/// Per-axis fetch outcome. `Inherited` carries the actual resource
/// the policy lives on (whichever ancestor in NiFi's resource graph).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AxisOutcome {
    /// Explicit policy on this exact resource. Carries the
    /// (users, groups) lists from the response.
    Direct {
        users: Vec<TenantRef>,
        groups: Vec<TenantRef>,
    },
    /// Inherited from an ancestor. `source` is `component.resource`
    /// from the response.
    Inherited {
        source: String,
        users: Vec<TenantRef>,
        groups: Vec<TenantRef>,
    },
    /// 404 — no policy in the inheritance chain.
    None,
    /// 403 — caller lacks read on `/policies/{...}`.
    Forbidden,
    /// Network / 5xx. Renders `?` and surfaces a retry hint.
    Error(String),
    /// Axis does not apply to this component kind. Cell renders `—`.
    NotApplicable,
}

/// A user or group identity returned in a NiFi
/// `/policies/{action}/{resource}` response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRef {
    pub id: String,
    pub identity: String,
    /// For groups: inline member count from `UserGroupDto.users`,
    /// else `None`.
    pub member_count: Option<usize>,
}

// ── Modal state ──────────────────────────────────────────────────────────────

use crate::widget::scroll::CursoredScrollState;
use crate::widget::search::SearchState;
use std::collections::HashMap;

/// Per-component matrix modal lifecycle.
#[derive(Debug, Clone)]
pub struct AccessModalState {
    pub component_id: String,
    pub component_kind: NodeKind,
    pub component_label: String,
    pub status: ModalStatus,
    pub matrix: Vec<MatrixRow>,
    pub scroll: CursoredScrollState,
    /// `Some` while the user is typing or after pressing Enter to
    /// commit. `None` means no active search. Mirrors the
    /// version-control / parameter-context modal lifecycle.
    pub search: Option<SearchState>,
}

/// Lifecycle status of the `AccessModalState`.
#[derive(Debug, Clone)]
pub enum ModalStatus {
    Loading,
    Loaded,
    Failed(String),
}

/// One row in the matrix — an identity (user or group) with its
/// per-axis cells.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MatrixRow {
    pub tenant: TenantRef,
    pub is_group: bool,
    pub cells: HashMap<Axis, MatrixCell>,
}

/// Per-axis, per-identity cell value in the access matrix.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MatrixCell {
    Direct,
    Inherited { source: String },
    None,
    Forbidden,
    Error,
    NotApplicable,
}

impl MatrixCell {
    /// Single-character glyph used in the matrix column.
    pub fn glyph(&self) -> &'static str {
        match self {
            Self::Direct => "✓",
            Self::Inherited { .. } => "↑",
            Self::None | Self::NotApplicable => "—",
            Self::Forbidden | Self::Error => "?",
        }
    }
}

impl AccessModalState {
    /// Create a new `AccessModalState` in `Loading` status.
    pub fn new(component_id: String, component_kind: NodeKind, component_label: String) -> Self {
        Self {
            component_id,
            component_kind,
            component_label,
            status: ModalStatus::Loading,
            matrix: Vec::new(),
            scroll: CursoredScrollState::default(),
            search: None,
        }
    }

    /// Flat searchable body for `/`-search. One line per matrix row in
    /// `matrix` order, so a `MatchSpan { line_idx, .. }` indexes
    /// directly into `self.matrix`. Identity strings are the natural
    /// search target ("alice", "ops-team", DN suffixes, etc.); the
    /// `[group]` tag lets users narrow by category with `:group`.
    pub fn searchable_body(&self) -> String {
        let mut lines: Vec<String> = Vec::with_capacity(self.matrix.len());
        for row in &self.matrix {
            let tag = if row.is_group { " [group]" } else { "" };
            lines.push(format!("{}{tag}", row.tenant.identity));
        }
        lines.join("\n")
    }

    /// Builds the matrix from a 5-axis fetch result. Identities are
    /// the union of all (users ∪ groups) referenced by any axis.
    pub fn apply_fetch(&mut self, result: crate::client::access::AccessFetchResult) {
        let mut by_id: HashMap<String, MatrixRow> = HashMap::new();
        for axis in Axis::ALL {
            let outcome = result
                .outcomes
                .get(&axis)
                .cloned()
                .unwrap_or(AxisOutcome::NotApplicable);
            match &outcome {
                AxisOutcome::Direct { users, groups } => {
                    for u in users {
                        upsert_cell(&mut by_id, u, false, axis, MatrixCell::Direct);
                    }
                    for g in groups {
                        upsert_cell(&mut by_id, g, true, axis, MatrixCell::Direct);
                    }
                }
                AxisOutcome::Inherited {
                    source,
                    users,
                    groups,
                } => {
                    for u in users {
                        upsert_cell(
                            &mut by_id,
                            u,
                            false,
                            axis,
                            MatrixCell::Inherited {
                                source: source.clone(),
                            },
                        );
                    }
                    for g in groups {
                        upsert_cell(
                            &mut by_id,
                            g,
                            true,
                            axis,
                            MatrixCell::Inherited {
                                source: source.clone(),
                            },
                        );
                    }
                }
                _ => {}
            }
            // For all rows that didn't get a cell for this axis, fill
            // with the appropriate "absent" marker.
            for row in by_id.values_mut() {
                row.cells.entry(axis).or_insert_with(|| match &outcome {
                    AxisOutcome::None => MatrixCell::None,
                    AxisOutcome::Forbidden => MatrixCell::Forbidden,
                    AxisOutcome::Error(_) => MatrixCell::Error,
                    AxisOutcome::NotApplicable => MatrixCell::NotApplicable,
                    _ => MatrixCell::None,
                });
            }
        }
        let mut rows: Vec<_> = by_id.into_values().collect();
        rows.sort_by(|a, b| a.tenant.identity.cmp(&b.tenant.identity));
        self.matrix = rows;
        self.status = ModalStatus::Loaded;
        self.scroll.clamp_to_content(self.matrix.len());
    }
}

fn upsert_cell(
    by_id: &mut HashMap<String, MatrixRow>,
    tenant: &TenantRef,
    is_group: bool,
    axis: Axis,
    cell: MatrixCell,
) {
    let entry = by_id.entry(tenant.id.clone()).or_insert_with(|| MatrixRow {
        tenant: tenant.clone(),
        is_group,
        cells: HashMap::new(),
    });
    entry.cells.insert(axis, cell);
}

#[cfg(test)]
mod state_tests {
    use super::*;
    use crate::client::access::AccessFetchResult;

    fn alice() -> TenantRef {
        TenantRef {
            id: "u1".into(),
            identity: "alice@corp".into(),
            member_count: None,
        }
    }

    fn ops_team() -> TenantRef {
        TenantRef {
            id: "g1".into(),
            identity: "ops-team".into(),
            member_count: Some(12),
        }
    }

    #[test]
    fn apply_fetch_unions_users_across_axes() {
        let mut result = AccessFetchResult::default();
        result.outcomes.insert(
            Axis::ViewComponent,
            AxisOutcome::Direct {
                users: vec![alice()],
                groups: vec![ops_team()],
            },
        );
        result.outcomes.insert(
            Axis::ModifyComponent,
            AxisOutcome::Direct {
                users: vec![alice()],
                groups: vec![],
            },
        );
        for axis in [Axis::ViewData, Axis::Operate, Axis::ManagePolicies] {
            result.outcomes.insert(axis, AxisOutcome::None);
        }

        let mut s = AccessModalState::new("p1".into(), NodeKind::Processor, "EnrichOrders".into());
        s.apply_fetch(result);

        assert_eq!(s.matrix.len(), 2);
        let alice_row = s
            .matrix
            .iter()
            .find(|r| r.tenant.identity == "alice@corp")
            .unwrap();
        assert_eq!(alice_row.cells[&Axis::ViewComponent], MatrixCell::Direct);
        assert_eq!(alice_row.cells[&Axis::ModifyComponent], MatrixCell::Direct);
        assert_eq!(alice_row.cells[&Axis::ViewData], MatrixCell::None);
        let ops_row = s
            .matrix
            .iter()
            .find(|r| r.tenant.identity == "ops-team")
            .unwrap();
        assert_eq!(ops_row.cells[&Axis::ViewComponent], MatrixCell::Direct);
        assert_eq!(ops_row.cells[&Axis::ModifyComponent], MatrixCell::None);
    }

    #[test]
    fn apply_fetch_renders_inherited_when_source_differs() {
        let mut result = AccessFetchResult::default();
        result.outcomes.insert(
            Axis::ViewComponent,
            AxisOutcome::Inherited {
                source: "/process-groups/root".into(),
                users: vec![alice()],
                groups: vec![],
            },
        );
        for axis in [
            Axis::ModifyComponent,
            Axis::ViewData,
            Axis::Operate,
            Axis::ManagePolicies,
        ] {
            result.outcomes.insert(axis, AxisOutcome::None);
        }

        let mut s = AccessModalState::new("p1".into(), NodeKind::Processor, "EnrichOrders".into());
        s.apply_fetch(result);

        let cell = &s.matrix[0].cells[&Axis::ViewComponent];
        match cell {
            MatrixCell::Inherited { source } => assert_eq!(source, "/process-groups/root"),
            other => panic!("expected Inherited, got {other:?}"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axes_cover_all_five() {
        assert_eq!(Axis::ALL.len(), 5);
    }

    #[test]
    fn applies_to_excludes_data_for_controller_service() {
        assert!(!Axis::ViewData.applies_to(NodeKind::ControllerService));
        assert!(Axis::ViewComponent.applies_to(NodeKind::ControllerService));
        assert!(Axis::ManagePolicies.applies_to(NodeKind::ControllerService));
    }

    #[test]
    fn axis_resource_assembles_correctly() {
        assert_eq!(
            axis_resource(Axis::ViewComponent, NodeKind::Processor, "abc-123"),
            Some("/processors/abc-123".to_string())
        );
        assert_eq!(
            axis_resource(Axis::ViewData, NodeKind::Processor, "abc-123"),
            Some("/data/processors/abc-123".to_string())
        );
        assert_eq!(
            axis_resource(Axis::Operate, NodeKind::Processor, "abc-123"),
            Some("/operate/processors/abc-123".to_string())
        );
        assert_eq!(
            axis_resource(Axis::ManagePolicies, NodeKind::Processor, "abc-123"),
            Some("/policies/processors/abc-123".to_string())
        );
        assert_eq!(
            axis_resource(Axis::ViewData, NodeKind::ControllerService, "abc-123"),
            None
        );
        assert_eq!(
            axis_resource(
                Axis::ViewComponent,
                NodeKind::Folder(crate::client::FolderKind::Queues),
                "id"
            ),
            None,
            "Folder must always return None to prevent malformed paths",
        );
    }
}
