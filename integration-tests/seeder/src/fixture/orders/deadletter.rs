//! deadletter/ — captures failed flowfiles from transform/UpdateRecord-fx-rate.
//!
//! Stages:
//!   in-failed (input port)
//!   LogAttribute-WARN (auto-terminate success)
//!
//! No parameter-context binding — pure logging consumer. Once the FX-rate
//! parameter is broken (phase 8), every transform output that fails routes
//! here, producing WARN bulletins on the LogAttribute. The connection's
//! queue retains the failed flowfiles for content-modal inspection.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_in_pg, create_input_port, create_processor,
    start_input_port, start_processor, wait_for_valid,
};

pub struct DeadletterIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(client: &DynamicClient, orders_pg_id: &str) -> Result<DeadletterIds> {
    tracing::info!("seeding orders-pipeline/deadletter");

    let pg_id = create_child_pg(client, orders_pg_id, "deadletter").await?;
    let in_port_id = create_input_port(client, &pg_id, "in-failed").await?;

    let log_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-WARN",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "warn"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-WARN",
    )
    .await?;

    create_connection_in_pg(
        client,
        &pg_id,
        &in_port_id,
        "INPUT_PORT",
        &log_id,
        "PROCESSOR",
        vec![],
    )
    .await?;

    wait_for_valid(client, &log_id, "LogAttribute-WARN").await?;
    start_processor(client, &log_id).await?;
    start_input_port(client, &in_port_id).await?;

    Ok(DeadletterIds { pg_id, in_port_id })
}
