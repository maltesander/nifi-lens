//! Noisy-pipeline fixture: constant WARN + ERROR bulletin source for the
//! Bulletins tab and the Overview noisy leaderboard.
//!
//! ```text
//! noisy-pipeline/
//! ├── GenerateFlowFile (2 sec, Batch Size 3)
//! ├── LogMessage-WARN (Log Level=warn, Log Message=fixture-warn)
//! └── LogMessage-ERROR (Log Level=error, Log Message=fixture-error,
//!                       auto-terminate "success")
//! ```

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::healthy::{
    create_child_pg, create_connection_in_pg, create_processor, start_processor, wait_for_valid,
};

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding noisy-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "noisy-pipeline").await?;

    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[("Batch Size", "3")]),
            Some("2 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let warn_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogMessage-WARN",
            "org.apache.nifi.processors.standard.LogMessage",
            props(&[("Log Level", "warn"), ("Log Message", "fixture-warn")]),
            None,
            vec![],
        ),
        "LogMessage-WARN",
    )
    .await?;

    let error_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogMessage-ERROR",
            "org.apache.nifi.processors.standard.LogMessage",
            props(&[("Log Level", "error"), ("Log Message", "fixture-error")]),
            None,
            vec!["success"],
        ),
        "LogMessage-ERROR",
    )
    .await?;

    create_connection_in_pg(
        client,
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &warn_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &warn_id,
        "PROCESSOR",
        &error_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // Start downstream first.
    wait_for_valid(client, &error_id, "LogMessage-ERROR").await?;
    start_processor(client, &error_id).await?;
    wait_for_valid(client, &warn_id, "LogMessage-WARN").await?;
    start_processor(client, &warn_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("noisy-pipeline seeded and running");
    Ok(())
}
