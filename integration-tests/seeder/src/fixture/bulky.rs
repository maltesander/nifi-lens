//! bulky-pipeline: large-payload fixture for Tracer content truncation.
//!
//! ```text
//! bulky-pipeline/
//! ├── GenerateFlowFile (Data Format = "Text", File Size = "1536 KB",
//! │       Schedule = "30 sec")
//! └── LogAttribute (Log Level = "info", Log Payload = "false")
//! ```
//!
//! Produces ~1.5 MiB flowfiles at a low rate. Provenance events in
//! this PG exceed the Tracer's 1 MiB preview cap, exercising the
//! Range-header truncation path end-to-end.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::healthy::{
    create_child_pg, create_connection_in_pg, create_processor, start_processor, wait_for_valid,
};

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding bulky-pipeline");

    let bulky_pg_id = create_child_pg(client, parent_pg_id, "bulky-pipeline").await?;

    let gen_id = create_processor(
        client,
        &bulky_pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[("Data Format", "Text"), ("File Size", "1536 KB")]),
            Some("30 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let log_id = create_processor(
        client,
        &bulky_pg_id,
        make_processor(
            "LogAttribute",
            "org.apache.nifi.processors.standard.LogAttribute",
            // Log Payload=false keeps the 1.5 MiB body out of the
            // NiFi log. We only care about provenance events here.
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        "LogAttribute",
    )
    .await?;

    create_connection_in_pg(
        client,
        &bulky_pg_id,
        &gen_id,
        "PROCESSOR",
        &log_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // Start downstream first so nothing backs up on startup.
    wait_for_valid(client, &log_id, "LogAttribute").await?;
    start_processor(client, &log_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("bulky-pipeline seeded and running");
    Ok(())
}
