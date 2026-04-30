//! Fixture topology definitions.

pub mod backpressure;
pub mod common;
pub mod invalid;
pub mod orders;
pub mod parameter_contexts;
pub mod registry;
pub mod services;
pub mod versioned;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::make_pg;
use crate::error::{Result, SeederError};
use crate::marker::FIXTURE_MARKER_NAME;

/// Top-level seed entry point. Creates the marker PG and populates it
/// with the full fixture topology. Assumes the cluster has already been
/// nuke-and-repaved (or is fresh).
pub async fn seed(
    client: &DynamicClient,
    detected_version: &semver::Version,
    break_after: std::time::Duration,
) -> Result<()> {
    tracing::info!("ensuring registry-client and fixture bucket");
    let registry_ids = registry::seed(client).await?;

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

    tracing::info!("seeding orders parameter contexts");
    let orders_ctx = parameter_contexts::seed(client).await?;

    // OLD fixtures (still active so unmigrated tests keep working).
    backpressure::seed(client, &marker_pg_id).await?;
    invalid::seed(client, &marker_pg_id).await?;
    versioned::seed(client, &marker_pg_id, &registry_ids, detected_version).await?;

    // NEW orders-pipeline.
    orders::seed(
        client,
        &marker_pg_id,
        &orders_ctx,
        &service_ids,
        detected_version,
    )
    .await?;

    // --break-after sleep + three-step parameter mutation.
    orders::break_::apply_break(client, &orders_ctx.orders_id, break_after).await?;

    tracing::info!("fixture seed complete");
    Ok(())
}

/// Internal property key for GenerateFlowFile's "Custom Text" field.
///
/// NiFi 2.6.0's GenerateFlowFile uses `generate-ff-custom-text` as the
/// descriptor key (display name "Custom Text"). From 2.9.0 the key was
/// renamed to match the display name. Setting the wrong key silently
/// turns the value into a user-defined dynamic flowfile attribute —
/// GenerateFlowFile then emits 0-byte flowfiles with the full payload
/// as an attribute. The 2.8 cutoff is a conservative guess between our
/// floor (2.6.0) and ceiling (2.9.0); revisit if we add a fixture
/// version in between.
pub fn custom_text_property_key(version: &semver::Version) -> &'static str {
    if version.major < 2 || (version.major == 2 && version.minor < 9) {
        "generate-ff-custom-text"
    } else {
        "Custom Text"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_text_key_uses_legacy_on_floor() {
        let v = semver::Version::parse("2.6.0").unwrap();
        assert_eq!(custom_text_property_key(&v), "generate-ff-custom-text");
    }

    #[test]
    fn custom_text_key_uses_modern_on_ceiling() {
        let v = semver::Version::parse("2.9.0").unwrap();
        assert_eq!(custom_text_property_key(&v), "Custom Text");
    }

    #[test]
    fn custom_text_key_switches_at_2_9() {
        assert_eq!(
            custom_text_property_key(&semver::Version::parse("2.8.99").unwrap()),
            "generate-ff-custom-text"
        );
        assert_eq!(
            custom_text_property_key(&semver::Version::parse("2.9.0").unwrap()),
            "Custom Text"
        );
    }
}
