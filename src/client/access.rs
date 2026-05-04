//! NiFi `/policies/*` fetchers for the access modal.
//!
//! View-local — never invoked from `ClusterStore`.

use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::view::browser::state::access_modal::{Axis, AxisOutcome, TenantRef, axis_resource};
use nifi_rust_client::NifiError;
use nifi_rust_client::dynamic::types::TenantEntity;

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
