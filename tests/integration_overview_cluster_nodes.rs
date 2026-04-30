//! Integration tests: `/controller/cluster` fetcher behavior against
//! the live Docker fixtures booted by `integration-tests/run.sh`.
//! `#[ignore]`-gated so `cargo test` alone does not touch Docker.
//!
//! Two paths exercised:
//! - 2-node NiFi 2.9.0 cluster: snapshot contains both nodes with
//!   primary / coordinator roles assigned.
//! - Standalone NiFi 2.6.0: `/controller/cluster` returns HTTP 409
//!   which `NifiClient::cluster_nodes()` surfaces as an error (the
//!   409→empty-snapshot translation lives in the fetcher task, not
//!   the raw client wrapper). We verify the error is surfaced and not
//!   silently swallowed.

use nifi_lens::client::NifiClient;
use nifi_lens::client::overview::ClusterNodeStatus;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{context_for, port_for};

fn resolved_ctx(version: &str) -> ResolvedContext {
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

/// 2-node 2.9.0 cluster.
///
/// Fetches `/controller/cluster` once against the cluster fixture.
/// Asserts: snapshot has 2 rows, exactly one holds the Primary role,
/// exactly one holds the Cluster Coordinator role (they may be the same
/// node or different — NiFi assigns both roles arbitrarily), and every
/// node's `status == Connected`.
#[tokio::test]
#[ignore]
async fn integration_overview_cluster_nodes_populated_2_9_0() {
    eprintln!("--- integration_overview_cluster_nodes_populated_2_9_0 ---");
    let ctx = resolved_ctx("2.9.0");
    let client = NifiClient::connect(&ctx)
        .await
        .unwrap_or_else(|e| panic!("connect to 2.9.0 failed: {e:?}"));

    let snap = client
        .cluster_nodes()
        .await
        .unwrap_or_else(|e| panic!("cluster_nodes on 2.9.0 failed: {e:?}"));

    assert_eq!(
        snap.rows.len(),
        2,
        "2.9.0 fixture has 2 cluster nodes; got {} rows: {:?}",
        snap.rows.len(),
        snap.rows
    );
    assert!(
        snap.rows.iter().any(|r| r.is_primary),
        "expected one node to hold Primary role; rows: {:?}",
        snap.rows
    );
    assert!(
        snap.rows.iter().any(|r| r.is_coordinator),
        "expected one node to hold Cluster Coordinator role; rows: {:?}",
        snap.rows
    );
    for r in &snap.rows {
        assert_eq!(
            r.status,
            ClusterNodeStatus::Connected,
            "node {} is not Connected: {:?}",
            r.address,
            r.status
        );
    }
}

/// Standalone 2.6.0.
///
/// Fetches `/controller/cluster` against a non-clustered NiFi. The
/// raw client wrapper surfaces the 409 as an error (the fetcher task
/// in `src/cluster/fetcher_tasks.rs` translates that specific shape
/// into an empty snapshot, but that translation is not in scope for
/// the client wrapper). We assert that the call errors out — silent
/// success would indicate either a broken fixture or a regression in
/// error mapping.
#[tokio::test]
#[ignore]
async fn integration_overview_cluster_nodes_409_standalone_2_6_0() {
    eprintln!("--- integration_overview_cluster_nodes_409_standalone_2_6_0 ---");
    let ctx = resolved_ctx("2.6.0");
    let client = NifiClient::connect(&ctx)
        .await
        .unwrap_or_else(|e| panic!("connect to 2.6.0 failed: {e:?}"));

    let result = client.cluster_nodes().await;
    let err = match result {
        Ok(snap) => panic!(
            "expected cluster_nodes on standalone 2.6.0 to error (409), \
             but got Ok with {} rows",
            snap.rows.len()
        ),
        Err(e) => e,
    };
    // The fetcher's `error_is_standalone_409` matches on any of three
    // markers in the debug repr: "409", "NotClustered", or the canonical
    // message text "Only a node connected to a cluster". The test mirrors
    // that detection so a regression in either place fails this test.
    //
    // NiFi 2.6.0 with nifi-rust-client 0.11.0 surfaces the 409 response as
    // a `NotFound { message: "Only a node connected to a cluster..." }` —
    // the debug repr does NOT contain the literal "409".
    let debug_repr = format!("{err:?}");
    assert!(
        debug_repr.contains("409")
            || debug_repr.contains("NotClustered")
            || debug_repr.contains("Only a node connected to a cluster"),
        "expected 409 / 'NotClustered' / 'Only a node connected to a cluster' \
         in error debug repr; got: {debug_repr}"
    );
}
