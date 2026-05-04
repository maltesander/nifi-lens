//! NiFi `/policies/*` fetchers for the access modal.
//!
//! View-local — never invoked from `ClusterStore`.

use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::view::browser::state::access_modal::{Axis, AxisOutcome, TenantRef, axis_resource};
use futures::StreamExt;
use nifi_rust_client::NifiError;
use nifi_rust_client::dynamic::types::TenantEntity;
use std::collections::HashMap;

/// Calls `GET /nifi-api/policies/{action}/{resource}`. Maps 404 →
/// `AxisOutcome::None`, 403 → `AxisOutcome::Forbidden`, other errors
/// → `AxisOutcome::Error`.
///
/// Fetch a single axis for one component. Returns
/// `AxisOutcome::NotApplicable` when the axis does not apply to the
/// kind (without making a request).
pub async fn fetch_axis(
    client: &NifiClient,
    axis: Axis,
    kind: crate::client::NodeKind,
    id: &str,
) -> Result<AxisOutcome, NifiLensError> {
    let Some(resource) = axis_resource(axis, kind, id) else {
        return Ok(AxisOutcome::NotApplicable);
    };
    // The generated API template is `/policies/{action}/{resource}`;
    // `axis_resource` returns a leading-slash form (e.g. `/processors/id`)
    // which would produce a double-slash in the URL. Strip the leading `/`
    // before passing to the library so the request hits the correct path.
    let resource_param = resource.trim_start_matches('/');
    let result = client
        .policies()
        .get_access_policy_for_resource(axis.action(), resource_param)
        .await;
    Ok(translate_response(result, &resource))
}

fn translate_response(
    result: Result<nifi_rust_client::dynamic::types::AccessPolicyEntity, NifiError>,
    requested: &str,
) -> AxisOutcome {
    match result {
        Ok(entity) => {
            let component = match entity.component {
                Some(c) => c,
                None => return AxisOutcome::None,
            };
            let actual = component.resource.clone().unwrap_or_default();
            let users = component
                .users
                .unwrap_or_default()
                .into_iter()
                .map(tenant_ref_from_entity)
                .collect();
            let groups = component
                .user_groups
                .unwrap_or_default()
                .into_iter()
                .map(tenant_ref_from_entity)
                .collect();
            if actual == requested {
                AxisOutcome::Direct { users, groups }
            } else {
                AxisOutcome::Inherited {
                    source: actual,
                    users,
                    groups,
                }
            }
        }
        Err(NifiError::NotFound { .. }) => AxisOutcome::None,
        Err(NifiError::Forbidden { .. }) => AxisOutcome::Forbidden,
        Err(other) => AxisOutcome::Error(format!("{other}")),
    }
}

fn tenant_ref_from_entity(e: TenantEntity) -> TenantRef {
    let identity = e
        .component
        .as_ref()
        .and_then(|c| c.identity.clone())
        .unwrap_or_default();
    TenantRef {
        id: e.id.unwrap_or_default(),
        identity,
        // TenantDto (the component type in AccessPolicyDto for both users
        // and user_groups) has no members field; member_count is unavailable
        // from the /policies endpoint response.
        member_count: None,
    }
}

use crate::cluster::AccessAuditState;

/// Folds an axis outcome into the cluster-wide `AccessAuditState`.
///
/// Rules (see spec §Auth-Disabled Detection):
/// - Any successful outcome → `Supported`.
/// - From `Unknown`: an `Error` whose message contains "no authorizer"
///   / "not configurable" (a 409) → `Unsupported`.
/// - From `Unknown`: an `Error` whose message contains the canonical
///   "Access is denied. Contact the system administrator" body
///   (an unsecured-NiFi 403 with no real authorizer) → `Unsupported`.
/// - Per-axis `Forbidden` (a real 403 from a configured authorizer
///   where the *caller* lacks read on `/policies/...`) is NOT a
///   global signal — leave the state alone.
/// - All other transitions: state unchanged.
pub fn observe_audit_state(current: AccessAuditState, outcome: &AxisOutcome) -> AccessAuditState {
    match outcome {
        AxisOutcome::Direct { .. } | AxisOutcome::Inherited { .. } | AxisOutcome::None => {
            AccessAuditState::Supported
        }
        AxisOutcome::Error(body) if current == AccessAuditState::Unknown => {
            let lower = body.to_lowercase();
            if lower.contains("no authorizer")
                || lower.contains("not configurable")
                || lower.contains("access is denied. contact the system administrator")
            {
                AccessAuditState::Unsupported
            } else {
                current
            }
        }
        _ => current,
    }
}

/// Run the fan-out and return both the outcomes and the new audit
/// state. Caller folds the new state into `ClusterStore.access_audit`.
pub async fn fetch_component_access_with_audit(
    client: &NifiClient,
    kind: crate::client::NodeKind,
    id: &str,
    current_audit: AccessAuditState,
) -> (AccessFetchResult, AccessAuditState) {
    let result = fetch_component_access(client, kind, id).await;
    let new_audit = result
        .outcomes
        .values()
        .fold(current_audit, observe_audit_state);
    (result, new_audit)
}

/// Per-component result from `fetch_component_access`. Holds one outcome
/// per axis; inapplicable axes carry `AxisOutcome::NotApplicable`.
#[derive(Debug, Clone, Default)]
pub struct AccessFetchResult {
    /// Per-axis outcome. Keys are present for every applicable axis;
    /// inapplicable axes carry `AxisOutcome::NotApplicable`.
    pub outcomes: HashMap<Axis, AxisOutcome>,
}

/// Fan out five parallel `fetch_axis` calls via `buffer_unordered(5)`.
///
/// All five `Axis::ALL` entries are dispatched concurrently; axes that
/// do not apply to the given `kind` return `AxisOutcome::NotApplicable`
/// immediately without issuing a network request.
///
/// The futures returned by the dynamic client are `!Send` — this
/// function must be called from within a `tokio::task::LocalSet`.
pub async fn fetch_component_access(
    client: &NifiClient,
    kind: crate::client::NodeKind,
    id: &str,
) -> AccessFetchResult {
    let calls = Axis::ALL.into_iter().map(|axis| {
        let id_owned = id.to_string();
        async move {
            let outcome = match fetch_axis(client, axis, kind, &id_owned).await {
                Ok(o) => o,
                Err(e) => AxisOutcome::Error(format!("{e}")),
            };
            (axis, outcome)
        }
    });
    let outcomes = futures::stream::iter(calls)
        .buffer_unordered(5)
        .collect::<HashMap<_, _>>()
        .await;
    AccessFetchResult { outcomes }
}
