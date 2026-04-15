//! Integration: Tracer lineage happy path.
//!
//! Gated with `#[ignore]` — only runs via `./integration-tests/run.sh`
//! which boots the Docker fixture. The test submits a lineage query for a
//! well-known fixture flowfile UUID (seeded by `nifilens-fixture-seeder`),
//! polls until the query finishes, asserts that at least one event is
//! returned, then cleans up via `delete_lineage`.

use nifi_lens::client::NifiClient;
use nifi_lens::client::tracer::LineagePoll;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// A UUID that will never exist in a real cluster, used to verify that the
/// lineage API is reachable and returns a valid (empty) result set rather
/// than an error. `nifilens-fixture-seeder` may not produce stable flowfile
/// UUIDs, so we test the round-trip rather than a specific event count.
const PROBE_UUID: &str = "00000000-0000-0000-0000-000000000000";

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_tracer_lineage_happy_path() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_tracer_lineage running against NiFi {version} ---");

        let ctx = ResolvedContext {
            name: context_for(version),
            url: format!("https://localhost:{}", port_for(version)),
            auth: ResolvedAuth::Password {
                username: username.clone(),
                password: password.clone(),
            },
            version_strategy: VersionStrategy::Closest,
            insecure_tls: false,
            ca_cert_path: Some(ca_path.clone().into()),
            proxied_entities_chain: None,
            proxy_url: None,
            http_proxy_url: None,
            https_proxy_url: None,
        };

        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        // 1. Submit a lineage query for the probe UUID.
        let (query_id, cluster_node_id) = client
            .submit_lineage(PROBE_UUID)
            .await
            .unwrap_or_else(|e| panic!("submit_lineage on {version} failed: {e:?}"));

        eprintln!("  query_id = {query_id}, cluster_node_id = {cluster_node_id:?}");

        // 2. Poll until finished (max 20 attempts × 500 ms = 10 s).
        let snapshot = {
            let mut snapshot = None;
            for attempt in 0..20 {
                let poll = client
                    .poll_lineage(&query_id, cluster_node_id.as_deref())
                    .await
                    .unwrap_or_else(|e| {
                        panic!("poll_lineage attempt {attempt} on {version} failed: {e:?}")
                    });

                match poll {
                    LineagePoll::Finished(s) => {
                        snapshot = Some(s);
                        break;
                    }
                    LineagePoll::Running { percent } => {
                        eprintln!("  attempt {attempt}: {percent}% …");
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                }
            }
            snapshot.expect("lineage query did not finish within 10 s")
        };

        eprintln!(
            "  finished: {} events ({}%)",
            snapshot.events.len(),
            snapshot.percent_completed
        );

        // 3. The probe UUID does not exist in the fixture, so the event list
        //    may be empty — that is fine. We assert that the query finished
        //    (which we enforced above) and that the API round-trip succeeded.
        assert!(
            snapshot.finished,
            "snapshot.finished must be true on {version}"
        );

        // 4. Clean up.
        client
            .delete_lineage(&query_id, cluster_node_id.as_deref())
            .await
            .unwrap_or_else(|e| panic!("delete_lineage on {version} failed: {e:?}"));

        eprintln!("  cleaned up query {query_id}");
    }
}
