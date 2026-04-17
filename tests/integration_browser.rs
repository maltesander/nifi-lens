//! Integration tests for the Browser tab: CS tree membership, CS
//! referencing-components, port detail. All gated on `#[ignore]` — run
//! via `./integration-tests/run.sh` or `cargo test --test
//! integration_browser -- --ignored` after bringing up the Docker
//! fixture. Loops over every version in `FIXTURE_VERSIONS`.

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
async fn integration_browser_tree_contains_controller_services_under_owning_pgs() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_browser_tree_contains_controller_services_under_owning_pgs \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .browser_tree()
            .await
            .unwrap_or_else(|e| panic!("browser_tree on {version} failed: {e:?}"));

        let cs: Vec<_> = snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::ControllerService))
            .collect();
        assert!(
            cs.len() >= 3,
            "fixture seeds at least 3 CS on {version}, got {}",
            cs.len()
        );
        for n in cs {
            let parent = n
                .parent_idx
                .unwrap_or_else(|| panic!("CS {} on {version} must be parented to a PG", n.id));
            assert!(
                matches!(snap.nodes[parent].kind, NodeKind::ProcessGroup),
                "CS parent must be a PG in the arena on {version}"
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

        let snap = client
            .browser_tree()
            .await
            .unwrap_or_else(|e| panic!("browser_tree on {version} failed: {e:?}"));

        // At least one fixture CS must be referenced by the pipelines.
        let mut found = false;
        for n in snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::ControllerService))
        {
            let d = client
                .browser_cs_detail(&n.id)
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

        let snap = client
            .browser_tree()
            .await
            .unwrap_or_else(|e| panic!("browser_tree on {version} failed: {e:?}"));

        let port = snap
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
