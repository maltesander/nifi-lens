//! Controller service fixture: one ENABLED JsonTreeReader, one DISABLED
//! CSVReader, one INVALID JsonRecordSetWriter.

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_controller_service, props};
use crate::error::{Result, SeederError};
use crate::state::poll_until;

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    create_enabled_json_reader(client, parent_pg_id).await?;
    create_disabled_csv_reader(client, parent_pg_id).await?;
    create_invalid_json_writer(client, parent_pg_id).await?;
    Ok(())
}

async fn create_enabled_json_reader(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("creating fixture-json-reader (to be ENABLED)");
    let body = make_controller_service(
        "fixture-json-reader",
        "org.apache.nifi.json.JsonTreeReader",
        props(&[]),
    );
    let created = client
        .processgroups()
        .create_controller_service(parent_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-json-reader".into(),
            source: Box::new(e),
        })?;
    let id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: "fixture-json-reader has no id after create".into(),
        })?;

    // Freshly created CS sits in DISABLED. Poll to confirm validation finished.
    let id_poll = id.clone();
    poll_until(
        "fixture-json-reader",
        "DISABLED (validated)",
        Duration::from_secs(15),
        || {
            let id = id_poll.clone();
            async move {
                let got = client
                    .controller_services()
                    .get_controller_service(&id, None)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: "poll json reader validation".into(),
                        source: Box::new(e),
                    })?;
                let state = got.component.and_then(|c| c.state).unwrap_or_default();
                if state == "DISABLED" {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await?;

    // Flip to ENABLED — fetch the current revision first; NiFi uses
    // optimistic concurrency and rejects stale revision numbers.
    let current = client
        .controller_services()
        .get_controller_service(&id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: "fetch revision for fixture-json-reader".into(),
            source: Box::new(e),
        })?;
    let revision = current.revision.ok_or_else(|| SeederError::Invariant {
        message: "fixture-json-reader has no revision".into(),
    })?;

    let mut run_status = types::ControllerServiceRunStatusEntity::default();
    run_status.state = Some("ENABLED".to_string());
    run_status.revision = Some(revision);
    client
        .controller_services()
        .update_run_status(&id, &run_status)
        .await
        .map_err(|e| SeederError::Api {
            message: "enable fixture-json-reader".into(),
            source: Box::new(e),
        })?;

    let id_poll = id.clone();
    poll_until(
        "fixture-json-reader",
        "ENABLED",
        Duration::from_secs(30),
        || {
            let id = id_poll.clone();
            async move {
                let got = client
                    .controller_services()
                    .get_controller_service(&id, None)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: "poll json reader enable".into(),
                        source: Box::new(e),
                    })?;
                let state = got.component.and_then(|c| c.state).unwrap_or_default();
                if state == "ENABLED" {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await?;

    tracing::info!(%id, "fixture-json-reader ENABLED");
    Ok(())
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
    // continue rather than failing the whole seed. Phase 3 rendering
    // tolerates either state; Phase 1 gets its "invalid" bucket from
    // invalid-pipeline's processor.
    // Poll for any terminal state. If the CS ends up valid, that's fine —
    // Phase 3 rendering tolerates either state; Phase 1 gets its "invalid"
    // bucket from invalid-pipeline's processor. If the poll times out
    // we swallow the error and log: the CS exists, which is what we need.
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
