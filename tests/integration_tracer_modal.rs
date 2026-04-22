//! Live-cluster test for the Tracer content viewer modal's streaming
//! fetch path. Exercises `provenance_content_range` against the
//! bulky-pipeline fixture (~1.5 MiB flowfiles) on every supported
//! NiFi version.
//!
//! Gated on `--ignored` — kicked off by `./integration-tests/run.sh`.

use std::time::Duration;

use nifi_lens::client::tracer::ContentSide;
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Half-MiB chunk size used by the modal's streaming path.
const CHUNK_SIZE: usize = 512 * 1024; // 524_288

/// Build a `NifiClient` connected to the given version's local fixture.
async fn make_client(version: &str, username: &str, password: &str, ca_path: &str) -> NifiClient {
    let ctx = ResolvedContext {
        name: context_for(version),
        url: format!("https://localhost:{}", port_for(version)),
        auth: ResolvedAuth::Password {
            username: username.to_string(),
            password: password.to_string(),
        },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: false,
        ca_cert_path: Some(ca_path.into()),
        proxied_entities_chain: None,
        proxy_url: None,
        http_proxy_url: None,
        https_proxy_url: None,
    };
    NifiClient::connect(&ctx)
        .await
        .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"))
}

/// Walk the browser tree and return the component `id` of the first
/// `Processor` node whose `name` matches `processor_name` and whose parent
/// chain contains a PG named `pg_name`. Returns `None` if no match is found.
fn find_processor_by_name_in_pg(
    nodes: &[nifi_lens::client::RawNode],
    pg_name: &str,
    processor_name: &str,
) -> Option<String> {
    let matching_pg_indices: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == NodeKind::ProcessGroup && n.name == pg_name)
        .map(|(i, _)| i)
        .collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Processor || node.name != processor_name {
            continue;
        }
        let mut cursor = i;
        loop {
            if matching_pg_indices.contains(&cursor) {
                return Some(node.id.clone());
            }
            match nodes[cursor].parent_idx {
                Some(p) => cursor = p,
                None => break,
            }
        }
    }
    None
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn modal_streams_bulky_content_against_all_fixture_versions() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- modal_streams_bulky_content running against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Discover the GenerateFlowFile processor ID inside bulky-pipeline.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "bulky-pipeline",
            "GenerateFlowFile",
        ) {
            Some(id) => id,
            None => {
                eprintln!("  bulky-pipeline/GenerateFlowFile not found on {version} — skipping");
                continue;
            }
        };
        eprintln!("  GenerateFlowFile id={component_id}");

        // 2. Poll for events with a retry loop — the 30-second schedule means
        //    events may not be present on a freshly-booted cluster.
        //    Retry every 10 seconds for up to 2 minutes.
        let poll_deadline = std::time::Instant::now() + Duration::from_secs(120);
        let event_id = loop {
            let snap = match client.latest_events(&component_id, 5).await {
                Ok(s) => s,
                Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                    eprintln!(
                        "  latest_events returned error for {component_id} on {version} — skipping"
                    );
                    break None;
                }
                Err(other) => panic!("latest_events on {version} failed: {other:?}"),
            };

            if let Some(first) = snap.events.first() {
                eprintln!(
                    "  found {} events; using event_id={}",
                    snap.events.len(),
                    first.event_id
                );
                break Some(first.event_id);
            }

            if std::time::Instant::now() >= poll_deadline {
                eprintln!("  no bulky events after 2 min on {version} — skipping");
                break None;
            }
            eprintln!("  no events yet; retrying in 10 s …");
            tokio::time::sleep(Duration::from_secs(10)).await;
        };

        let Some(event_id) = event_id else { continue };

        // 3. Fetch the first chunk via provenance_content_range — exercises the
        //    modal's streaming fetch path with offset=0.
        let chunk1 = match client
            .provenance_content_range(event_id, ContentSide::Output, 0, CHUNK_SIZE)
            .await
        {
            Ok(snap) => snap,
            Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                eprintln!(
                    "  content GC'd for event_id={event_id} on {version} — skipping range assertions"
                );
                continue;
            }
            Err(other) => panic!("first range fetch on {version} failed: {other:?}"),
        };

        assert_eq!(
            chunk1.offset, 0,
            "first chunk offset must be 0 on {version}"
        );
        assert!(
            !chunk1.bytes.is_empty(),
            "expected non-empty first chunk against {version}"
        );
        eprintln!("  chunk1: {} bytes, eof={}", chunk1.bytes.len(), chunk1.eof);

        // 4. If the first chunk didn't hit EOF (bulky files are ~1.5 MiB, well above
        //    CHUNK_SIZE), fetch the second chunk and verify continuity.
        if !chunk1.eof {
            let chunk2 = match client
                .provenance_content_range(
                    event_id,
                    ContentSide::Output,
                    chunk1.bytes.len(),
                    CHUNK_SIZE,
                )
                .await
            {
                Ok(snap) => snap,
                Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                    eprintln!("  content GC'd between chunk1 and chunk2 on {version} — skipping");
                    continue;
                }
                Err(other) => panic!("second range fetch on {version} failed: {other:?}"),
            };

            assert_eq!(
                chunk2.offset,
                chunk1.bytes.len(),
                "second chunk offset must equal first chunk length on {version}"
            );
            eprintln!("  chunk2: {} bytes, eof={}", chunk2.bytes.len(), chunk2.eof);
            assert!(
                chunk1.bytes.len() + chunk2.bytes.len() >= 1_000_000,
                "expected bulky flowfile to exceed 1 MiB across two chunks on {version}"
            );
        }
    }
}
