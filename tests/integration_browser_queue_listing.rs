//! Live-fixture integration tests for the Browser queue listing panel.
//!
//! Runs via `./integration-tests/run.sh` against both the 2.6.0 floor
//! and the 2.9.0 cluster fixture. Marked `#[ignore]` so the default
//! `cargo test` skips them.

use std::time::{Duration, Instant};

use nifi_lens::client::queues;
use nifi_lens::client::{NifiClient, NodeKind};
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

/// Find the connection id for the backpressure-pipeline's queue.
///
/// Walks the root PG status snapshot to find the PG named
/// "backpressure-pipeline", then fetches its connections via
/// `GET /process-groups/{pg_id}/connections` and returns the first
/// connection's id.
async fn find_backpressure_connection_id(client: &NifiClient, version: &str) -> String {
    let snap = client
        .root_pg_status()
        .await
        .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

    let pg_node = snap
        .nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.name == "backpressure-pipeline")
        .unwrap_or_else(|| panic!("backpressure-pipeline PG not found in fixture on {version}"));

    let conns = client
        .processgroups()
        .get_connections(&pg_node.id)
        .await
        .unwrap_or_else(|e| {
            panic!("get_connections for backpressure-pipeline on {version} failed: {e:?}")
        });

    conns
        .connections
        .unwrap_or_default()
        .into_iter()
        .next()
        .and_then(|c| c.id)
        .unwrap_or_else(|| panic!("backpressure-pipeline has no connections on {version}"))
}

/// Poll a listing request until finished or the 30-second ceiling is hit.
async fn wait_for_listing_finished(
    client: &NifiClient,
    queue_id: &str,
    request_id: &str,
    version: &str,
) -> nifi_rust_client::dynamic::types::ListingRequestDto {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let dto = queues::poll_listing_request(client, queue_id, request_id)
            .await
            .unwrap_or_else(|e| panic!("poll_listing_request on {version} failed: {e:?}"));
        if dto.finished.unwrap_or(false) {
            return dto;
        }
        assert!(
            Instant::now() < deadline,
            "listing request for queue {queue_id} did not finish within 30 s on {version}"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
}

// ---------------------------------------------------------------------------
// Test 1: listing populates rows for the backpressure pipeline
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "live fixture"]
async fn listing_populates_for_backpressure_pipeline() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- listing_populates_for_backpressure_pipeline running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let queue_id = find_backpressure_connection_id(&client, version).await;
        eprintln!("  backpressure connection id: {queue_id}");

        let dto = queues::submit_listing_request(&client, &queue_id)
            .await
            .unwrap_or_else(|e| panic!("submit_listing_request on {version} failed: {e:?}"));
        let request_id = dto
            .id
            .clone()
            .unwrap_or_else(|| panic!("submit_listing_request returned no id on {version}"));

        let finished = wait_for_listing_finished(&client, &queue_id, &request_id, version).await;

        let summaries = finished.flow_file_summaries.unwrap_or_default();
        assert!(
            !summaries.is_empty(),
            "expected backed-up flowfiles in backpressure-pipeline queue on {version}"
        );
        assert!(
            summaries[0].uuid.is_some(),
            "first summary must carry a uuid on {version}"
        );
        eprintln!(
            "  got {} flowfile summaries, first uuid: {:?}",
            summaries.len(),
            summaries[0].uuid
        );

        queues::cancel_listing_request(&client, &queue_id, &request_id)
            .await
            .unwrap_or_else(|e| panic!("cancel_listing_request on {version} failed: {e:?}"));
    }
}

// ---------------------------------------------------------------------------
// Test 2: peek returns full attributes (filename key present)
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "live fixture"]
async fn peek_returns_full_attributes() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- peek_returns_full_attributes running against NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let queue_id = find_backpressure_connection_id(&client, version).await;

        let dto = queues::submit_listing_request(&client, &queue_id)
            .await
            .unwrap_or_else(|e| panic!("submit_listing_request on {version} failed: {e:?}"));
        let request_id = dto
            .id
            .clone()
            .unwrap_or_else(|| panic!("submit_listing_request returned no id on {version}"));

        let finished = wait_for_listing_finished(&client, &queue_id, &request_id, version).await;

        let summary = finished
            .flow_file_summaries
            .unwrap_or_default()
            .into_iter()
            .next()
            .unwrap_or_else(|| {
                panic!("no flowfile summaries in backpressure-pipeline queue on {version}")
            });
        let uuid = summary
            .uuid
            .unwrap_or_else(|| panic!("summary has no uuid on {version}"));
        let cluster_node_id = summary.cluster_node_id.as_deref();

        eprintln!("  peeking uuid={uuid} cluster_node_id={cluster_node_id:?}");

        let ff = queues::get_flowfile(&client, &queue_id, &uuid, cluster_node_id)
            .await
            .unwrap_or_else(|e| panic!("get_flowfile on {version} failed: {e:?}"));

        assert_eq!(
            ff.uuid.as_deref(),
            Some(uuid.as_str()),
            "uuid roundtrip on {version}"
        );

        let attrs = ff.attributes.unwrap_or_default();
        assert!(
            attrs.contains_key("filename"),
            "expected 'filename' key in flowfile attributes on {version}; got keys: {:?}",
            attrs.keys().collect::<Vec<_>>()
        );
        eprintln!(
            "  attributes ok, filename={:?}",
            attrs.get("filename").and_then(|v| v.as_deref())
        );

        queues::cancel_listing_request(&client, &queue_id, &request_id)
            .await
            .unwrap_or_else(|e| panic!("cancel on {version} failed: {e:?}"));
    }
}

// ---------------------------------------------------------------------------
// Test 3: cancel is idempotent — second DELETE (404) returns Ok
// ---------------------------------------------------------------------------

#[tokio::test(flavor = "current_thread")]
#[ignore = "live fixture"]
async fn cancel_listing_request_idempotent() {
    // Run against the floor version only — the idempotency is client-side
    // logic, not version-specific NiFi behavior.
    let version = *FIXTURE_VERSIONS
        .first()
        .expect("FIXTURE_VERSIONS non-empty");
    eprintln!("--- cancel_listing_request_idempotent running against NiFi {version} ---");

    let ctx = it_context(version);
    let client = NifiClient::connect(&ctx)
        .await
        .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

    let queue_id = find_backpressure_connection_id(&client, version).await;

    let dto = queues::submit_listing_request(&client, &queue_id)
        .await
        .unwrap_or_else(|e| panic!("submit on {version} failed: {e:?}"));
    let request_id = dto
        .id
        .clone()
        .unwrap_or_else(|| panic!("no request id on {version}"));

    // First cancel — should succeed.
    queues::cancel_listing_request(&client, &queue_id, &request_id)
        .await
        .unwrap_or_else(|e| panic!("first cancel on {version} failed: {e:?}"));
    eprintln!("  first cancel ok");

    // Second cancel — the request is gone; NiFi returns 404, which
    // `cancel_listing_request` maps to Ok(()) (best-effort cleanup).
    queues::cancel_listing_request(&client, &queue_id, &request_id)
        .await
        .unwrap_or_else(|e| panic!("second cancel (idempotent) on {version} failed: {e:?}"));
    eprintln!("  second cancel ok (idempotent)");
}
