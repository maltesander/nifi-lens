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
//!   5. Delete all parameter contexts. Contexts are cluster-scoped (not
//!      nested under any PG), so they survive step 3 and must be cleaned
//!      up explicitly. Deletion is only legal once no PG binds to the
//!      context, which is why this runs after step 3.
//!
//! Step 3 subsumes the other component types because deleting a PG
//! deletes its contents atomically.

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::error::{Result, SeederError};
use crate::state::poll_until;

/// Delete every process group and controller service beneath root,
/// leaving the cluster in the same state as a freshly started NiFi.
pub async fn nuke_and_repave(client: &DynamicClient) -> Result<()> {
    tracing::info!("nuke-and-repave: stopping all processors under root");
    stop_all_under_root(client).await?;

    tracing::info!("nuke-and-repave: disabling controller services");
    disable_all_controller_services(client).await?;

    tracing::info!("nuke-and-repave: stopping transmitting RPGs");
    stop_all_rpgs_recursively(client, "root").await?;

    tracing::info!("nuke-and-repave: deleting child process groups");
    delete_all_child_pgs(client).await?;

    tracing::info!("nuke-and-repave: deleting root controller services");
    delete_all_root_controller_services(client).await?;

    tracing::info!("nuke-and-repave: deleting parameter contexts");
    delete_all_parameter_contexts(client).await?;

    tracing::info!("nuke-and-repave: complete");
    Ok(())
}

async fn delete_all_parameter_contexts(client: &DynamicClient) -> Result<()> {
    // Parameter contexts are cluster-scoped — they aren't nested under any
    // PG, so the child-PG delete pass leaves them behind. Deletion is also
    // gated by inter-context references: a context inherited by another
    // (e.g. `fixture-pc-base` referenced by `fixture-pc-prod`) can only be
    // deleted after every inheritor is gone.
    //
    // Multi-pass deletion handles both ordering constraints uniformly:
    // each pass refetches the list and tries to delete every remaining
    // context. NiFi 409s the ones still referenced; surviving contexts go
    // into the next pass. The loop terminates when the list is empty
    // (success) or no pass makes progress (genuine error).
    const MAX_PASSES: usize = 8;
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;

    for _ in 0..MAX_PASSES {
        let entity =
            client
                .flow()
                .get_parameter_contexts()
                .await
                .map_err(|e| SeederError::Api {
                    message: "list parameter contexts".into(),
                    source: Box::new(e),
                })?;

        let contexts = entity.parameter_contexts.unwrap_or_default();
        if contexts.is_empty() {
            return Ok(());
        }

        let mut progress = false;
        for ctx in contexts {
            let Some(id) = ctx.id.clone() else { continue };
            let version = ctx
                .revision
                .as_ref()
                .and_then(|r| r.version)
                .map(|v| v.to_string());
            match client
                .parametercontexts()
                .delete_parameter_context(&id, version.as_deref(), None, None)
                .await
            {
                Ok(_) => {
                    progress = true;
                }
                Err(e) => {
                    // Likely 409 — context still referenced by an inheritor.
                    // Retry on the next pass after the inheritor is deleted.
                    last_error = Some(Box::new(e));
                }
            }
        }

        if !progress {
            return Err(SeederError::Api {
                message: "delete parameter contexts (no progress; check inheritance topology)"
                    .into(),
                source: last_error
                    .unwrap_or_else(|| Box::new(std::io::Error::other("no error captured"))),
            });
        }
    }

    Err(SeederError::Api {
        message: format!(
            "delete parameter contexts (exceeded {MAX_PASSES} passes — possible cycle?)"
        ),
        source: last_error.unwrap_or_else(|| Box::new(std::io::Error::other("no error captured"))),
    })
}

async fn stop_all_under_root(client: &DynamicClient) -> Result<()> {
    // `PUT /flow/process-groups/{id}` with state=STOPPED and no explicit
    // `components` map propagates to all authorized descendants.
    let mut body = types::ScheduleComponentsEntity::default();
    body.id = Some("root".to_string());
    body.state = Some("STOPPED".to_string());

    client
        .flow()
        .schedule_components("root", &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "stop all processors under root".into(),
            source: Box::new(e),
        })?;

    // The bulk schedule_components call doesn't tear down NiFi's internal
    // Site-to-Site receive plumbing for input ports with allow_remote_access
    // = true. NiFi creates an internal "receive" connection (not visible
    // via /connections/) whose destination is the receive port; if the
    // port stays RUNNING, the parent PG cannot be deleted (409
    // "Destination of Connection X is running"). Walking the tree and
    // stopping every input/output port via its own /run-status endpoint
    // forces NiFi to tear down the internal plumbing too.
    tokio::time::sleep(Duration::from_secs(2)).await;
    stop_all_ports_recursively(client, "root").await?;
    Ok(())
}

/// Recursively walk every PG under `pg_id` and explicitly stop each
/// input/output port via its own `/run-status` endpoint. Best-effort —
/// failures (e.g. already stopped) are logged at debug and skipped.
fn stop_all_ports_recursively<'a>(
    client: &'a DynamicClient,
    pg_id: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        // Recurse into child PGs first (depth-first) so a child's S2S
        // receive plumbing is dismantled before its parent's delete.
        let entity = client
            .processgroups()
            .get_process_groups(pg_id)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("list child PGs of {pg_id} for port-stop walk"),
                source: Box::new(e),
            })?;
        for child in entity.process_groups.unwrap_or_default() {
            let Some(child_id) = child
                .component
                .as_ref()
                .and_then(|c| c.id.clone())
                .or_else(|| child.id.clone())
            else {
                continue;
            };
            stop_all_ports_recursively(client, &child_id).await?;
        }

        // Stop input ports in this PG.
        let inputs = client
            .processgroups()
            .get_input_ports(pg_id)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("list input ports of {pg_id}"),
                source: Box::new(e),
            })?;
        for port in inputs.input_ports.unwrap_or_default() {
            let Some(port_id) = port
                .component
                .as_ref()
                .and_then(|c| c.id.clone())
                .or_else(|| port.id.clone())
            else {
                continue;
            };
            let mut body = types::PortRunStatusEntity::default();
            body.state = Some("STOPPED".to_string());
            body.revision = port.revision.clone();
            if let Err(e) = client.inputports().update_run_status(&port_id, &body).await {
                tracing::debug!(%port_id, error = %e, "input port stop failed (may already be stopped)");
            }
        }

        // Stop output ports in this PG.
        let outputs = client
            .processgroups()
            .get_output_ports(pg_id)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("list output ports of {pg_id}"),
                source: Box::new(e),
            })?;
        for port in outputs.output_ports.unwrap_or_default() {
            let Some(port_id) = port
                .component
                .as_ref()
                .and_then(|c| c.id.clone())
                .or_else(|| port.id.clone())
            else {
                continue;
            };
            let mut body = types::PortRunStatusEntity::default();
            body.state = Some("STOPPED".to_string());
            body.revision = port.revision.clone();
            if let Err(e) = client
                .outputports()
                .update_run_status(&port_id, &body)
                .await
            {
                tracing::debug!(%port_id, error = %e, "output port stop failed (may already be stopped)");
            }
        }

        Ok(())
    })
}

/// Recursively walk every PG under `pg_id` and set `transmitting=false`
/// on each Remote Process Group. An RPG with `transmitting=true` keeps
/// its internal input-port mapping in a "running" state; the mapping is
/// the destination of an internal `PROCESSOR -> REMOTE_INPUT_PORT`
/// connection (not visible via `/connections/`). Cascade-delete of the
/// parent PG fails with 409 "Destination of Connection X is running"
/// where X is the mapping's id. The bulk `schedule_components(STOPPED)`
/// call doesn't reach RPG transmission state — only the per-RPG
/// `/run-status` endpoint does. Stop them explicitly here.
fn stop_all_rpgs_recursively<'a>(
    client: &'a DynamicClient,
    pg_id: &'a str,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<()>> + Send + 'a>> {
    Box::pin(async move {
        let entity = client
            .processgroups()
            .get_process_groups(pg_id)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("list child PGs of {pg_id} for RPG-stop walk"),
                source: Box::new(e),
            })?;
        for child in entity.process_groups.unwrap_or_default() {
            let Some(child_id) = child
                .component
                .as_ref()
                .and_then(|c| c.id.clone())
                .or_else(|| child.id.clone())
            else {
                continue;
            };
            stop_all_rpgs_recursively(client, &child_id).await?;
        }

        let rpgs = client
            .processgroups()
            .get_remote_process_groups(pg_id)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("list RPGs of {pg_id}"),
                source: Box::new(e),
            })?;
        for rpg in rpgs.remote_process_groups.unwrap_or_default() {
            let transmitting = rpg
                .component
                .as_ref()
                .and_then(|c| c.transmitting)
                .unwrap_or(false);
            if !transmitting {
                continue;
            }
            let Some(rpg_id) = rpg
                .component
                .as_ref()
                .and_then(|c| c.id.clone())
                .or_else(|| rpg.id.clone())
            else {
                continue;
            };

            // Re-fetch revision — the bulk schedule_components above may
            // have advanced it.
            let current = client
                .remoteprocessgroups()
                .get_remote_process_group(&rpg_id)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("get RPG {rpg_id} for revision"),
                    source: Box::new(e),
                })?;
            let revision = current.revision.unwrap_or_default();

            let mut body = types::RemotePortRunStatusEntity::default();
            body.revision = Some(revision);
            body.state = Some(
                types::RemotePortRunStatusEntityState::Stopped
                    .as_str()
                    .to_string(),
            );

            if let Err(e) = client
                .remoteprocessgroups()
                .update_remote_process_group_run_status(&rpg_id, &body)
                .await
            {
                tracing::debug!(%rpg_id, error = %e, "stop RPG failed (may already be stopped)");
            }
        }

        Ok(())
    })
}

async fn disable_all_controller_services(client: &DynamicClient) -> Result<()> {
    // List every CS reachable from root (recursive).
    let entity = client
        .flow()
        .get_controller_services_from_group(
            "root",
            Some(false),
            Some(true),
            Some(false),
            Some(false),
        )
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
            .controller_services()
            .update_run_status(&id, &run_status)
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
                        .controller_services()
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
    // Multi-pass delete handles the race between `schedule_components(STOPPED)`
    // (fire-and-forget) and the per-PG remove call: NiFi rejects deletion of
    // a PG whose connections' destination ports/processors are still
    // transitioning from RUNNING to STOPPED with "Destination of Connection
    // X is running". The error is transient — within a few hundred ms the
    // stop propagates cluster-wide. Each pass refetches the surviving PGs
    // and retries; if a pass makes no progress we surface the last error.
    const MAX_PASSES: usize = 12;
    const STALL_THRESHOLD: usize = 4; // consecutive passes with no progress
    let mut last_error: Option<Box<dyn std::error::Error + Send + Sync>> = None;
    let mut stalled = 0usize;

    for attempt in 0..MAX_PASSES {
        let entity = client
            .processgroups()
            .get_process_groups("root")
            .await
            .map_err(|e| SeederError::Api {
                message: "list root child PGs".into(),
                source: Box::new(e),
            })?;

        let pgs = entity.process_groups.unwrap_or_default();
        if pgs.is_empty() {
            return Ok(());
        }

        let mut progress = false;
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
                .processgroups()
                .create_empty_all_connections_request(&id)
                .await
            {
                tracing::debug!(%id, error = %e, "empty-all-connections request failed; continuing");
            }

            let version = pg
                .revision
                .as_ref()
                .and_then(|r| r.version)
                .map(|v| v.to_string());
            match client
                .processgroups()
                .remove_process_group(&id, version.as_deref(), None, None)
                .await
            {
                Ok(_) => {
                    progress = true;
                }
                Err(e) => {
                    tracing::debug!(%id, error = %e, "delete PG retry; component still stopping");
                    last_error = Some(Box::new(e));
                }
            }
        }

        if progress {
            stalled = 0;
        } else {
            stalled += 1;
            if stalled >= STALL_THRESHOLD {
                return Err(SeederError::Api {
                    message: format!(
                        "delete child PGs (no progress on {STALL_THRESHOLD} consecutive passes; \
                         last attempt was pass {attempt})"
                    ),
                    source: last_error
                        .unwrap_or_else(|| Box::new(std::io::Error::other("no error captured"))),
                });
            }
        }

        // Pause before the next pass — gives NiFi time to propagate the
        // stop across the cluster before we retry survivors.
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    Err(SeederError::Api {
        message: format!("delete child PGs (exceeded {MAX_PASSES} passes)"),
        source: last_error.unwrap_or_else(|| Box::new(std::io::Error::other("no error captured"))),
    })
}

async fn delete_all_root_controller_services(client: &DynamicClient) -> Result<()> {
    // Only the services directly on root (no descendants) — the child PG
    // delete pass above cascade-deletes any services inside them.
    let entity = client
        .flow()
        .get_controller_services_from_group(
            "root",
            Some(false),
            Some(false),
            Some(false),
            Some(false),
        )
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
            .controller_services()
            .remove_controller_service(&id, version.as_deref(), None, None)
            .await
            .map_err(|e| SeederError::Api {
                message: format!("delete controller service {id}"),
                source: Box::new(e),
            })?;
    }
    Ok(())
}
