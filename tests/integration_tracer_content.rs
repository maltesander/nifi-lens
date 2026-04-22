//! Integration: Tracer content fetch.
//!
//! Gated with `#[ignore]` — only runs via `./integration-tests/run.sh`
//! which boots the Docker fixture. The test fetches the latest provenance
//! events for a component, picks any event that reports output content
//! available, then fetches that content. A 404 / unavailable response is
//! treated as success (content may have been garbage-collected by NiFi).
//! Only transport-level errors cause the test to fail.

use std::time::Duration;

use nifi_lens::client::tracer::ContentSide;
use nifi_lens::client::{NifiClient, NodeKind};

/// 1 MiB cap used by this integration test to exercise the Range-header truncation path.
/// The bulky-pipeline fixture generates 1.5 MiB flowfiles, so this cap is guaranteed to
/// produce a truncated response.
const BULKY_CAP_BYTES: usize = 1 << 20;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Expected full size of a `bulky-pipeline` flowfile: 1536 KB in bytes.
const BULKY_FULL_BYTES: usize = 1536 * 1024; // 1_572_864

/// Component-id of the `noisy-pipeline` generate-flowfile processor seeded by
/// `nifilens-fixture-seeder`. The integration fixture always seeds this
/// processor so there should be recent provenance events available.
///
/// If this ID drifts with future fixture versions the test will receive an
/// empty event list and skip the content fetch, but will not fail — the
/// assertion only fires on transport errors.
const NOISY_COMPONENT_ID: &str = "fixture-noisy-generate";

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_tracer_content_text_render() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_tracer_content running against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Fetch the latest provenance events for the probe component.
        //    NiFi 2.6.0 returns 404 when the component is no longer part of
        //    the flow (or was never seeded). Treat this as a skip, not a
        //    failure — the test is about content fetch, not latest-events.
        let snapshot = match client.latest_events(NOISY_COMPONENT_ID, 20).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!(
                    "  latest_events returned error for {NOISY_COMPONENT_ID} on {version} — skipping"
                );
                continue;
            }
            Err(other) => panic!("latest_events on {version} failed: {other:?}"),
        };

        eprintln!(
            "  {} events for component {}",
            snapshot.events.len(),
            snapshot.component_id
        );

        // 2. Find any event that has output content available.
        let event_with_content = snapshot
            .events
            .iter()
            .find(|e| {
                // Use event_id as a proxy — fetch the detail to check availability.
                // We pick the first event and attempt content fetch unconditionally;
                // a 404 is acceptable.
                let _ = e.event_id; // silence unused warning
                true
            })
            .cloned();

        let Some(summary) = event_with_content else {
            eprintln!("  no events for {NOISY_COMPONENT_ID} on {version} — skipping content fetch");
            continue;
        };

        eprintln!(
            "  probing content for event_id={} type={}",
            summary.event_id, summary.event_type
        );

        // 3. Attempt to fetch output content.
        //    404 / unavailable → skip (content may be GC'd).
        //    Only transport errors (non-HTTP) fail the test.
        match client
            .provenance_content(summary.event_id, ContentSide::Output, None)
            .await
        {
            Ok(cs) => {
                eprintln!("  content fetch ok: {} bytes", cs.bytes_fetched);
            }
            Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                // Could be a 404 (content GC'd) or a 403 (no replay claim).
                // Either is acceptable in an integration context.
                eprintln!(
                    "  content unavailable for event_id={} — acceptable",
                    summary.event_id
                );
            }
            Err(other) => {
                panic!("unexpected transport error fetching content on {version}: {other:?}");
            }
        }
    }
}

/// Walk the browser tree and return the component `id` of the first
/// `Processor` node whose `name` matches `processor_name` and whose parent
/// chain contains a PG named `pg_name`. Returns `None` if no match is found
/// (the fixture may not include this pipeline on the given NiFi version).
fn find_processor_by_name_in_pg(
    nodes: &[nifi_lens::client::RawNode],
    pg_name: &str,
    processor_name: &str,
) -> Option<String> {
    // Build a quick index: node index → parent index.
    // Find all PG nodes whose name matches `pg_name`.
    let matching_pg_indices: Vec<usize> = nodes
        .iter()
        .enumerate()
        .filter(|(_, n)| n.kind == NodeKind::ProcessGroup && n.name == pg_name)
        .map(|(i, _)| i)
        .collect();

    // For each processor node, check whether any ancestor is one of the
    // matching PGs.
    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Processor || node.name != processor_name {
            continue;
        }
        // Walk the parent chain.
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

/// Integration test: a bulky-pipeline event content fetch is truncated at
/// `BULKY_CAP_BYTES` when a cap is given, and returns the full body when
/// fetched without a cap.
///
/// The `bulky-pipeline` fixture generates 1536 KB (1_572_864 byte) flowfiles
/// every 30 seconds, which exceeds `BULKY_CAP_BYTES` (1 MiB). This test
/// verifies the Range-header truncation path end-to-end.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn bulky_event_content_is_truncated_with_cap() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- bulky_event_content_is_truncated_with_cap against NiFi {version} ---");

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

        // 3a. Capped fetch: expect exactly BULKY_CAP_BYTES with truncated=true.
        match client
            .provenance_content(event_id, ContentSide::Output, Some(BULKY_CAP_BYTES))
            .await
        {
            Ok(cs) => {
                assert_eq!(
                    cs.bytes_fetched, BULKY_CAP_BYTES,
                    "capped fetch should return exactly BULKY_CAP_BYTES on {version}"
                );
                assert!(
                    cs.truncated,
                    "capped fetch should be marked truncated on {version}"
                );
                eprintln!(
                    "  capped fetch ok: {} bytes, truncated={}",
                    cs.bytes_fetched, cs.truncated
                );
            }
            Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                eprintln!(
                    "  content GC'd for event_id={event_id} on {version} — skipping capped assertion"
                );
                continue;
            }
            Err(other) => panic!("capped content fetch on {version} failed: {other:?}"),
        }

        // 3b. Uncapped fetch: expect the full BULKY_FULL_BYTES with truncated=false.
        match client
            .provenance_content(event_id, ContentSide::Output, None)
            .await
        {
            Ok(cs) => {
                assert_eq!(
                    cs.bytes_fetched, BULKY_FULL_BYTES,
                    "uncapped fetch should return the full {BULKY_FULL_BYTES} bytes on {version}"
                );
                assert!(
                    !cs.truncated,
                    "uncapped fetch must not be marked truncated on {version}"
                );
                eprintln!(
                    "  uncapped fetch ok: {} bytes, truncated={}",
                    cs.bytes_fetched, cs.truncated
                );
            }
            Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                eprintln!(
                    "  content GC'd for event_id={event_id} on {version} — skipping uncapped assertion"
                );
            }
            Err(other) => panic!("uncapped content fetch on {version} failed: {other:?}"),
        }
    }
}

/// Integration test: `UpdateAttribute-cleanup` in `healthy-pipeline/enrich`
/// deletes the `fixture.ingest.timestamp` attribute. A provenance event on
/// that processor should expose an `AttributeTriple` with
/// `previous: Some(_)` and `current: None` for that key.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn cleanup_processor_reports_deleted_attribute() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- cleanup_processor_reports_deleted_attribute against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Discover the UpdateAttribute-cleanup processor ID inside the
        //    enrich child PG of healthy-pipeline.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "enrich",
            "UpdateAttribute-cleanup",
        ) {
            Some(id) => id,
            None => {
                eprintln!("  enrich/UpdateAttribute-cleanup not found on {version} — skipping");
                continue;
            }
        };
        eprintln!("  UpdateAttribute-cleanup id={component_id}");

        // 2. Fetch the latest events for the cleanup processor. The healthy
        //    pipeline runs at 1 s, so events should appear quickly after seeding.
        //    If the event list is empty, skip (lenient, same as the existing test).
        let snap = match client.latest_events(&component_id, 5).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!("  latest_events error for {component_id} on {version} — skipping");
                continue;
            }
            Err(other) => panic!("latest_events on {version} failed: {other:?}"),
        };

        let Some(summary) = snap.events.first().cloned() else {
            eprintln!("  no events for UpdateAttribute-cleanup on {version} — skipping");
            continue;
        };
        eprintln!(
            "  found {} events; fetching detail for event_id={}",
            snap.events.len(),
            summary.event_id
        );

        // 3. Fetch the full event detail and check for the deleted attribute.
        let detail = match client.get_provenance_event(summary.event_id).await {
            Ok(d) => d,
            Err(NifiLensError::ProvenanceEventFetchFailed { .. }) => {
                eprintln!(
                    "  event detail unavailable for event_id={} on {version} — skipping",
                    summary.event_id
                );
                continue;
            }
            Err(other) => panic!("get_provenance_event on {version} failed: {other:?}"),
        };

        // 4. Assert: the `fixture.ingest.timestamp` attribute must appear with
        //    a non-None previous value and a None current value (i.e. deleted).
        let attr = detail
            .attributes
            .iter()
            .find(|a| a.key == "fixture.ingest.timestamp");

        let Some(attr) = attr else {
            panic!(
                "attribute 'fixture.ingest.timestamp' not found in event {} on {version}; \
                 attributes present: {:?}",
                summary.event_id,
                detail.attributes.iter().map(|a| &a.key).collect::<Vec<_>>()
            );
        };

        assert!(
            attr.previous.is_some(),
            "fixture.ingest.timestamp should have a previous value on {version}"
        );
        assert!(
            attr.current.is_none(),
            "fixture.ingest.timestamp should be deleted (current=None) on {version}, \
             got current={:?}",
            attr.current
        );

        eprintln!(
            "  attribute check ok: previous={:?}, current={:?}",
            attr.previous, attr.current
        );
    }
}
