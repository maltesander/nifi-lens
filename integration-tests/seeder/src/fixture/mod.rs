//! Fixture topology definitions.

pub mod backpressure;
pub mod bulky;
pub mod diff;
pub mod healthy;
pub mod invalid;
pub mod noisy;
pub mod payload;
pub mod services;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::make_pg;
use crate::error::{Result, SeederError};
use crate::marker::FIXTURE_MARKER_NAME;

/// Top-level seed entry point. Creates the marker PG and populates it
/// with the full fixture topology. Assumes the cluster has already been
/// nuke-and-repaved (or is fresh).
///
/// `_detected_version` is the NiFi semver version. All current pipelines
/// work on the 2.6.0 floor; the parameter is kept on the signature for
/// future version-gating.
pub async fn seed(client: &DynamicClient, _detected_version: &semver::Version) -> Result<()> {
    tracing::info!("seeding controller services at root");
    let service_ids = services::seed(client, "root").await?;

    tracing::info!(marker = FIXTURE_MARKER_NAME, "creating fixture marker PG");
    let body = make_pg(FIXTURE_MARKER_NAME);
    let created = client
        .processgroups()
        .create_process_group("root", None, &body)
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

    healthy::seed(client, &marker_pg_id, &service_ids).await?;
    noisy::seed(client, &marker_pg_id).await?;
    backpressure::seed(client, &marker_pg_id).await?;
    invalid::seed(client, &marker_pg_id).await?;
    bulky::seed(client, &marker_pg_id).await?;
    diff::seed(client, &marker_pg_id).await?;

    tracing::info!("fixture seed complete");
    Ok(())
}
