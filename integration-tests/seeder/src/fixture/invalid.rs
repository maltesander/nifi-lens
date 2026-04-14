//! Invalid-pipeline fixture: a processor that is intentionally INVALID
//! and never started, for the Overview "invalid" count and the Browser
//! tab's validation-error surface.
//!
//! ```text
//! invalid-pipeline/
//! └── GenerateFlowFile (File Size = "", Data Format = "Text") — INVALID
//! ```

use std::time::Duration;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::healthy::{create_child_pg, create_processor};
use crate::state::poll_until;

pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding invalid-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "invalid-pipeline").await?;

    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[("File Size", ""), ("Data Format", "Text")]),
            None,
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    // Wait for validation_errors to populate so we know the processor
    // reached a terminal INVALID state.
    let id_poll = gen_id.clone();
    poll_until(
        "invalid-pipeline GenerateFlowFile",
        "INVALID",
        Duration::from_secs(15),
        || {
            let id = id_poll.clone();
            async move {
                let got =
                    client
                        .processors()
                        .get_processor(&id)
                        .await
                        .map_err(|e| SeederError::Api {
                            message: format!("poll invalid processor {id}"),
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

    tracing::info!(%gen_id, "invalid-pipeline GenerateFlowFile INVALID as expected");
    Ok(())
}
