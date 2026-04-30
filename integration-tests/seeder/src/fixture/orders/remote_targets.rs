//! Sibling subtree under the marker PG hosting the two RPG-target input
//! ports for the orders-pipeline regional offload story.
//!
//! Each port feeds a LogAttribute-INFO; the LogAttribute auto-terminates
//! `success`. The RPGs in `sink-eu` / `sink-apac` connect to these ports
//! via the cluster's own Site-to-Site endpoint (self-targeting RPGs).
//!
//! Returns `(incoming_eu_port_id, incoming_apac_port_id)`.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_port, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::common::{
    create_connection_in_pg, create_processor, start_input_port, start_processor, wait_for_valid,
};

pub async fn seed(client: &DynamicClient, pg_id: &str) -> Result<(String, String)> {
    tracing::info!("seeding remote-targets");

    let incoming_eu_id = build_target(client, pg_id, "incoming-eu").await?;
    let incoming_apac_id = build_target(client, pg_id, "incoming-apac").await?;

    Ok((incoming_eu_id, incoming_apac_id))
}

async fn build_target(client: &DynamicClient, pg_id: &str, port_name: &str) -> Result<String> {
    // Nested input ports inside a child PG normally need a parent-level
    // incoming connection to start (NiFi rejects start_input_port with
    // "Port has no incoming connections" otherwise). Setting
    // allow_remote_access = true marks the port as a Site-to-Site receive
    // endpoint — S2S provides the implicit input source, so NiFi both
    // exposes the port for RPG discovery via handshake AND allows it to
    // start without local incoming wiring.
    let mut body = make_port(port_name);
    if let Some(component) = body.component.as_mut() {
        component.allow_remote_access = Some(true);
    }
    let created = client
        .processgroups()
        .create_input_port(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create remote-accessible input port {port_name}"),
            source: Box::new(e),
        })?;
    let port_id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("input port {port_name} has no id"),
        })?;
    let log_id = create_processor(
        client,
        pg_id,
        make_processor(
            &format!("LogAttribute-INFO-{port_name}"),
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        port_name,
    )
    .await?;

    create_connection_in_pg(
        client,
        pg_id,
        &port_id,
        "INPUT_PORT",
        &log_id,
        "PROCESSOR",
        vec![],
    )
    .await?;

    wait_for_valid(client, &log_id, port_name).await?;
    start_processor(client, &log_id).await?;
    start_input_port(client, &port_id).await?;

    Ok(port_id)
}
