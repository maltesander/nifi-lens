//! Remote-pipeline fixture: a top-level PG containing two Remote Process
//! Groups (RPGs) for live coverage of nifi-lens's RPG read paths.
//!
//! ```text
//! remote-pipeline/
//! ├── transmitting-rpg   (run_status: TRANSMITTING, target = floor NiFi)
//! └── idle-rpg           (run_status: STOPPED — default after create)
//! ```
//!
//! Both RPGs target the floor NiFi's own bind URL
//! (`https://nifi-2-6-0:8443/nifi`). That hostname is resolvable across
//! containers in the integration-tests docker network, so the RPGs can
//! actually report a connection state. The targets are syntactic only —
//! NiFi creates the RPG entity regardless of whether the remote responds.
//!
//! Used by `tests/integration_remote_process_groups.rs` (Task 21).

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::error::{Result, SeederError};
use crate::fixture::healthy::create_child_pg;

/// Hostname:port of the floor NiFi inside the docker compose network.
/// Resolvable from the cluster nodes too (shared docker network). The
/// floor NiFi binds HTTPS on 8443 — see `integration-tests/docker-compose.yml`.
const REMOTE_TARGET_URI: &str = "https://nifi-2-6-0:8443/nifi";

/// Create the `remote-pipeline` PG under `parent_pg_id` with two RPGs:
/// `transmitting-rpg` (started → TRANSMITTING) and `idle-rpg` (STOPPED).
pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding remote-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "remote-pipeline").await?;

    let transmitting_id =
        create_remote_process_group(client, &pg_id, "transmitting-rpg", REMOTE_TARGET_URI).await?;
    set_rpg_run_status(
        client,
        &transmitting_id,
        types::RemotePortRunStatusEntityState::Transmitting,
    )
    .await?;
    tracing::info!(%transmitting_id, "transmitting-rpg created and set TRANSMITTING");

    let idle_id =
        create_remote_process_group(client, &pg_id, "idle-rpg", REMOTE_TARGET_URI).await?;
    tracing::info!(%idle_id, "idle-rpg created (default STOPPED)");

    tracing::info!("remote-pipeline seeded");
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// POST a new Remote Process Group under `pg_id` with the given target URI.
async fn create_remote_process_group(
    client: &DynamicClient,
    pg_id: &str,
    name: &str,
    target_uri: &str,
) -> Result<String> {
    let body = make_rpg_entity(name, target_uri);
    let created = client
        .processgroups()
        .create_remote_process_group(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create remote process group {name}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("remote process group {name} has no id after create"),
        })
}

/// Build a `RemoteProcessGroupEntity` for creation. Uses the singular
/// `target_uri` field (supported on every NiFi 2.x version including the
/// 2.6.0 floor), and lets NiFi default the rest of the DTO.
fn make_rpg_entity(name: &str, target_uri: &str) -> types::RemoteProcessGroupEntity {
    let mut component = types::RemoteProcessGroupDto::default();
    component.name = Some(name.to_string());
    component.target_uri = Some(target_uri.to_string());

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::RemoteProcessGroupEntity::default();
    entity.component = Some(component);
    entity.revision = Some(revision);
    entity
}

/// Build a `RemotePortRunStatusEntity` requesting `state`. The body needs
/// the current revision (NiFi enforces optimistic concurrency on
/// `/run-status` PUTs); we read revision `0` here because we just created
/// the RPG and haven't mutated it yet.
///
/// The dynamic `state` field is `Option<String>`; we serialise the typed
/// helper enum via its `as_str()` wire form so the conversion lives in
/// one place.
fn make_run_status(
    state: types::RemotePortRunStatusEntityState,
) -> types::RemotePortRunStatusEntity {
    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::RemotePortRunStatusEntity::default();
    entity.revision = Some(revision);
    entity.state = Some(state.as_str().to_string());
    entity
}

/// PUT `/remote-process-groups/{id}/run-status` to transition the RPG.
async fn set_rpg_run_status(
    client: &DynamicClient,
    rpg_id: &str,
    state: types::RemotePortRunStatusEntityState,
) -> Result<()> {
    let body = make_run_status(state);
    client
        .remoteprocessgroups()
        .update_remote_process_group_run_status(rpg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("update run-status of remote process group {rpg_id}"),
            source: Box::new(e),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn make_rpg_entity_sets_name_and_target_uri() {
        let entity = make_rpg_entity("transmitting-rpg", "https://nifi-east:8443/nifi");
        let component = entity.component.unwrap();
        assert_eq!(component.name.as_deref(), Some("transmitting-rpg"));
        assert_eq!(
            component.target_uri.as_deref(),
            Some("https://nifi-east:8443/nifi")
        );
        let rev = entity.revision.unwrap();
        assert_eq!(rev.version, Some(0));
    }

    #[test]
    fn make_run_status_sets_state_and_revision_zero() {
        let body = make_run_status(types::RemotePortRunStatusEntityState::Transmitting);
        assert_eq!(body.state.as_deref(), Some("TRANSMITTING"));
        assert_eq!(body.revision.unwrap().version, Some(0));
    }

    #[test]
    fn make_run_status_serialises_stopped_as_wire_value() {
        let body = make_run_status(types::RemotePortRunStatusEntityState::Stopped);
        assert_eq!(body.state.as_deref(), Some("STOPPED"));
    }

    #[test]
    fn target_uri_constant_uses_floor_hostname() {
        assert!(REMOTE_TARGET_URI.contains("nifi-2-6-0"));
        assert!(REMOTE_TARGET_URI.starts_with("https://"));
    }
}
