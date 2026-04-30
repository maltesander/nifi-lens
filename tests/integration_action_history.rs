//! Integration test for the action-history paginator against the live
//! `nifilens-fixture-v8` cluster. Verifies that the seeder's initial
//! setup creates auditable actions on the orders-pipeline ConvertRecord-csv2json
//! processor and that `flow_actions_paginator` returns them filtered by
//! `sourceId`.
//!
//! Gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_action_history -- --ignored` after
//! bringing up the Docker fixture.

use nifi_lens::client::NifiClient;
use nifi_lens::client::NodeKind;
use nifi_lens::client::history::flow_actions_paginator;
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

/// Find a processor by name from the recursive root status snapshot.
async fn find_processor_id_by_name(client: &NifiClient, proc_name: &str) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Processor) && n.name == proc_name)
        .map(|n| n.id.clone())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_action_history_paginator_returns_actions_filtered_by_source_id() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_action_history_paginator_returns_actions_filtered_by_source_id \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        // The seeder creates exactly one ConvertRecord-csv2json under
        // orders-pipeline/transform. Configuring it during seeding produces
        // at least one Configure action attributable to the processor.
        let proc_id = find_processor_id_by_name(&client, "ConvertRecord-csv2json")
            .await
            .unwrap_or_else(|| {
                panic!("fixture ConvertRecord-csv2json processor not found on {version}")
            });

        let mut p = flow_actions_paginator(&client, &proc_id, 100);
        let page = p
            .next_page()
            .await
            .unwrap_or_else(|e| panic!("flow_actions_paginator on {version} failed: {e:?}"))
            .unwrap_or_else(|| {
                panic!("expected at least one action page for ConvertRecord-csv2json on {version}")
            });

        assert!(
            !page.is_empty(),
            "expected at least one action recorded against ConvertRecord-csv2json on {version}"
        );

        // Every returned action must reference the requested processor.
        for action in &page {
            assert_eq!(
                action.source_id.as_deref(),
                Some(proc_id.as_str()),
                "paginator returned action for the wrong source on {version}: {:?}",
                action.source_id
            );
        }
    }
}
