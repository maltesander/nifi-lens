//! Integration test: verify connection endpoint IDs are populated for
//! every connection inside `orders-pipeline/transform`. The
//! central-cluster-store refactor moved the per-PG `/connections`
//! fan-out from `browser_tree` into
//! `ClusterStore::spawn_connections_by_pg`; this test exercises the
//! Browser reducer's equivalent by calling the same pair of endpoints
//! the store subscribes to (`root_pg_status` for the PG skeleton + a
//! per-PG `get_connections` fetch via the dynamic client) and asserts
//! the endpoint ids come back populated.
//!
//! Scope: the `orders-pipeline/transform` PG owns the densest set of
//! intra-PG connections in the fixture (split-path through
//! UpdateAttribute/RouteOnAttribute into the regional output ports),
//! plus four cross-PG connections from its output ports to each sink.
//! Looking up the PG by suffix-matched name path keeps the test stable
//! against fixture rearrangement and avoids mixing legacy fixture
//! noise into the assertion set.

use nifi_lens::client::{NifiClient, NodeKind, RawNode};
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

/// Resolve a PG id by walking the arena and matching the slash-separated
/// suffix of the PG's full path-from-root.
fn find_pg_id_by_path(nodes: &[RawNode], target_pg_path: &str) -> Option<String> {
    let parts: Vec<&str> = target_pg_path.split('/').collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::ProcessGroup {
            continue;
        }

        // Collect this PG's full path-from-root (innermost last).
        let mut chain = vec![node.name.as_str()];
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            cursor = p;
            if matches!(nodes[cursor].kind, NodeKind::ProcessGroup) {
                chain.push(nodes[cursor].name.as_str());
            }
        }
        chain.reverse();

        if chain.len() >= parts.len() {
            let start = chain.len() - parts.len();
            if chain[start..] == parts[..] {
                return Some(node.id.clone());
            }
        }
    }
    None
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

        let transform_pg_id = find_pg_id_by_path(&pg_snap.nodes, "orders-pipeline/transform")
            .unwrap_or_else(|| {
                panic!("fixture orders-pipeline/transform PG not found on {version}")
            });

        // Sanity: at least one Connection node lives inside the
        // transform PG.
        let connection_count = pg_snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::Connection) && n.group_id == transform_pg_id)
            .count();
        assert!(
            connection_count > 0,
            "orders-pipeline/transform on {version} must have at least one connection \
             (got {connection_count})"
        );

        // Issue the same per-PG fetch the cluster-store fetcher does in
        // production, but only for the transform PG. Every connection
        // returned must have both endpoint ids populated — that's the
        // contract the Browser reducer relies on to wire cross-links.
        let conns = client
            .processgroups()
            .get_connections(&transform_pg_id)
            .await
            .unwrap();

        let mut total = 0usize;
        let mut with_both_ids = 0usize;
        for c in conns.connections.unwrap_or_default() {
            total += 1;
            let has_src = c.source_id.as_deref().is_some_and(|s| !s.is_empty());
            let has_dst = c.destination_id.as_deref().is_some_and(|s| !s.is_empty());
            if has_src && has_dst {
                with_both_ids += 1;
            }
        }
        eprintln!(
            "  orders-pipeline/transform: {total} connections, \
             {with_both_ids} with both endpoint ids populated"
        );
        assert!(
            total > 0,
            "expected /connections on orders-pipeline/transform to return at least one \
             connection on {version}"
        );
        assert_eq!(
            with_both_ids, total,
            "all connections in orders-pipeline/transform on {version} must have source_id \
             and destination_id populated via the per-PG /connections fetch"
        );
    }
}
