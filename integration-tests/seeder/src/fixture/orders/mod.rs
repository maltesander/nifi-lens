//! orders-pipeline: production-shaped centerpiece pipeline.
//!
//! Topology (top-level under marker PG):
//!
//! ```text
//! orders-pipeline/
//! ├── ingest/                  child PG, bound to fixture-pc-platform
//! ├── transform/               child PG, bound to fixture-pc-orders
//! ├── sink-eu/                 child PG, bound to fixture-pc-region-eu
//! ├── sink-us/                 child PG, bound to fixture-pc-region-us
//! ├── sink-apac/               child PG, bound to fixture-pc-region-apac
//! └── deadletter/              child PG, no parameter binding
//! remote-targets/               sibling subtree, RPG send targets
//! ```
//!
//! Connections:
//!   ingest/raw-orders         -> transform/incoming-orders   (parent)
//!   transform/out-eu          -> sink-eu/in-eu               (parent)
//!   transform/out-us          -> sink-us/in-us               (parent)
//!   transform/out-apac        -> sink-apac/in-apac           (parent)
//!   transform/out-failed      -> deadletter/in-failed        (parent)
//!
//! Mutation of the orders parameter context for `--break-after` is
//! wired by `fixture::seed` (Task 13); see `orders::break_::apply_break`.
//!
//! Module-level allow(dead_code) covers the multi-task build-up: the
//! submodule stubs return SeederError::Invariant until tasks 6-12 fill
//! them in. Remove this attribute in Task 13 once `fixture::seed` calls
//! `orders::seed` and every field/function becomes reachable.
#![allow(dead_code)]

pub mod break_; // `break` is a Rust keyword; use `break_`
pub mod deadletter;
pub mod remote_targets;
pub mod sink_apac;
pub mod sink_eu;
pub mod sink_us;
pub mod transform;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_between, create_connection_in_pg, create_output_port,
    create_processor, start_output_port, start_processor, wait_for_valid,
};
use crate::fixture::custom_text_property_key;
use crate::fixture::parameter_contexts::{self, OrdersContextIds};
use crate::fixture::services::ServiceIds;

/// Embedded order payload used as GenerateFlowFile's Custom Text.
/// ~10000 records of synthetic order data; see seeder/tools/gen-orders-payload.
const ORDERS_PAYLOAD: &str = include_str!("../../../assets/orders_payload.csv");

pub async fn seed(
    client: &DynamicClient,
    parent_pg_id: &str,
    contexts: &OrdersContextIds,
    service_ids: &ServiceIds,
    version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding orders-pipeline");

    let orders_pg_id = create_child_pg(client, parent_pg_id, "orders-pipeline").await?;

    // remote-targets is a sibling subtree of orders-pipeline (under marker PG).
    let remote_targets_pg_id = create_child_pg(client, parent_pg_id, "remote-targets").await?;
    let (incoming_eu_port_id, incoming_apac_port_id) =
        remote_targets::seed(client, &remote_targets_pg_id).await?;

    // Build each child PG. Each `*::seed` returns the IDs needed for
    // cross-PG wiring (input/output ports primarily).
    let ingest = build_ingest(client, &orders_pg_id, contexts, version).await?;
    let transform = transform::seed(client, &orders_pg_id, contexts, service_ids, version).await?;
    let sink_eu = sink_eu::seed(client, &orders_pg_id, contexts, &incoming_eu_port_id).await?;
    let sink_us = sink_us::seed(client, &orders_pg_id, contexts).await?;
    let sink_apac =
        sink_apac::seed(client, &orders_pg_id, contexts, &incoming_apac_port_id).await?;
    let deadletter = deadletter::seed(client, &orders_pg_id).await?;

    // Cross-PG wiring at the orders-pipeline level.
    create_connection_between(
        client,
        &orders_pg_id,
        &ingest.pg_id,
        &ingest.raw_orders_port_id,
        "OUTPUT_PORT",
        &transform.pg_id,
        &transform.incoming_port_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;
    create_connection_between(
        client,
        &orders_pg_id,
        &transform.pg_id,
        &transform.out_eu_port_id,
        "OUTPUT_PORT",
        &sink_eu.pg_id,
        &sink_eu.in_port_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;
    create_connection_between(
        client,
        &orders_pg_id,
        &transform.pg_id,
        &transform.out_us_port_id,
        "OUTPUT_PORT",
        &sink_us.pg_id,
        &sink_us.in_port_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;
    create_connection_between(
        client,
        &orders_pg_id,
        &transform.pg_id,
        &transform.out_apac_port_id,
        "OUTPUT_PORT",
        &sink_apac.pg_id,
        &sink_apac.in_port_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;
    create_connection_between(
        client,
        &orders_pg_id,
        &transform.pg_id,
        &transform.out_failed_port_id,
        "OUTPUT_PORT",
        &deadletter.pg_id,
        &deadletter.in_port_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;

    tracing::info!("orders-pipeline topology seeded; ports & processors started by sub-modules");
    Ok(())
}

/// IDs needed by orders/mod.rs to wire ingest's output port at the parent level.
pub struct IngestIds {
    pub pg_id: String,
    pub raw_orders_port_id: String,
}

async fn build_ingest(
    client: &DynamicClient,
    orders_pg_id: &str,
    contexts: &OrdersContextIds,
    version: &semver::Version,
) -> Result<IngestIds> {
    tracing::info!("seeding orders-pipeline/ingest");
    let pg_id = create_child_pg(client, orders_pg_id, "ingest").await?;
    parameter_contexts::bind(client, &pg_id, &contexts.platform_id).await?;

    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[
                (custom_text_property_key(version), ORDERS_PAYLOAD),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("10 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let raw_orders_port_id = create_output_port(client, &pg_id, "raw-orders").await?;

    create_connection_in_pg(
        client,
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &raw_orders_port_id,
        "OUTPUT_PORT",
        vec!["success"],
    )
    .await?;

    // Start downstream first.
    start_output_port(client, &raw_orders_port_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    Ok(IngestIds {
        pg_id,
        raw_orders_port_id,
    })
}
