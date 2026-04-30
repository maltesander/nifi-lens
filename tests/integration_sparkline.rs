//! Integration test for the status-history client helper against the
//! live `nifilens-fixture-v8` cluster. Verifies that the seeder's
//! orders-pipeline/transform ConvertRecord-csv2json processor returns at least one bucket
//! after a brief warm-up — exercising the `/flow/{type}/{id}/status/history`
//! dispatcher path on both fixture NiFi versions.
//!
//! Gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_sparkline -- --ignored` after
//! bringing up the Docker fixture.

use nifi_lens::client::NifiClient;
use nifi_lens::client::NodeKind;
use nifi_lens::client::history::{ComponentKind, status_history};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

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

fn find_processor_id_by_name(
    nodes: &[nifi_lens::client::RawNode],
    proc_name: &str,
    target_pg_name: &str,
) -> Option<String> {
    // Split the target_pg by "/" to handle nested paths like "orders-pipeline/transform"
    let pg_parts: Vec<&str> = target_pg_name.split('/').collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Processor || node.name != proc_name {
            continue;
        }

        // Walk up the parent chain and collect all parent names
        let mut parent_names = Vec::new();
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            cursor = p;
            if matches!(nodes[cursor].kind, NodeKind::ProcessGroup) {
                parent_names.push(nodes[cursor].name.as_str());
            }
        }

        // Reverse to get the path from root to parent
        parent_names.reverse();

        // Check if the parent path ends with our target pg path
        if parent_names.len() >= pg_parts.len() {
            let start = parent_names.len() - pg_parts.len();
            if parent_names[start..] == *pg_parts {
                return Some(node.id.clone());
            }
        }
    }
    None
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_sparkline_status_history_returns_buckets() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_sparkline_status_history_returns_buckets \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let proc_id = find_processor_id_by_name(
            &snapshot.nodes,
            "ConvertRecord-csv2json",
            "orders-pipeline/transform",
        )
        .unwrap_or_else(|| {
            panic!(
                "fixture ConvertRecord-csv2json in orders-pipeline/transform not found on {version}"
            )
        });

        let series = status_history(&client, ComponentKind::Processor, &proc_id)
            .await
            .unwrap_or_else(|e| panic!("status_history on {version} failed: {e:?}"));

        // NiFi reports at least one bucket immediately on a freshly-seeded
        // processor; we don't assert specific counts (depends on traffic).
        assert!(
            !series.buckets.is_empty(),
            "expected at least one status_history bucket for ConvertRecord-csv2json on {version}"
        );
    }
}
