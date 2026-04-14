//! Healthy-pipeline fixture: nested process groups, rich JSON ingest,
//! cross-PG connection, enrichment, INFO-level logging.
//!
//! Topology:
//!
//! ```text
//! healthy-pipeline/
//! ├── ingest/ (child PG)
//! │   ├── GenerateFlowFile (1 sec, Custom Text = HEALTHY_INGEST_CUSTOM_TEXT)
//! │   ├── UpdateAttribute-ingest
//! │   └── output port "ingest-out"
//! ├── enrich/ (child PG)
//! │   ├── input port "enrich-in"
//! │   ├── UpdateAttribute-enrich
//! │   └── LogAttribute-INFO
//! └── (parent-level connection: ingest/ingest-out -> enrich/enrich-in)
//! ```

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_connection, make_pg, make_port, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::payload::HEALTHY_INGEST_CUSTOM_TEXT;
use crate::state::poll_until;

/// Kind of port used when starting it.
#[derive(Clone, Copy)]
pub(crate) enum PortKind {
    Input,
    Output,
}

/// Build the complete healthy-pipeline topology under `parent_pg_id` and
/// start every component in it.
pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding healthy-pipeline");

    // Parent PG.
    let healthy_pg_id = create_child_pg(client, parent_pg_id, "healthy-pipeline").await?;

    // Ingest child PG.
    let ingest_pg_id = create_child_pg(client, &healthy_pg_id, "ingest").await?;
    let gen_id = create_processor(
        client,
        &ingest_pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            // NiFi 2.8.0 added a validation rule that rejects
            // (Custom Text + Unique FlowFiles=true). Setting it to
            // false satisfies both 2.6.0 and 2.8.0.
            props(&[
                ("Custom Text", HEALTHY_INGEST_CUSTOM_TEXT),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("1 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;
    let ingest_ua_id = create_processor(
        client,
        &ingest_pg_id,
        make_processor(
            "UpdateAttribute-ingest",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("stage", "ingest"),
                ("fixture.ingest.timestamp", "${now():toNumber()}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-ingest",
    )
    .await?;
    let ingest_out_id = create_output_port(client, &ingest_pg_id, "ingest-out").await?;

    create_connection_in_pg(
        client,
        &ingest_pg_id,
        &gen_id,
        "PROCESSOR",
        &ingest_ua_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &ingest_pg_id,
        &ingest_ua_id,
        "PROCESSOR",
        &ingest_out_id,
        "OUTPUT_PORT",
        vec!["success"],
    )
    .await?;

    // Enrich child PG.
    let enrich_pg_id = create_child_pg(client, &healthy_pg_id, "enrich").await?;
    let enrich_in_id = create_input_port(client, &enrich_pg_id, "enrich-in").await?;
    let enrich_ua_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "UpdateAttribute-enrich",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("stage", "enrich"),
                (
                    "severity",
                    "${random():mod(3):equals(0):ifElse('INFO','WARN')}",
                ),
                ("fixture.enrich.timestamp", "${now():toNumber()}"),
                ("fixture.tag", "synthetic-enriched"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-enrich",
    )
    .await?;
    let log_attr_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "LogAttribute-INFO",
            "org.apache.nifi.processors.standard.LogAttribute",
            // LogAttribute uses legacy display-name property keys in
            // NiFi 2.x. "Log Prefix" differs in capitalization between
            // 2.6.0 (`Log prefix`) and 2.8.0 (`Log Prefix`), so we omit
            // it — the default (empty prefix) is fine for the fixture.
            props(&[("Log Level", "info"), ("Log Payload", "true")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-INFO",
    )
    .await?;

    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &enrich_in_id,
        "INPUT_PORT",
        &enrich_ua_id,
        "PROCESSOR",
        vec![],
    )
    .await?;
    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &enrich_ua_id,
        "PROCESSOR",
        &log_attr_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // Parent-level cross-PG connection from ingest-out -> enrich-in. The
    // connection lives on the healthy-pipeline PG, but each port's
    // `group_id` must point to the child PG that owns it.
    create_connection_between(
        client,
        &healthy_pg_id,
        &ingest_pg_id,
        &ingest_out_id,
        "OUTPUT_PORT",
        &enrich_pg_id,
        &enrich_in_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;

    // Start everything. Downstream first so nothing backs up on startup.
    // Enrich first.
    wait_for_valid(client, &log_attr_id, "LogAttribute-INFO").await?;
    start_processor(client, &log_attr_id).await?;
    wait_for_valid(client, &enrich_ua_id, "UpdateAttribute-enrich").await?;
    start_processor(client, &enrich_ua_id).await?;
    start_input_port(client, &enrich_in_id).await?;

    // Ingest second.
    wait_for_valid(client, &ingest_ua_id, "UpdateAttribute-ingest").await?;
    start_processor(client, &ingest_ua_id).await?;
    start_output_port(client, &ingest_out_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("healthy-pipeline seeded and running");
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared helpers re-used by noisy / backpressure / invalid modules.
// ---------------------------------------------------------------------------

pub(crate) async fn create_child_pg(
    client: &DynamicClient,
    parent_pg_id: &str,
    name: &str,
) -> Result<String> {
    let body = make_pg(name);
    let created = client
        .processgroups()
        .create_process_group(parent_pg_id, None, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create child PG {name}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("child PG {name} has no id after create"),
        })
}

pub(crate) async fn create_processor(
    client: &DynamicClient,
    pg_id: &str,
    body: types::ProcessorEntity,
    name: &str,
) -> Result<String> {
    let created = client
        .processgroups()
        .create_processor(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create processor {name}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("processor {name} has no id after create"),
        })
}

pub(crate) async fn create_input_port(
    client: &DynamicClient,
    pg_id: &str,
    name: &str,
) -> Result<String> {
    let body = make_port(name);
    let created = client
        .processgroups()
        .create_input_port(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create input port {name}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("input port {name} has no id"),
        })
}

pub(crate) async fn create_output_port(
    client: &DynamicClient,
    pg_id: &str,
    name: &str,
) -> Result<String> {
    let body = make_port(name);
    let created = client
        .processgroups()
        .create_output_port(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create output port {name}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("output port {name} has no id"),
        })
}

/// Create a connection where source and destination live in the same PG
/// (the typical intra-PG case).
pub(crate) async fn create_connection_in_pg(
    client: &DynamicClient,
    pg_id: &str,
    source_id: &str,
    source_type: &str,
    destination_id: &str,
    destination_type: &str,
    relationships: Vec<&str>,
) -> Result<String> {
    create_connection_between(
        client,
        pg_id,
        pg_id,
        source_id,
        source_type,
        pg_id,
        destination_id,
        destination_type,
        relationships,
    )
    .await
}

/// Create a connection where source and destination may live in different
/// child PGs (used for cross-PG connections via ports).
///
/// * `container_pg_id` — PG that owns the connection itself
/// * `source_group_id` — PG that contains the source component
/// * `destination_group_id` — PG that contains the destination component
#[allow(clippy::too_many_arguments)]
pub(crate) async fn create_connection_between(
    client: &DynamicClient,
    container_pg_id: &str,
    source_group_id: &str,
    source_id: &str,
    source_type: &str,
    destination_group_id: &str,
    destination_id: &str,
    destination_type: &str,
    relationships: Vec<&str>,
) -> Result<String> {
    tracing::debug!(
        %container_pg_id,
        %source_group_id,
        %source_id,
        %source_type,
        %destination_group_id,
        %destination_id,
        %destination_type,
        "creating connection"
    );
    let body = make_connection(
        source_group_id,
        source_id,
        source_type,
        destination_group_id,
        destination_id,
        destination_type,
        relationships,
    );
    let created = client
        .processgroups()
        .create_connection(container_pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create connection in pg {container_pg_id}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("connection in pg {container_pg_id} has no id"),
        })
}

/// Poll until the processor has `component.state == "STOPPED"` and no
/// validation errors.
pub(crate) async fn wait_for_valid(
    client: &DynamicClient,
    processor_id: &str,
    name: &str,
) -> Result<()> {
    let what = format!("processor {name}");
    let id = processor_id.to_string();
    poll_until(what, "VALID+STOPPED", Duration::from_secs(15), || {
        let id = id.clone();
        async move {
            let got =
                client
                    .processors()
                    .get_processor(&id)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: format!("poll processor {id} validation"),
                        source: Box::new(e),
                    })?;
            let Some(component) = got.component else {
                return Ok(None);
            };
            let state = component.state.clone().unwrap_or_default();
            let errs = component.validation_errors.clone().unwrap_or_default();
            if state == "STOPPED" && errs.is_empty() {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    })
    .await
}

pub(crate) async fn start_processor(client: &DynamicClient, id: &str) -> Result<()> {
    // NiFi uses optimistic concurrency — we must send the current revision,
    // not a hardcoded 0. Fetch it just before the PUT.
    let current = client
        .processors()
        .get_processor(id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch revision for processor {id}"),
            source: Box::new(e),
        })?;
    let revision = current.revision.ok_or_else(|| SeederError::Invariant {
        message: format!("processor {id} has no revision"),
    })?;

    let mut body = types::ProcessorRunStatusEntity::default();
    body.state = Some("RUNNING".to_string());
    body.revision = Some(revision);

    client
        .processors()
        .update_run_status(id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("start processor {id}"),
            source: Box::new(e),
        })?;

    let id_owned = id.to_string();
    poll_until(
        format!("processor {id}"),
        "RUNNING",
        Duration::from_secs(30),
        || {
            let id = id_owned.clone();
            async move {
                let got =
                    client
                        .processors()
                        .get_processor(&id)
                        .await
                        .map_err(|e| SeederError::Api {
                            message: format!("poll processor {id} run"),
                            source: Box::new(e),
                        })?;
                let state = got.component.and_then(|c| c.state).unwrap_or_default();
                if state == "RUNNING" {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await
}

pub(crate) async fn start_input_port(client: &DynamicClient, id: &str) -> Result<()> {
    start_port(client, id, PortKind::Input).await
}

pub(crate) async fn start_output_port(client: &DynamicClient, id: &str) -> Result<()> {
    start_port(client, id, PortKind::Output).await
}

async fn start_port(client: &DynamicClient, id: &str, kind: PortKind) -> Result<()> {
    // Fetch current revision — NiFi rejects stale revisions on PUT.
    let revision = match kind {
        PortKind::Input => {
            let current =
                client
                    .inputports()
                    .get_input_port(id)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: format!("fetch revision for input port {id}"),
                        source: Box::new(e),
                    })?;
            current.revision.ok_or_else(|| SeederError::Invariant {
                message: format!("input port {id} has no revision"),
            })?
        }
        PortKind::Output => {
            let current = client
                .outputports()
                .get_output_port(id)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("fetch revision for output port {id}"),
                    source: Box::new(e),
                })?;
            current.revision.ok_or_else(|| SeederError::Invariant {
                message: format!("output port {id} has no revision"),
            })?
        }
    };

    let mut body = types::PortRunStatusEntity::default();
    body.state = Some("RUNNING".to_string());
    body.revision = Some(revision);

    match kind {
        PortKind::Input => {
            client
                .inputports()
                .update_run_status(id, &body)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("start input port {id}"),
                    source: Box::new(e),
                })?;
        }
        PortKind::Output => {
            client
                .outputports()
                .update_run_status(id, &body)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("start output port {id}"),
                    source: Box::new(e),
                })?;
        }
    }

    let id_owned = id.to_string();
    poll_until(
        format!("port {id}"),
        "RUNNING",
        Duration::from_secs(30),
        || {
            let id = id_owned.clone();
            async move {
                let state = match kind {
                    PortKind::Input => client
                        .inputports()
                        .get_input_port(&id)
                        .await
                        .map_err(|e| SeederError::Api {
                            message: format!("poll input port {id}"),
                            source: Box::new(e),
                        })?
                        .component
                        .and_then(|c| c.state)
                        .unwrap_or_default(),
                    PortKind::Output => client
                        .outputports()
                        .get_output_port(&id)
                        .await
                        .map_err(|e| SeederError::Api {
                            message: format!("poll output port {id}"),
                            source: Box::new(e),
                        })?
                        .component
                        .and_then(|c| c.state)
                        .unwrap_or_default(),
                };
                if state == "RUNNING" {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await
}
