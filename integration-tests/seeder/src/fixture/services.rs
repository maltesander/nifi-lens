//! Controller service fixture: one ENABLED JsonTreeReader, one ENABLED
//! JsonRecordSetWriter, one DISABLED CSVReader, one INVALID JsonRecordSetWriter.

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};
use nifi_rust_client::{NifiError, wait};

use crate::entities::{make_controller_service, props};
use crate::error::{Result, SeederError};
use crate::state::poll_until;

/// Map a `wait::` failure into the seeder's error vocabulary. Timeouts
/// surface as `SeederError::StateTimeout` (carrying the configured
/// timeout as `elapsed_secs`, matching the seeder's own `poll_until`
/// convention); all other errors become `SeederError::Api`.
fn map_wait_err(err: NifiError, what: &str, target: &str, timeout: Duration) -> SeederError {
    match err {
        NifiError::Timeout { .. } => SeederError::StateTimeout {
            what: what.to_string(),
            target_state: target.to_string(),
            elapsed_secs: timeout.as_secs(),
        },
        other => SeederError::Api {
            message: format!("wait for {what} {target}"),
            source: Box::new(other),
        },
    }
}

/// IDs of the controller services that other fixture modules need to
/// reference (e.g. to wire ConvertRecord properties).
pub struct ServiceIds {
    pub json_reader_id: String,
    pub json_writer_id: String,
}

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<ServiceIds> {
    let json_reader_id = create_enabled_json_reader(client, parent_pg_id).await?;
    let json_writer_id = create_enabled_json_writer(client, parent_pg_id).await?;
    create_disabled_csv_reader(client, parent_pg_id).await?;
    create_invalid_json_writer(client, parent_pg_id).await?;
    Ok(ServiceIds {
        json_reader_id,
        json_writer_id,
    })
}

async fn create_enabled_json_reader(client: &DynamicClient, parent_pg_id: &str) -> Result<String> {
    create_and_enable_cs(
        client,
        parent_pg_id,
        "fixture-json-reader",
        "org.apache.nifi.json.JsonTreeReader",
    )
    .await
}

async fn create_enabled_json_writer(client: &DynamicClient, parent_pg_id: &str) -> Result<String> {
    create_and_enable_cs(
        client,
        parent_pg_id,
        "fixture-json-writer",
        "org.apache.nifi.json.JsonRecordSetWriter",
    )
    .await
}

/// Create a controller service, wait for it to reach DISABLED (validated),
/// then enable it and wait until ENABLED. Returns the CS id.
async fn create_and_enable_cs(
    client: &DynamicClient,
    parent_pg_id: &str,
    name: &str,
    cs_type: &str,
) -> Result<String> {
    tracing::info!(%name, "creating controller service (to be ENABLED)");
    let body = make_controller_service(name, cs_type, props(&[]));
    let created = client
        .processgroups()
        .create_controller_service(parent_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create {name}"),
            source: Box::new(e),
        })?;
    let id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("{name} has no id after create"),
        })?;

    // Freshly created CS sits in DISABLED. Wait for validation to finish.
    let validated_timeout = Duration::from_secs(15);
    wait::controller_service_state_dynamic(
        client,
        &id,
        wait::ControllerServiceTargetState::Disabled,
        wait::WaitConfig {
            timeout: validated_timeout,
            poll_interval: Duration::from_millis(250),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| map_wait_err(e, name, "DISABLED (validated)", validated_timeout))?;

    // Flip to ENABLED — fetch the current revision first; NiFi uses
    // optimistic concurrency and rejects stale revision numbers.
    let current = client
        .controller_services()
        .get_controller_service(&id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch revision for {name}"),
            source: Box::new(e),
        })?;
    let revision = current.revision.ok_or_else(|| SeederError::Invariant {
        message: format!("{name} has no revision"),
    })?;

    let mut run_status = types::ControllerServiceRunStatusEntity::default();
    run_status.state = Some("ENABLED".to_string());
    run_status.revision = Some(revision);
    client
        .controller_services()
        .update_run_status(&id, &run_status)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("enable {name}"),
            source: Box::new(e),
        })?;

    let enabled_timeout = Duration::from_secs(30);
    wait::controller_service_state_dynamic(
        client,
        &id,
        wait::ControllerServiceTargetState::Enabled,
        wait::WaitConfig {
            timeout: enabled_timeout,
            poll_interval: Duration::from_millis(500),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| map_wait_err(e, name, "ENABLED", enabled_timeout))?;

    tracing::info!(%id, %name, "controller service ENABLED");
    Ok(id)
}

async fn create_disabled_csv_reader(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("creating fixture-csv-reader (leave DISABLED)");
    let body = make_controller_service(
        "fixture-csv-reader",
        "org.apache.nifi.csv.CSVReader",
        props(&[]),
    );
    client
        .processgroups()
        .create_controller_service(parent_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-csv-reader".into(),
            source: Box::new(e),
        })?;
    Ok(())
}

async fn create_invalid_json_writer(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("creating fixture-broken-writer (target: INVALID)");
    // DBCPConnectionPool requires Database Connection URL + Database Driver
    // Class Name + Database Driver Location — all unset here, so the CS
    // cannot validate. Using a JDBC pool rather than a JSON reader/writer
    // because the JSON services tolerate empty properties in NiFi 2.x.
    let body = make_controller_service(
        "fixture-broken-writer",
        "org.apache.nifi.dbcp.DBCPConnectionPool",
        props(&[]),
    );
    let created = client
        .processgroups()
        .create_controller_service(parent_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-broken-writer".into(),
            source: Box::new(e),
        })?;
    let id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: "fixture-broken-writer has no id".into(),
        })?;

    // Poll for any terminal state. The CS name is already correct — if
    // validation doesn't populate errors in 15s, we log the state and
    // continue rather than failing the whole seed. Browser rendering
    // tolerates either state; the Overview "invalid" bucket gets
    // populated from invalid-pipeline's processor. If the poll times
    // out we swallow the error and log: the CS exists, which is what
    // we need.
    let id_poll = id.clone();
    let poll_result: Result<String> = poll_until(
        "fixture-broken-writer",
        "terminal state",
        Duration::from_secs(5),
        || {
            let id = id_poll.clone();
            async move {
                let got = client
                    .controller_services()
                    .get_controller_service(&id, None)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: "poll broken writer state".into(),
                        source: Box::new(e),
                    })?;
                let state = got
                    .component
                    .as_ref()
                    .and_then(|c| c.state.clone())
                    .unwrap_or_default();
                let errs = got
                    .component
                    .and_then(|c| c.validation_errors)
                    .unwrap_or_default();
                // Terminal = DISABLED (validation finished) or errors reported.
                if state == "DISABLED" || !errs.is_empty() {
                    Ok(Some(state))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await;

    match poll_result {
        Ok(state) => tracing::info!(%id, %state, "fixture-broken-writer reached terminal state"),
        Err(_) => tracing::warn!(%id, "fixture-broken-writer did not settle; continuing"),
    }
    Ok(())
}
