//! Fixture topology definitions.

pub mod backpressure;
pub mod healthy;
pub mod invalid;
pub mod noisy;
pub mod payload;
pub mod services;

use nifi_rust_client::dynamic::{
    DynamicClient,
    traits::{ProcessGroupsApi as _, ProcessGroupsProcessGroupsApi as _},
};

use crate::entities::make_pg;
use crate::error::{Result, SeederError};
use crate::marker::FIXTURE_MARKER_NAME;

/// Top-level seed entry point. Creates the marker PG and populates it
/// with the full fixture topology. Assumes the cluster has already been
/// nuke-and-repaved (or is fresh).
pub async fn seed(client: &DynamicClient) -> Result<()> {
    tracing::info!("seeding controller services at root");
    services::seed(client, "root").await?;

    tracing::info!(marker = FIXTURE_MARKER_NAME, "creating fixture marker PG");
    let body = make_pg(FIXTURE_MARKER_NAME);
    let created = client
        .processgroups_api()
        .process_groups("root")
        .create_process_group(None, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create fixture marker PG".into(),
            source: Box::new(e),
        })?;
    let marker_pg_id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: "fixture marker PG has no id".into(),
        })?;

    healthy::seed(client, &marker_pg_id).await?;
    noisy::seed(client, &marker_pg_id).await?;
    backpressure::seed(client, &marker_pg_id).await?;
    invalid::seed(client, &marker_pg_id).await?;

    tracing::info!("fixture seed complete");
    Ok(())
}
