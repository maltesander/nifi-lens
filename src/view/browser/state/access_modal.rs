//! State and types for the per-component Access modal.
//!
//! See `docs/superpowers/specs/2026-05-04-access-policies-audit-design.md`
//! for the design (gitignored — local only).

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

    /// `(action, resource_prefix)` pair. The full resource is
    /// `format!("/{prefix}{maybe_node_kind}/{id}")` — see
    /// `axis_resource()` for the assembly helper.
    pub fn action(self) -> &'static str {
        match self {
            Self::ViewComponent | Self::ViewData => "read",
            Self::ModifyComponent | Self::Operate | Self::ManagePolicies => "write",
        }
    }

    pub fn resource_prefix(self) -> &'static str {
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
pub fn node_kind_resource(kind: NodeKind) -> &'static str {
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TenantRef {
    pub id: String,
    pub identity: String,
    /// For groups: inline member count from `UserGroupDto.users`,
    /// else `None`.
    pub member_count: Option<usize>,
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
    }
}
