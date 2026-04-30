//! Integration tests for the Browser tab: CS tree membership, CS
//! referencing-components, port detail, top-level fixture roster. All
//! gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_browser -- --ignored` after bringing
//! up the Docker fixture. Loops over every version in
//! `FIXTURE_VERSIONS`.
//!
//! Task 6 of the central-cluster-store refactor retired
//! `NifiClient::browser_tree`; these tests now exercise the pair of
//! endpoints (`root_pg_status` for the arena skeleton +
//! `controller_services_snapshot` for CS identity) that the reducer
//! consumes at runtime.

use nifi_lens::client::{NifiClient, NodeKind, PortKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Build a `ResolvedContext` for `version` from the standard integration
/// env vars. Mirrors the inline context construction used by the other
/// `integration_*` tests in this crate.
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

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_controller_services_reports_members_under_owning_pgs() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_controller_services_reports_members_under_owning_pgs \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let cs_snap = client
            .controller_services_snapshot()
            .await
            .unwrap_or_else(|e| panic!("controller_services on {version} failed: {e:?}"));

        assert!(
            cs_snap.members.len() >= 3,
            "fixture seeds at least 3 CS on {version}, got {}",
            cs_snap.members.len()
        );
        for m in &cs_snap.members {
            assert!(
                !m.parent_group_id.is_empty(),
                "CS {} on {version} must report a parent group id",
                m.id
            );
        }
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_browser_cs_detail_reports_referencing_components() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_browser_cs_detail_reports_referencing_components \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let cs_snap = client
            .controller_services_snapshot()
            .await
            .unwrap_or_else(|e| panic!("controller_services on {version} failed: {e:?}"));

        // At least one fixture CS must be referenced by the pipelines.
        let mut found = false;
        for m in &cs_snap.members {
            let d = client
                .browser_cs_detail(&m.id)
                .await
                .unwrap_or_else(|e| panic!("browser_cs_detail on {version} failed: {e:?}"));
            if !d.referencing_components.is_empty() {
                found = true;
                break;
            }
        }
        assert!(
            found,
            "at least one fixture CS on {version} must have referencing components"
        );
    }
}

/// Pins the current fixture roster: 6 top-level PGs under the marker.
/// The `orders-pipeline` centerpiece, the `remote-targets` RPG-receive
/// subtree, and four standalone fixtures retained for state-encoding
/// (`invalid`, `backpressure`, `versioned-clean`, `versioned-modified`).
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_browser_lists_expected_top_level_fixture_pgs() {
    const FIXTURE_MARKER: &str = "nifilens-fixture-v8";
    const REQUIRED_TOP_LEVEL_PGS: &[&str] = &[
        // Centerpiece + RPG-receive subtree from the orders rework.
        "orders-pipeline",
        "remote-targets",
        // Retained standalone fixtures (each encodes a state hard to
        // reach mid-narrative, so they stay separate from orders).
        "invalid-pipeline",
        "backpressure-pipeline",
        "versioned-clean",
        "versioned-modified",
    ];

    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_browser_lists_expected_top_level_fixture_pgs \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let marker_idx = snap
            .nodes
            .iter()
            .position(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.name == FIXTURE_MARKER)
            .unwrap_or_else(|| {
                panic!("fixture marker PG `{FIXTURE_MARKER}` not present on {version}")
            });

        let top_level: Vec<&str> = snap
            .nodes
            .iter()
            .filter(|n| {
                matches!(n.kind, NodeKind::ProcessGroup) && n.parent_idx == Some(marker_idx)
            })
            .map(|n| n.name.as_str())
            .collect();

        eprintln!(
            "  marker `{FIXTURE_MARKER}` on {version} has {} top-level PG(s): {:?}",
            top_level.len(),
            top_level
        );

        for required in REQUIRED_TOP_LEVEL_PGS {
            assert!(
                top_level.contains(required),
                "expected top-level PG `{required}` under `{FIXTURE_MARKER}` on {version}; \
                 found {top_level:?}"
            );
        }

        // Loose `>= REQUIRED.len()` bound tolerates incidental extras
        // (e.g. user-added PGs against a live cluster) without
        // weakening the required-set assertion above.
        assert!(
            top_level.len() >= REQUIRED_TOP_LEVEL_PGS.len(),
            "expected at least {} top-level PGs under `{FIXTURE_MARKER}` on {version}, \
             got {} ({top_level:?})",
            REQUIRED_TOP_LEVEL_PGS.len(),
            top_level.len()
        );
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_browser_port_detail_resolves() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_browser_port_detail_resolves running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let pg_snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let port = pg_snap
            .nodes
            .iter()
            .find(|n| matches!(n.kind, NodeKind::InputPort | NodeKind::OutputPort));
        let Some(port) = port else {
            eprintln!("  fixture on {version} has no ports; skipping");
            continue;
        };
        let kind = match port.kind {
            NodeKind::InputPort => PortKind::Input,
            NodeKind::OutputPort => PortKind::Output,
            _ => unreachable!(),
        };
        let d = client
            .browser_port_detail(&port.id, kind)
            .await
            .unwrap_or_else(|e| panic!("browser_port_detail on {version} failed: {e:?}"));
        assert_eq!(d.id, port.id, "port id roundtrip on {version}");
        assert_eq!(d.kind, kind, "port kind roundtrip on {version}");
    }
}
