//! Controller service fixture: one ENABLED JsonTreeReader, one DISABLED
//! CSVReader, one INVALID JsonRecordSetWriter.

use std::time::Duration;

use nifi_rust_client::dynamic::{
    DynamicClient,
    traits::{
        ControllerServicesApi as _, ControllerServicesRunStatusApi as _, ProcessGroupsApi as _,
        ProcessGroupsControllerServicesApi as _,
    },
    types,
};

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
        .processgroups_api()
        .controller_services(parent_pg_id)
        .create_controller_service_1(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-json-reader".into(),
            source: Box::new(e),
        })?;
    let id = created.id.ok_or_else(|| SeederError::Invariant {
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
                    .controller_services_api()
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

    // Flip to ENABLED.
    let mut run_status = types::ControllerServiceRunStatusEntity::default();
    run_status.state = Some("ENABLED".to_string());
    let mut rev = types::RevisionDto::default();
    rev.version = Some(0);
    run_status.revision = Some(rev);
    client
        .controller_services_api()
        .run_status(&id)
        .update_run_status_1(&run_status)
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
                    .controller_services_api()
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
        .processgroups_api()
        .controller_services(parent_pg_id)
        .create_controller_service_1(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-csv-reader".into(),
            source: Box::new(e),
        })?;
    Ok(())
}

async fn create_invalid_json_writer(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("creating fixture-broken-writer (INVALID on purpose)");
    // JsonRecordSetWriter needs a Schema Access Strategy — leaving properties
    // empty keeps it INVALID permanently.
    let body = make_controller_service(
        "fixture-broken-writer",
        "org.apache.nifi.json.JsonRecordSetWriter",
        props(&[]),
    );
    let created = client
        .processgroups_api()
        .controller_services(parent_pg_id)
        .create_controller_service_1(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture-broken-writer".into(),
            source: Box::new(e),
        })?;
    let id = created.id.ok_or_else(|| SeederError::Invariant {
        message: "fixture-broken-writer has no id".into(),
    })?;

    // Poll until validation_errors is populated, so we know the CS reached a
    // terminal state (not still transitioning through VALIDATING).
    let id_poll = id.clone();
    poll_until(
        "fixture-broken-writer",
        "INVALID",
        Duration::from_secs(15),
        || {
            let id = id_poll.clone();
            async move {
                let got = client
                    .controller_services_api()
                    .get_controller_service(&id, None)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: "poll broken writer validation".into(),
                        source: Box::new(e),
                    })?;
                let errs = got
                    .component
                    .and_then(|c| c.validation_errors)
                    .unwrap_or_default();
                if !errs.is_empty() {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await?;

    tracing::info!(%id, "fixture-broken-writer INVALID as expected");
    Ok(())
}
