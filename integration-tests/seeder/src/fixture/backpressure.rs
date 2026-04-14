//! Backpressure-pipeline fixture: saturated queue for the unhealthy
//! leaderboard.
//!
//! ```text
//! backpressure-pipeline/
//! ├── GenerateFlowFile (100 ms)
//! ├── connection with obj=10, size=1 KB thresholds
//! └── ControlRate (1 flowfile / 1 min, auto-terminate success+failure)
//! ```
//!
//! ControlRate starts FIRST so the tiny queue doesn't flood before the
//! throttle is up.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_connection, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::healthy::{create_child_pg, create_processor, start_processor, wait_for_valid};

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding backpressure-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "backpressure-pipeline").await?;

    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[]),
            Some("100 ms"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let ctrl_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ControlRate",
            "org.apache.nifi.processors.standard.ControlRate",
            props(&[
                ("Rate Control Criteria", "flowfile count"),
                ("Maximum Rate", "1"),
                ("Time Duration", "1 min"),
            ]),
            None,
            vec!["success", "failure"],
        ),
        "ControlRate",
    )
    .await?;

    // Connection with low backpressure thresholds so the queue saturates
    // rapidly once GenerateFlowFile starts producing.
    let mut conn_body = make_connection(
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &pg_id,
        &ctrl_id,
        "PROCESSOR",
        vec!["success"],
    );
    if let Some(component) = conn_body.component.as_mut() {
        component.back_pressure_object_threshold = Some(10);
        component.back_pressure_data_size_threshold = Some("1 KB".to_string());
    }

    client
        .processgroups()
        .create_connection(&pg_id, &conn_body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create backpressure-pipeline connection".into(),
            source: Box::new(e),
        })?;

    // Start ControlRate BEFORE GenerateFlowFile.
    wait_for_valid(client, &ctrl_id, "ControlRate").await?;
    start_processor(client, &ctrl_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("backpressure-pipeline seeded and running");
    Ok(())
}
