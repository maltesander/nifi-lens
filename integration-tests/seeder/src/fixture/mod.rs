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
pub async fn seed(client: &DynamicClient, detected_version: &semver::Version) -> Result<()> {
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

    healthy::seed(client, &marker_pg_id, &service_ids, detected_version).await?;
    noisy::seed(client, &marker_pg_id).await?;
    backpressure::seed(client, &marker_pg_id).await?;
    invalid::seed(client, &marker_pg_id).await?;
    bulky::seed(client, &marker_pg_id).await?;
    diff::seed(client, &marker_pg_id, detected_version).await?;

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

/// Property keys for QueryRecord's Record Reader / Record Writer fields.
///
/// Returns `(reader_key, writer_key)`. Drift case symmetric with
/// [`custom_text_property_key`]: 2.6.0 uses kebab-case descriptor
/// keys (`record-reader`, `record-writer`) while 2.9.0 reverted them
/// to the display-name form (`Record Reader`, `Record Writer`).
/// Setting the wrong key creates dynamic properties — and dynamic
/// properties on QueryRecord are interpreted as SQL queries, which
/// validation rejects with "Non-query expression encountered in
/// illegal context". UpdateRecord doesn't have this drift.
pub fn query_record_io_property_keys(version: &semver::Version) -> (&'static str, &'static str) {
    if version.major < 2 || (version.major == 2 && version.minor < 9) {
        ("record-reader", "record-writer")
    } else {
        ("Record Reader", "Record Writer")
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
