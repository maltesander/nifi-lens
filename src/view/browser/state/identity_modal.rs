//! State and types for the drill-in Identity modal.

use crate::view::browser::state::access_modal::Axis;

/// Whether a `/tenants/*` lookup targets a user or a user group.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    User,
    UserGroup,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IdentityGrant {
    /// Resolved axis if the (action, resource) maps to one of our 5
    /// known axes; `None` for global / unmapped resources (e.g. `/flow`).
    pub axis: Option<Axis>,
    /// Raw resource path from NiFi (`/processors/abc`, `/flow`, …).
    pub resource: String,
    /// Resolved bucket for grouped rendering.
    pub bucket: ResourceBucket,
    /// `Direct` or `ViaGroup(group_identity)` — the latter for users
    /// only (since users inherit from groups).
    pub source: GrantSource,
}

/// Render bucket for the drill-in modal — resources with a known
/// kind segment land in their own section; everything else is `Global`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResourceBucket {
    ProcessGroups,
    Processors,
    ControllerServices,
    InputPorts,
    OutputPorts,
    RemoteProcessGroups,
    Connections,
    ReportingTasks,
    ParameterContexts,
    /// `/flow`, `/controller`, `/restricted-components`, etc.
    Global,
}

/// How an identity gained a grant — directly, or via group membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantSource {
    Direct,
    ViaGroup(String),
}

impl ResourceBucket {
    /// Resolve a NiFi resource path to a render bucket. Strips
    /// leading `/data/` / `/operate/` / `/policies/` axis segments
    /// before matching the kind segment.
    pub fn from_resource(resource: &str) -> Self {
        let trimmed = resource
            .strip_prefix("/data")
            .or_else(|| resource.strip_prefix("/operate"))
            .or_else(|| resource.strip_prefix("/policies"))
            .or_else(|| resource.strip_prefix("/provenance-data"))
            .unwrap_or(resource);
        let trimmed = trimmed.trim_start_matches('/');
        match trimmed.split_once('/').map(|(k, _)| k).unwrap_or(trimmed) {
            "process-groups" => Self::ProcessGroups,
            "processors" => Self::Processors,
            "controller-services" => Self::ControllerServices,
            "input-ports" => Self::InputPorts,
            "output-ports" => Self::OutputPorts,
            "remote-process-groups" => Self::RemoteProcessGroups,
            "connections" => Self::Connections,
            "reporting-tasks" => Self::ReportingTasks,
            "parameter-contexts" => Self::ParameterContexts,
            _ => Self::Global,
        }
    }

    /// User-facing section label for the drill-in modal.
    pub fn header(self) -> &'static str {
        match self {
            Self::ProcessGroups => "Process groups",
            Self::Processors => "Processors",
            Self::ControllerServices => "Controller services",
            Self::InputPorts => "Input ports",
            Self::OutputPorts => "Output ports",
            Self::RemoteProcessGroups => "Remote process groups",
            Self::Connections => "Connections",
            Self::ReportingTasks => "Reporting tasks",
            Self::ParameterContexts => "Parameter contexts",
            Self::Global => "Global resources",
        }
    }
}

/// Map an `(action, resource)` pair from a TenantEntity's
/// `accessPolicies` array to a known `Axis`, if any.
pub fn axis_from_action_and_resource(action: &str, resource: &str) -> Option<Axis> {
    match (action, resource_axis_segment(resource)) {
        ("read", "") => Some(Axis::ViewComponent),
        ("write", "") => Some(Axis::ModifyComponent),
        ("read", "/data") => Some(Axis::ViewData),
        ("write", "/operate") => Some(Axis::Operate),
        ("write", "/policies") => Some(Axis::ManagePolicies),
        _ => None,
    }
}

fn resource_axis_segment(resource: &str) -> &str {
    if resource.starts_with("/data/") {
        "/data"
    } else if resource.starts_with("/operate/") {
        "/operate"
    } else if resource.starts_with("/policies/") {
        "/policies"
    } else if resource.starts_with("/provenance-data/") {
        "/data"
    } else {
        ""
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bucket_strips_axis_prefix() {
        assert_eq!(
            ResourceBucket::from_resource("/processors/abc"),
            ResourceBucket::Processors
        );
        assert_eq!(
            ResourceBucket::from_resource("/data/processors/abc"),
            ResourceBucket::Processors
        );
        assert_eq!(
            ResourceBucket::from_resource("/operate/process-groups/x"),
            ResourceBucket::ProcessGroups
        );
        assert_eq!(
            ResourceBucket::from_resource("/policies/controller-services/cs-1"),
            ResourceBucket::ControllerServices
        );
        assert_eq!(
            ResourceBucket::from_resource("/flow"),
            ResourceBucket::Global
        );
        assert_eq!(
            ResourceBucket::from_resource("/controller"),
            ResourceBucket::Global
        );
    }

    #[test]
    fn axis_from_action_and_resource_resolves_known_pairs() {
        assert_eq!(
            axis_from_action_and_resource("read", "/processors/abc"),
            Some(Axis::ViewComponent)
        );
        assert_eq!(
            axis_from_action_and_resource("write", "/processors/abc"),
            Some(Axis::ModifyComponent)
        );
        assert_eq!(
            axis_from_action_and_resource("read", "/data/processors/abc"),
            Some(Axis::ViewData)
        );
        assert_eq!(
            axis_from_action_and_resource("write", "/operate/processors/abc"),
            Some(Axis::Operate)
        );
        assert_eq!(
            axis_from_action_and_resource("write", "/policies/processors/abc"),
            Some(Axis::ManagePolicies)
        );
        assert_eq!(
            axis_from_action_and_resource("read", "/flow"),
            Some(Axis::ViewComponent)
        );
        assert_eq!(
            axis_from_action_and_resource("read", "/provenance-data/processors/abc"),
            Some(Axis::ViewData),
            "/provenance-data/* must resolve to ViewData (same axis as /data/*)"
        );
    }
}
