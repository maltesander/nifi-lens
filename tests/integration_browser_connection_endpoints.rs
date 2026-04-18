//! Integration test: verify connection endpoint IDs are populated for
//! every connection in the fixture. Task 6 of the central-cluster-store
//! refactor moved the per-PG `/connections` fan-out from `browser_tree`
//! into `ClusterStore::spawn_connections_by_pg`; this test exercises
//! the Browser reducer's equivalent by calling the same pair of
//! endpoints the store subscribes to (`root_pg_status` for the PG
//! skeleton + a per-PG `get_connections` fetch via the dynamic client)
//! and asserting the endpoint ids come back populated.

use nifi_lens::client::{NifiClient, NodeKind};
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
async fn integration_per_pg_connections_populate_endpoint_ids() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- per-PG connections on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx).await.unwrap();

        // Root PG status gives the arena skeleton — every connection row
        // (with blank source_id/destination_id, since NiFi's recursive
        // status endpoint leaves them null) plus the list of every PG
        // the store needs to fan the per-PG `/connections` call over.
        let pg_snap = client.root_pg_status().await.unwrap();
        let connection_count = pg_snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Connection))
            .count();
        assert!(
            connection_count > 0,
            "fixture on {version} has at least one connection"
        );

        // Fan out the per-PG connections fetch — this is what the
        // cluster-store fetcher does in production, issued inline here
        // so the test doesn't depend on the store machinery.
        let mut total = 0usize;
        let mut with_both_ids = 0usize;
        for pg_id in &pg_snap.process_group_ids {
            let conns = client.processgroups().get_connections(pg_id).await.unwrap();
            for c in conns.connections.unwrap_or_default() {
                total += 1;
                let has_src = c.source_id.as_deref().is_some_and(|s| !s.is_empty());
                let has_dst = c.destination_id.as_deref().is_some_and(|s| !s.is_empty());
                if has_src && has_dst {
                    with_both_ids += 1;
                }
            }
        }
        eprintln!("  {total} connections, {with_both_ids} with both endpoint ids populated");
        assert_eq!(
            with_both_ids, total,
            "all connections on {version} must have source_id and destination_id populated \
             via the per-PG /connections fetch"
        );
    }
}
