//! Nuke-and-repave: delete everything under root in a safe order.
//!
//! NiFi's REST API rejects deletion of non-terminal components (running
//! processors, enabled controller services, etc.). The sequence enforced
//! here is:
//!
//!   1. Stop all processors in root recursively (`schedule_components` with
//!      state `STOPPED`).
//!   2. Disable all controller services under root and poll each to
//!      `DISABLED`.
//!   3. Delete all child PGs under root — this cascade-deletes their
//!      processors, connections, ports, and controller services in one
//!      atomic operation.
//!   4. Delete any remaining root-level controller services.
//!
//! Step 3 subsumes the other component types because deleting a PG
//! deletes its contents atomically.

use std::time::Duration;

use nifi_rust_client::dynamic::{
    DynamicClient,
    traits::{
        ControllerServicesApi as _, ControllerServicesRunStatusApi as _, FlowApi as _,
        FlowControllerServicesApi as _, ProcessGroupsApi as _,
        ProcessGroupsEmptyAllConnectionsRequestsApi as _, ProcessGroupsProcessGroupsApi as _,
    },
    types,
};

use crate::error::{Result, SeederError};
use crate::state::poll_until;

/// Delete every process group and controller service beneath root,
/// leaving the cluster in the same state as a freshly started NiFi.
pub async fn nuke_and_repave(client: &DynamicClient) -> Result<()> {
    tracing::info!("nuke-and-repave: stopping all processors under root");
    stop_all_under_root(client).await?;

    tracing::info!("nuke-and-repave: disabling controller services");
    disable_all_controller_services(client).await?;

    tracing::info!("nuke-and-repave: deleting child process groups");
    delete_all_child_pgs(client).await?;

    tracing::info!("nuke-and-repave: deleting root controller services");
    delete_all_root_controller_services(client).await?;

    tracing::info!("nuke-and-repave: complete");
    Ok(())
}

async fn stop_all_under_root(client: &DynamicClient) -> Result<()> {
    // `PUT /flow/process-groups/{id}` with state=STOPPED and no explicit
    // `components` map propagates to all authorized descendants.
    let mut body = types::ScheduleComponentsEntity::default();
    body.id = Some("root".to_string());
    body.state = Some("STOPPED".to_string());

    client
        .flow_api()
        .schedule_components("root", &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "stop all processors under root".into(),
            source: Box::new(e),
        })?;

    // No polling here — the subsequent delete calls will reject if any
    // component is still in a non-terminal state, and the error message
    // will identify the offender.
    Ok(())
}

async fn disable_all_controller_services(client: &DynamicClient) -> Result<()> {
    // List every CS reachable from root (recursive).
    let entity = client
        .flow_api()
        .controller_services("root")
        .get_controller_services_from_group(Some(false), Some(true), Some(false), Some(false))
        .await
        .map_err(|e| SeederError::Api {
            message: "list controller services under root".into(),
            source: Box::new(e),
        })?;

    let services = entity.controller_services.unwrap_or_default();
    for svc in services {
        let Some(id) = svc.id.clone() else { continue };
        let state = svc
            .component
            .as_ref()
            .and_then(|c| c.state.as_ref())
            .cloned()
            .unwrap_or_default();
        if state != "ENABLED" && state != "ENABLING" {
            continue;
        }

        let mut run_status = types::ControllerServiceRunStatusEntity::default();
        run_status.state = Some("DISABLED".to_string());
        if let Some(rev) = svc.revision.clone() {
            run_status.revision = Some(rev);
        }
        client
            .controller_services_api()
            .run_status(&id)
            .update_run_status_1(&run_status)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("disable controller service {id}"),
                source: Box::new(e),
            })?;

        // Poll until DISABLED.
        let id_for_poll = id.clone();
        poll_until(
            format!("controller service {id}"),
            "DISABLED",
            Duration::from_secs(30),
            || {
                let id = id_for_poll.clone();
                async move {
                    let got = client
                        .controller_services_api()
                        .get_controller_service(&id, None)
                        .await
                        .map_err(|e| SeederError::Api {
                            message: format!("poll controller service {id}"),
                            source: Box::new(e),
                        })?;
                    let s = got.component.and_then(|c| c.state).unwrap_or_default();
                    if s == "DISABLED" {
                        Ok(Some(()))
                    } else {
                        Ok(None)
                    }
                }
            },
        )
        .await?;
    }
    Ok(())
}

async fn delete_all_child_pgs(client: &DynamicClient) -> Result<()> {
    let entity = client
        .processgroups_api()
        .process_groups("root")
        .get_process_groups()
        .await
        .map_err(|e| SeederError::Api {
            message: "list root child PGs".into(),
            source: Box::new(e),
        })?;

    let pgs = entity.process_groups.unwrap_or_default();
    for pg in pgs {
        let Some(id) = pg
            .component
            .as_ref()
            .and_then(|c| c.id.clone())
            .or_else(|| pg.id.clone())
        else {
            continue;
        };

        // Empty all connections under this PG before deleting — NiFi
        // refuses to delete PGs whose connections still have queued
        // flowfiles (e.g. the backpressure fixture queue). The
        // empty-all-connections request is fire-and-forget; flowfiles
        // drop within ~100ms.
        if let Err(e) = client
            .processgroups_api()
            .empty_all_connections_requests(&id)
            .create_empty_all_connections_request()
            .await
        {
            tracing::debug!(%id, error = %e, "empty-all-connections request failed; continuing");
        }

        let version = pg
            .revision
            .as_ref()
            .and_then(|r| r.version)
            .map(|v| v.to_string());
        client
            .processgroups_api()
            .remove_process_group(&id, version.as_deref(), None, None)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("delete process group {id}"),
                source: Box::new(e),
            })?;
    }
    Ok(())
}

async fn delete_all_root_controller_services(client: &DynamicClient) -> Result<()> {
    // Only the services directly on root (no descendants) — the child PG
    // delete pass above cascade-deletes any services inside them.
    let entity = client
        .flow_api()
        .controller_services("root")
        .get_controller_services_from_group(Some(false), Some(false), Some(false), Some(false))
        .await
        .map_err(|e| SeederError::Api {
            message: "list root controller services".into(),
            source: Box::new(e),
        })?;

    let services = entity.controller_services.unwrap_or_default();
    for svc in services {
        let Some(id) = svc.id.clone() else { continue };
        let version = svc
            .revision
            .as_ref()
            .and_then(|r| r.version)
            .map(|v| v.to_string());
        client
            .controller_services_api()
            .remove_controller_service(&id, version.as_deref(), None, None)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("delete controller service {id}"),
                source: Box::new(e),
            })?;
    }
    Ok(())
}
