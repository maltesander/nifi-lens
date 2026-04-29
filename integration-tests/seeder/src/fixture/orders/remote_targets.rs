//! Sibling subtree under the marker PG hosting the two RPG-target input
//! ports for the orders-pipeline regional offload story.
//!
//! Each port feeds a LogAttribute-INFO; the LogAttribute auto-terminates
//! `success`. The RPGs in `sink-eu` / `sink-apac` connect to these ports
//! via the cluster's own Site-to-Site endpoint (self-targeting RPGs).
//!
//! Returns `(incoming_eu_port_id, incoming_apac_port_id)`.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_connection_in_pg, create_input_port, create_processor, start_input_port,
    start_processor, wait_for_valid,
};

pub async fn seed(client: &DynamicClient, pg_id: &str) -> Result<(String, String)> {
    tracing::info!("seeding remote-targets");

    let incoming_eu_id = build_target(client, pg_id, "incoming-eu").await?;
    let incoming_apac_id = build_target(client, pg_id, "incoming-apac").await?;

    Ok((incoming_eu_id, incoming_apac_id))
}

async fn build_target(client: &DynamicClient, pg_id: &str, port_name: &str) -> Result<String> {
    let port_id = create_input_port(client, pg_id, port_name).await?;
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
