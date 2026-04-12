//! Fixture topology definitions.

pub mod backpressure;
pub mod healthy;
pub mod invalid;
pub mod noisy;
pub mod payload;
pub mod services;
pub mod stress;
pub mod stress_payload;

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
///
/// `detected_version` is the NiFi semver version. The stress pipeline
/// is only seeded when the version is >= 2.9.0.
pub async fn seed(client: &DynamicClient, detected_version: &semver::Version) -> Result<()> {
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

    let stress_min = semver::Version::new(2, 9, 0);
    if *detected_version >= stress_min {
        tracing::info!("NiFi >= 2.9.0 detected; seeding stress-pipeline");
        stress::seed(client, &marker_pg_id).await?;
    } else {
        tracing::info!(
            %detected_version,
            "NiFi < 2.9.0; skipping stress-pipeline"
        );
    }

    tracing::info!("fixture seed complete");
    Ok(())
}
