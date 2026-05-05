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

/// Canonical bucket display order. `apply_fetch` sorts `grants` by this
/// so the rendered row order matches the index used by `selected`.
pub(crate) const BUCKET_ORDER: &[ResourceBucket] = &[
    ResourceBucket::ProcessGroups,
    ResourceBucket::Processors,
    ResourceBucket::ControllerServices,
    ResourceBucket::InputPorts,
    ResourceBucket::OutputPorts,
    ResourceBucket::RemoteProcessGroups,
    ResourceBucket::Connections,
    ResourceBucket::ReportingTasks,
    ResourceBucket::ParameterContexts,
    ResourceBucket::Global,
];

/// How an identity gained a grant — directly, or via group membership.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GrantSource {
    Direct,
    ViaGroup(String),
}

/// NiFi axis-prefix path segments. The first three correspond directly
/// to `Axis::resource_prefix()` for `ViewData` / `Operate` /
/// `ManagePolicies`; `PROVENANCE_DATA` is a NiFi quirk that maps to the
/// same axis as `DATA` (`ViewData`).
const DATA_AXIS_PREFIX: &str = "/data";
const OPERATE_AXIS_PREFIX: &str = "/operate";
const POLICIES_AXIS_PREFIX: &str = "/policies";
const PROVENANCE_DATA_AXIS_PREFIX: &str = "/provenance-data";

impl ResourceBucket {
    /// Resolve a NiFi resource path to a render bucket. Strips
    /// leading `/data/` / `/operate/` / `/policies/` axis segments
    /// before matching the kind segment.
    pub fn from_resource(resource: &str) -> Self {
        let trimmed = resource
            .strip_prefix(DATA_AXIS_PREFIX)
            .or_else(|| resource.strip_prefix(OPERATE_AXIS_PREFIX))
            .or_else(|| resource.strip_prefix(POLICIES_AXIS_PREFIX))
            .or_else(|| resource.strip_prefix(PROVENANCE_DATA_AXIS_PREFIX))
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
/// `accessPolicies` array to a known `Axis`, if any. The action
/// strings (`"read"` / `"write"`) mirror `Axis::action()`.
pub fn axis_from_action_and_resource(action: &str, resource: &str) -> Option<Axis> {
    match (action, resource_axis_segment(resource)) {
        ("read", "") => Some(Axis::ViewComponent),
        ("write", "") => Some(Axis::ModifyComponent),
        ("read", DATA_AXIS_PREFIX) => Some(Axis::ViewData),
        ("write", OPERATE_AXIS_PREFIX) => Some(Axis::Operate),
        ("write", POLICIES_AXIS_PREFIX) => Some(Axis::ManagePolicies),
        _ => None,
    }
}

fn resource_axis_segment(resource: &str) -> &'static str {
    if resource.starts_with("/data/") {
        DATA_AXIS_PREFIX
    } else if resource.starts_with("/operate/") {
        OPERATE_AXIS_PREFIX
    } else if resource.starts_with("/policies/") {
        POLICIES_AXIS_PREFIX
    } else if resource.starts_with("/provenance-data/") {
        DATA_AXIS_PREFIX
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

// ── Modal state ──────────────────────────────────────────────────────────────

use crate::widget::scroll::CursoredScrollState;
use crate::widget::search::SearchState;

/// Drill-in identity modal lifecycle state.
#[derive(Debug, Clone)]
pub struct IdentityModalState {
    pub identity_id: String,
    pub kind: IdentityKind,
    pub identity: String,
    pub status: IdentityStatus,
    pub grants: Vec<IdentityGrant>,
    pub group_memberships: Vec<String>,
    pub scroll: CursoredScrollState,
    pub search: SearchState,
}

/// Lifecycle status of the `IdentityModalState`.
#[derive(Debug, Clone)]
pub enum IdentityStatus {
    Loading,
    Loaded,
    Failed(String),
}

impl IdentityModalState {
    /// Create a new `IdentityModalState` in `Loading` status.
    pub fn pending(kind: IdentityKind, identity_id: String, identity: String) -> Self {
        Self {
            identity_id,
            kind,
            identity,
            status: IdentityStatus::Loading,
            grants: Vec::new(),
            group_memberships: Vec::new(),
            scroll: CursoredScrollState::default(),
            search: SearchState::default(),
        }
    }

    /// Apply a successful fetch result, transitioning to `Loaded`.
    pub fn apply_fetch(&mut self, result: crate::client::access::IdentityFetchResult) {
        self.identity = result.identity;
        self.grants = result.grants;
        self.group_memberships = result.group_memberships;
        self.status = IdentityStatus::Loaded;
        self.grants.sort_by_key(|g| {
            BUCKET_ORDER
                .iter()
                .position(|b| *b == g.bucket)
                .unwrap_or(usize::MAX)
        });
        self.scroll.clamp_to_content(self.grants.len());
    }

    /// Group grants by `ResourceBucket` in the canonical bucket order.
    /// Sections with no grants are omitted.
    pub fn grouped_by_bucket(&self) -> Vec<(ResourceBucket, Vec<&IdentityGrant>)> {
        BUCKET_ORDER
            .iter()
            .filter_map(|bucket| {
                let group: Vec<&IdentityGrant> =
                    self.grants.iter().filter(|g| g.bucket == *bucket).collect();
                if group.is_empty() {
                    None
                } else {
                    Some((*bucket, group))
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod identity_state_tests {
    use super::*;

    fn grant(resource: &str, action: &str, bucket: ResourceBucket) -> IdentityGrant {
        IdentityGrant {
            axis: axis_from_action_and_resource(action, resource),
            resource: resource.into(),
            bucket,
            source: GrantSource::Direct,
        }
    }

    #[test]
    fn grouped_by_bucket_preserves_canonical_order() {
        let mut s = IdentityModalState::pending(IdentityKind::User, "u1".into(), "alice".into());
        s.grants = vec![
            grant("/flow", "read", ResourceBucket::Global),
            grant("/processors/abc", "read", ResourceBucket::Processors),
            grant("/process-groups/x", "read", ResourceBucket::ProcessGroups),
        ];
        let groups = s.grouped_by_bucket();
        assert_eq!(groups[0].0, ResourceBucket::ProcessGroups);
        assert_eq!(groups[1].0, ResourceBucket::Processors);
        assert_eq!(groups[2].0, ResourceBucket::Global);
    }
}
