//! Reporting-task fixture: one RUNNING (ControllerStatusReportingTask),
//! one STOPPED (MonitorDiskUsage configured against /opt/nifi/nifi-current/logs),
//! one INVALID (MonitorMemory missing required Memory Pool).
//!
//! Demonstrates all three Components-panel states (running / stopped /
//! invalid) and exercises the modal's validation-error rendering plus
//! the trailing `[t]` chord chip.

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_reporting_task, props};
use crate::error::{Result, SeederError};
use crate::state::poll_until;

pub async fn seed(client: &DynamicClient) -> Result<()> {
    create_running_controller_status(client).await?;
    create_stopped_disk_monitor(client).await?;
    create_invalid_memory_monitor(client).await?;
    Ok(())
}

async fn create_running_controller_status(client: &DynamicClient) -> Result<()> {
    let name = "fixture-controller-status";
    tracing::info!(%name, "creating reporting task (target: RUNNING)");
    let body = make_reporting_task(
        name,
        "org.apache.nifi.controller.ControllerStatusReportingTask",
        props(&[]),
    );
    let created = client
        .controller()
        .create_reporting_task(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create {name}"),
            source: Box::new(e),
        })?;
    let id = task_id(&created, name)?;
    wait_until_validated(client, &id, name).await?;
    start_task(client, &id, name).await?;
    Ok(())
}

async fn create_stopped_disk_monitor(client: &DynamicClient) -> Result<()> {
    let name = "fixture-disk-monitor";
    tracing::info!(%name, "creating reporting task (target: STOPPED/VALID)");
    let body = make_reporting_task(
        name,
        "org.apache.nifi.controller.MonitorDiskUsage",
        props(&[
            ("Directory Location", "/opt/nifi/nifi-current/logs"),
            ("Threshold", "90%"),
        ]),
    );
    let created = client
        .controller()
        .create_reporting_task(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create {name}"),
            source: Box::new(e),
        })?;
    let id = task_id(&created, name)?;
    // Create only — don't start. Wait for validation to settle (VALID).
    wait_until_validated(client, &id, name).await?;
    tracing::info!(%id, %name, "reporting task STOPPED/VALID");
    Ok(())
}

async fn create_invalid_memory_monitor(client: &DynamicClient) -> Result<()> {
    let name = "fixture-memory-monitor";
    tracing::info!(%name, "creating reporting task (target: INVALID)");
    // Don't set Memory Pool — required, so the task lands INVALID.
    let body = make_reporting_task(name, "org.apache.nifi.controller.MonitorMemory", props(&[]));
    let created = client
        .controller()
        .create_reporting_task(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create {name}"),
            source: Box::new(e),
        })?;
    let id = task_id(&created, name)?;
    // Not started — NiFi rejects starting an INVALID task.
    wait_until_invalid(client, &id, name).await?;
    tracing::info!(%id, %name, "reporting task INVALID");
    Ok(())
}

fn task_id(entity: &types::ReportingTaskEntity, name: &str) -> Result<String> {
    entity
        .component
        .as_ref()
        .and_then(|c| c.id.clone())
        .or_else(|| entity.id.clone())
        .ok_or_else(|| SeederError::Invariant {
            message: format!("{name} has no id after create"),
        })
}

async fn wait_until_validated(client: &DynamicClient, id: &str, name: &str) -> Result<()> {
    let timeout = Duration::from_secs(15);
    let id = id.to_string();
    poll_until(name, "VALID", timeout, move || {
        let id = id.clone();
        async move {
            let got = client
                .reportingtasks()
                .get_reporting_task(&id)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("poll validation for {id}"),
                    source: Box::new(e),
                })?;
            let status = got
                .component
                .as_ref()
                .and_then(|c| c.validation_status.clone())
                .unwrap_or_default();
            if status == "VALID" {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    })
    .await
}

async fn wait_until_invalid(client: &DynamicClient, id: &str, name: &str) -> Result<()> {
    let timeout = Duration::from_secs(15);
    let id = id.to_string();
    poll_until(name, "INVALID", timeout, move || {
        let id = id.clone();
        async move {
            let got = client
                .reportingtasks()
                .get_reporting_task(&id)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("poll validation for {id}"),
                    source: Box::new(e),
                })?;
            let status = got
                .component
                .as_ref()
                .and_then(|c| c.validation_status.clone())
                .unwrap_or_default();
            if status == "INVALID" {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    })
    .await
}

async fn start_task(client: &DynamicClient, id: &str, name: &str) -> Result<()> {
    // Fetch the current revision — NiFi uses optimistic concurrency and
    // rejects stale revision numbers.
    let current = client
        .reportingtasks()
        .get_reporting_task(id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch revision for {name}"),
            source: Box::new(e),
        })?;
    let revision = current.revision.ok_or_else(|| SeederError::Invariant {
        message: format!("{name} has no revision"),
    })?;

    let mut run_status = types::ReportingTaskRunStatusEntity::default();
    run_status.state = Some("RUNNING".to_string());
    run_status.revision = Some(revision);
    client
        .reportingtasks()
        .update_run_status(id, &run_status)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("start {name}"),
            source: Box::new(e),
        })?;

    let timeout = Duration::from_secs(15);
    let id_poll = id.to_string();
    poll_until(name, "RUNNING", timeout, move || {
        let id = id_poll.clone();
        async move {
            let got = client
                .reportingtasks()
                .get_reporting_task(&id)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("poll run state for {id}"),
                    source: Box::new(e),
                })?;
            let state = got
                .component
                .as_ref()
                .and_then(|c| c.state.clone())
                .unwrap_or_default();
            if state == "RUNNING" {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    })
    .await?;

    tracing::info!(%id, %name, "reporting task RUNNING");
    Ok(())
}
