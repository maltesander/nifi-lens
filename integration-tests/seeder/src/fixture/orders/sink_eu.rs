//! sink-eu/ — bound to fixture-pc-region-eu (chain depth 3).
//!
//! Stages:
//!   in-eu (input port)
//!   UpdateAttribute-tag-region (compliance = #{compliance_tag},
//!                               region = #{region_filter})
//!   RemoteProcessGroup → remote-targets/incoming-eu (TRANSMITTING)

use std::time::Duration;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_in_pg, create_input_port, create_processor,
    start_input_port, start_processor, wait_for_valid,
};
use crate::fixture::orders::shared::{
    connect_processor_to_remote_input_port, create_remote_process_group, set_rpg_transmitting,
    wait_for_remote_input_ports,
};
use crate::fixture::parameter_contexts::{self, OrdersContextIds};

pub struct SinkEuIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    client: &DynamicClient,
    orders_pg_id: &str,
    contexts: &OrdersContextIds,
    _incoming_eu_port_id: &str,
) -> Result<SinkEuIds> {
    tracing::info!("seeding orders-pipeline/sink-eu");

    let pg_id = create_child_pg(client, orders_pg_id, "sink-eu").await?;
    parameter_contexts::bind(client, &pg_id, &contexts.region_eu_id).await?;

    let in_port_id = create_input_port(client, &pg_id, "in-eu").await?;

    let tag_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-tag-region",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("compliance", "#{compliance_tag}"),
                ("region", "#{region_filter}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-tag-region",
    )
    .await?;

    let rpg_id = create_remote_process_group(client, &pg_id, "rpg-eu").await?;

    // Wait for NiFi to discover the input ports via S2S handshake.
    wait_for_remote_input_ports(client, &rpg_id, Duration::from_secs(60)).await?;

    // Wire input port → tag → RPG (incoming-eu remote port).
    create_connection_in_pg(
        client,
        &pg_id,
        &in_port_id,
        "INPUT_PORT",
        &tag_id,
        "PROCESSOR",
        vec![],
    )
    .await?;
    connect_processor_to_remote_input_port(client, &pg_id, &tag_id, &rpg_id, "incoming-eu").await?;

    // Set TRANSMITTING (after the connection is in place, so the RPG can
    // immediately accept flowfiles).
    set_rpg_transmitting(client, &rpg_id).await?;

    wait_for_valid(client, &tag_id, "UpdateAttribute-tag-region").await?;
    start_processor(client, &tag_id).await?;
    start_input_port(client, &in_port_id).await?;

    tracing::info!(%pg_id, %rpg_id, "sink-eu seeded (RPG TRANSMITTING)");
    Ok(SinkEuIds { pg_id, in_port_id })
}
