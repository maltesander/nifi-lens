//! Integration test: verify connection endpoint IDs are populated on
//! `NodeStatusSummary::Connection` via the per-PG connections fetch in
//! `browser_tree`.

use nifi_lens::client::{NifiClient, NodeKind, NodeStatusSummary};
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

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_browser_tree_populates_connection_endpoint_ids() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- connection endpoints on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();
        let snap = client.browser_tree().await.unwrap();

        let mut total = 0usize;
        let mut with_both_ids = 0usize;
        for n in snap.nodes.iter() {
            if !matches!(n.kind, NodeKind::Connection) {
                continue;
            }
            total += 1;
            if let NodeStatusSummary::Connection {
                source_id,
                destination_id,
                ..
            } = &n.status_summary
                && !source_id.is_empty()
                && !destination_id.is_empty()
            {
                with_both_ids += 1;
            }
        }
        eprintln!("  {total} connections, {with_both_ids} with both endpoint ids populated");
        assert!(
            total > 0,
            "fixture on {version} has at least one connection"
        );
        assert_eq!(
            with_both_ids, total,
            "all connections on {version} must have source_id and destination_id populated"
        );
    }
}
