//! RPG helpers shared between sink_eu and sink_apac.
//!
//! Both regional sinks need to:
//!   1. Create an RPG targeting the cluster's S2S endpoint
//!   2. Optionally set it TRANSMITTING (sink-eu does this; sink-apac stays idle)
//!   3. Wait until NiFi has discovered the remote input ports via S2S handshake
//!   4. Look up the port mapping by name and wire a local processor's
//!      output to it (REMOTE_INPUT_PORT connection)
//!
//! Patterns follow the proven `fixture::remote` module — same target URI,
//! same DTO construction style, same /run-status endpoint for transitioning
//! to TRANSMITTING. The new piece is `connect_processor_to_remote_input_port`,
//! which discovers the per-port mapping IDs from the RPG entity after
//! NiFi handshakes with the S2S target.

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::make_connection;
use crate::error::{Result, SeederError};
use crate::state::poll_until;

/// Hostname:port of the floor NiFi inside the docker compose network.
/// Both clusters' fixtures use this URL — the floor exposes S2S and lists
/// every input port in its flow as a remote-discoverable port.
pub const REMOTE_TARGET_URI: &str = "https://nifi-2-6-0:8443/nifi";

/// POST a new RPG under `parent_pg_id` targeting the floor NiFi's S2S
/// endpoint.
pub async fn create_remote_process_group(
    client: &DynamicClient,
    parent_pg_id: &str,
    name: &str,
) -> Result<String> {
    let body = make_rpg_entity(name, REMOTE_TARGET_URI);
    let created = client
        .processgroups()
        .create_remote_process_group(parent_pg_id, &body)
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

/// PUT `/remote-process-groups/{id}/run-status` to transition the RPG to
/// TRANSMITTING. Fetches a fresh revision first — NiFi may have advanced
/// the version counter during the target-side handshake that occurs after
/// `create_remote_process_group` returns, so a hardcoded `0` would cause a
/// 400 "not the most up-to-date revision" error.
pub async fn set_rpg_transmitting(client: &DynamicClient, rpg_id: &str) -> Result<()> {
    let entity = client
        .remoteprocessgroups()
        .get_remote_process_group(rpg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("get remote process group {rpg_id} for revision"),
            source: Box::new(e),
        })?;
    let revision_version = entity
        .revision
        .as_ref()
        .and_then(|r| r.version)
        .unwrap_or(0);

    let mut revision = types::RevisionDto::default();
    revision.version = Some(revision_version);

    let mut body = types::RemotePortRunStatusEntity::default();
    body.revision = Some(revision);
    body.state = Some(
        types::RemotePortRunStatusEntityState::Transmitting
            .as_str()
            .to_string(),
    );

    client
        .remoteprocessgroups()
        .update_remote_process_group_run_status(rpg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("set TRANSMITTING on remote process group {rpg_id}"),
            source: Box::new(e),
        })?;
    Ok(())
}

/// Wait up to `timeout` for the RPG to discover at least one remote input
/// port via S2S handshake. Discovery happens asynchronously inside NiFi
/// after the RPG is created; the `component.contents.input_ports` list is
/// initially empty and populates once the target's flow has been queried.
pub async fn wait_for_remote_input_ports(
    client: &DynamicClient,
    rpg_id: &str,
    timeout: Duration,
) -> Result<()> {
    let id = rpg_id.to_string();
    poll_until(
        format!("RPG {rpg_id} input port discovery"),
        "input ports discovered",
        timeout,
        || {
            let id = id.clone();
            async move {
                let got = client
                    .remoteprocessgroups()
                    .get_remote_process_group(&id)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: format!("poll RPG {id} for input port discovery"),
                        source: Box::new(e),
                    })?;
                let n = got
                    .component
                    .as_ref()
                    .and_then(|c| c.contents.as_ref())
                    .and_then(|cnt| cnt.input_ports.as_ref())
                    .map(|p| p.len())
                    .unwrap_or(0);
                if n > 0 { Ok(Some(())) } else { Ok(None) }
            }
        },
    )
    .await
}

/// Connect a local processor's `success` relationship to a remote input
/// port on the given RPG. Discovers the per-port mapping ID by matching
/// the `name` field of each entry in `component.contents.input_ports`.
pub async fn connect_processor_to_remote_input_port(
    client: &DynamicClient,
    parent_pg_id: &str,
    src_processor_id: &str,
    rpg_id: &str,
    remote_input_port_name: &str,
) -> Result<()> {
    let rpg = client
        .remoteprocessgroups()
        .get_remote_process_group(rpg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch RPG {rpg_id} for port mapping discovery"),
            source: Box::new(e),
        })?;

    let component = rpg.component.ok_or_else(|| SeederError::Invariant {
        message: format!("RPG {rpg_id} missing component"),
    })?;
    let contents = component.contents.ok_or_else(|| SeederError::Invariant {
        message: format!("RPG {rpg_id} missing component.contents"),
    })?;
    let input_ports = contents.input_ports.ok_or_else(|| SeederError::Invariant {
        message: format!("RPG {rpg_id} missing input_ports list"),
    })?;

    let mapping = input_ports
        .iter()
        .find(|p| p.name.as_deref() == Some(remote_input_port_name))
        .ok_or_else(|| SeederError::Invariant {
            message: format!(
                "RPG {rpg_id} has no input port mapping named '{remote_input_port_name}'"
            ),
        })?;

    let mapping_id = mapping.id.clone().ok_or_else(|| SeederError::Invariant {
        message: format!("RPG {rpg_id} mapping for '{remote_input_port_name}' has no id"),
    })?;

    let body = make_connection(
        parent_pg_id,
        src_processor_id,
        "PROCESSOR",
        rpg_id,
        &mapping_id,
        "REMOTE_INPUT_PORT",
        vec!["success"],
    );
    client
        .processgroups()
        .create_connection(parent_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!(
                "create connection from processor {src_processor_id} to RPG {rpg_id} \
                 port {remote_input_port_name} (mapping {mapping_id})"
            ),
            source: Box::new(e),
        })?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Private builders
// ---------------------------------------------------------------------------

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_uri_constant_uses_floor_hostname() {
        assert!(REMOTE_TARGET_URI.contains("nifi-2-6-0"));
        assert!(REMOTE_TARGET_URI.starts_with("https://"));
    }

    #[test]
    fn make_rpg_entity_sets_name_and_target_uri() {
        let entity = make_rpg_entity("rpg-eu", "https://nifi-2-6-0:8443/nifi");
        let component = entity.component.unwrap();
        assert_eq!(component.name.as_deref(), Some("rpg-eu"));
        assert_eq!(
            component.target_uri.as_deref(),
            Some("https://nifi-2-6-0:8443/nifi")
        );
        let rev = entity.revision.unwrap();
        assert_eq!(rev.version, Some(0));
    }
}
