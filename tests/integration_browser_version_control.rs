//! Integration tests for the version-control drift feature. Runs
//! against the live fixture (`./integration-tests/run.sh`) on both
//! NiFi 2.6.0 (floor) and 2.9.0 (ceiling).
//!
//! These tests exercise the data path — `version_information_optional`,
//! `local_modifications` — not the UI surface (which is covered by
//! snapshot tests at the unit level). The fixture must contain two
//! PGs created by the seeder's `fixture::versioned` module:
//!   - `versioned-clean` → `UP_TO_DATE`.
//!   - `versioned-modified` → `LOCALLY_MODIFIED`.
//!
//! The marker PG (`nifilens-fixture-v6`) itself is unversioned.

use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

fn it_context(version: &str) -> ResolvedContext {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");
    ResolvedContext {
        name: context_for(version),
        url: format!("https://localhost:{}", port_for(version)),
        auth: ResolvedAuth::Password { username, password },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: false,
        ca_cert_path: Some(ca_path.into()),
        proxied_entities_chain: None,
        proxy_url: None,
        http_proxy_url: None,
        https_proxy_url: None,
    }
}

/// Resolve a PG by name from the recursive root status snapshot.
async fn find_pg_id_by_name(client: &NifiClient, pg_name: &str) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.name == pg_name)
        .map(|n| n.id.clone())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn versioned_clean_pg_reports_up_to_date() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- versioned-clean on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();

        let pg_id = find_pg_id_by_name(&client, "versioned-clean")
            .await
            .unwrap_or_else(|| panic!("versioned-clean PG not found on {version}"));

        let summary = client
            .version_information_optional(&pg_id)
            .await
            .unwrap()
            .unwrap_or_else(|| {
                panic!("versioned-clean has no version_control_information on {version}")
            });

        assert_eq!(
            summary.state,
            VersionControlInformationDtoState::UpToDate,
            "versioned-clean must be UP_TO_DATE on {version}, got {:?}",
            summary.state
        );
        assert_eq!(summary.flow_name.as_deref(), Some("versioned-clean"));
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn versioned_modified_pg_reports_locally_modified() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- versioned-modified on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();

        let pg_id = find_pg_id_by_name(&client, "versioned-modified")
            .await
            .unwrap_or_else(|| panic!("versioned-modified PG not found on {version}"));

        let summary = client
            .version_information_optional(&pg_id)
            .await
            .unwrap()
            .unwrap_or_else(|| {
                panic!("versioned-modified has no version_control_information on {version}")
            });

        assert_eq!(
            summary.state,
            VersionControlInformationDtoState::LocallyModified,
            "versioned-modified must be LOCALLY_MODIFIED on {version}, got {:?}",
            summary.state
        );
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn versioned_modified_local_modifications_lists_property_change() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- local-modifications versioned-modified on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();

        let pg_id = find_pg_id_by_name(&client, "versioned-modified")
            .await
            .unwrap_or_else(|| panic!("versioned-modified PG not found on {version}"));

        let grouped = client.local_modifications(&pg_id).await.unwrap();
        assert!(
            !grouped.sections.is_empty(),
            "versioned-modified on {version} must surface at least one component diff section"
        );

        // Find the LogAttribute processor's diff section. Seeder names
        // it `vc-mod-LogAttribute`.
        let log_section = grouped
            .sections
            .iter()
            .find(|s| s.component_name == "vc-mod-LogAttribute");
        assert!(
            log_section.is_some(),
            "expected a section for vc-mod-LogAttribute on {version}, sections were: {:?}",
            grouped
                .sections
                .iter()
                .map(|s| &s.component_name)
                .collect::<Vec<_>>()
        );
        let log_section = log_section.unwrap();

        // At least one diff should be a PROPERTY_CHANGED on Log Level.
        // NiFi's wire format includes the property key in the description.
        let has_log_level_change = log_section.differences.iter().any(|d| {
            d.kind == "PROPERTY_CHANGED" && d.description.to_lowercase().contains("log level")
        });
        assert!(
            has_log_level_change,
            "expected PROPERTY_CHANGED 'Log Level' diff on {version}; differences were: {:?}",
            log_section.differences
        );
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn unversioned_pg_returns_none_from_version_information_optional() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- unversioned marker PG on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();

        // Marker PG is unversioned.
        let pg_id = find_pg_id_by_name(&client, "nifilens-fixture-v6")
            .await
            .unwrap_or_else(|| panic!("marker PG not found on {version}"));

        let res = client.version_information_optional(&pg_id).await.unwrap();
        assert!(
            res.is_none(),
            "unversioned marker PG must return Ok(None) on {version}, got {:?}",
            res
        );
    }
}
