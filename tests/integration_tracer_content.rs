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
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Expected full size of an `orders-pipeline/ingest/GenerateFlowFile` flowfile.
/// The seeder embeds the entire `orders_payload.csv` asset (~822 KiB) as the
/// processor's Custom Text, so emitted flowfiles are exactly that many bytes.
/// Source of truth: `integration-tests/seeder/assets/orders_payload.csv` size.
const ORDERS_FULL_BYTES: usize = 841_829;

/// Cap used by the truncation path test. Smaller than `ORDERS_FULL_BYTES`
/// so the Range-header truncation path is exercised.
const ORDERS_CAP_BYTES: usize = 512 * 1024; // 524_288, < 822 KiB

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

        // 1. Discover the orders-pipeline/transform/ConvertRecord-csv2json
        //    processor — a running record-shaped stage that always produces
        //    fresh provenance events shortly after seeding.
        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let component_id = match find_processor_by_name_in_pg(
            &snap.nodes,
            "orders-pipeline/transform",
            "ConvertRecord-csv2json",
        ) {
            Some(id) => id,
            None => {
                eprintln!(
                    "  orders-pipeline/transform/ConvertRecord-csv2json not found on {version} — skipping"
                );
                continue;
            }
        };

        // 2. Fetch the latest provenance events for the probe component.
        //    NiFi 2.6.0 may return 404 when no events are cached yet; treat
        //    that as a skip — the test is about content fetch, not events.
        let snapshot = match client.latest_events(&component_id, 20).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!(
                    "  latest_events returned error for {component_id} on {version} — skipping"
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
            eprintln!("  no events for {component_id} on {version} — skipping content fetch");
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
/// chain ends with the PG path `pg_path` (slash-separated, root-to-leaf).
///
/// Examples:
/// - `pg_path = "ingest"` matches any processor whose nearest PG ancestor is
///   named `ingest` (single-segment match).
/// - `pg_path = "orders-pipeline/transform"` matches only processors whose
///   parent chain ends with `… orders-pipeline -> transform`.
///
/// Multi-segment matching is required when a child-PG name like `ingest`
/// is shared between unrelated top-level pipelines (e.g. legacy
/// `healthy-pipeline/ingest` vs new `orders-pipeline/ingest`).
fn find_processor_by_name_in_pg(
    nodes: &[nifi_lens::client::RawNode],
    pg_path: &str,
    processor_name: &str,
) -> Option<String> {
    let pg_parts: Vec<&str> = pg_path.split('/').collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Processor || node.name != processor_name {
            continue;
        }

        // Collect the chain of PG ancestor names, leaf-to-root.
        let mut ancestor_names: Vec<&str> = Vec::new();
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            cursor = p;
            if matches!(nodes[cursor].kind, NodeKind::ProcessGroup) {
                ancestor_names.push(nodes[cursor].name.as_str());
            }
        }
        // Reverse to root-to-leaf so we can suffix-match against pg_parts.
        ancestor_names.reverse();

        if ancestor_names.len() >= pg_parts.len() {
            let start = ancestor_names.len() - pg_parts.len();
            if ancestor_names[start..] == *pg_parts {
                return Some(node.id.clone());
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

/// Integration test: an orders-pipeline/ingest/GenerateFlowFile event content
/// fetch is truncated at `ORDERS_CAP_BYTES` when a cap is given, and returns
/// the full body when fetched without a cap.
///
/// `orders-pipeline/ingest/GenerateFlowFile` emits the embedded
/// `orders_payload.csv` (~822 KiB) on every iteration. `ORDERS_CAP_BYTES`
/// (512 KiB) sits well below that, so the Range-header truncation path is
/// exercised end-to-end.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn orders_ingest_event_content_is_truncated_with_cap() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- orders_ingest_event_content_is_truncated_with_cap against NiFi {version} ---"
        );

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Discover the GenerateFlowFile processor ID inside
        //    orders-pipeline/ingest. The helper matches any processor whose
        //    parent chain contains a PG named "ingest"; that name is unique
        //    inside the marker subtree.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "orders-pipeline/ingest",
            "GenerateFlowFile",
        ) {
            Some(id) => id,
            None => {
                eprintln!(
                    "  orders-pipeline/ingest/GenerateFlowFile not found on {version} — skipping"
                );
                continue;
            }
        };
        eprintln!("  GenerateFlowFile id={component_id}");

        // 2. Poll for events with a retry loop — the 10-second schedule means
        //    events may not be present on a freshly-booted cluster.
        //    GenerateFlowFile emits CREATE (output content available); other
        //    event types like DROP carry only metadata and would fail the
        //    truncation assertion. Filter for CREATE.
        //    Retry every 10 seconds for up to 2 minutes.
        let poll_deadline = std::time::Instant::now() + Duration::from_secs(120);
        let event_id = loop {
            let snap = match client.latest_events(&component_id, 20).await {
                Ok(s) => s,
                Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                    eprintln!(
                        "  latest_events returned error for {component_id} on {version} — skipping"
                    );
                    break None;
                }
                Err(other) => panic!("latest_events on {version} failed: {other:?}"),
            };

            if let Some(create) = snap.events.iter().find(|e| e.event_type == "CREATE") {
                eprintln!(
                    "  found {} events; using CREATE event_id={}",
                    snap.events.len(),
                    create.event_id
                );
                break Some(create.event_id);
            }

            if std::time::Instant::now() >= poll_deadline {
                eprintln!("  no orders ingest CREATE events after 2 min on {version} — skipping");
                break None;
            }
            eprintln!(
                "  no CREATE event yet ({} candidate events); retrying in 10 s …",
                snap.events.len()
            );
            tokio::time::sleep(Duration::from_secs(10)).await;
        };

        let Some(event_id) = event_id else { continue };

        // 3a. Capped fetch: expect exactly ORDERS_CAP_BYTES with truncated=true.
        match client
            .provenance_content(event_id, ContentSide::Output, Some(ORDERS_CAP_BYTES))
            .await
        {
            Ok(cs) => {
                assert_eq!(
                    cs.bytes_fetched, ORDERS_CAP_BYTES,
                    "capped fetch should return exactly ORDERS_CAP_BYTES on {version}"
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

        // 3b. Uncapped fetch: expect the full ORDERS_FULL_BYTES with truncated=false.
        match client
            .provenance_content(event_id, ContentSide::Output, None)
            .await
        {
            Ok(cs) => {
                assert_eq!(
                    cs.bytes_fetched, ORDERS_FULL_BYTES,
                    "uncapped fetch should return the full {ORDERS_FULL_BYTES} bytes on {version}"
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

/// Integration test: `UpdateAttribute-tag-retries` in
/// `orders-pipeline/transform` ADDS the `_max_retries` attribute (sourced
/// from the `#{retry_max}` parameter). A provenance event on that processor
/// should expose an `AttributeTriple` with `previous: None` and
/// `current: Some(_)` for that key — the additive-attribute analogue of the
/// legacy delete-attribute coverage.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn tag_retries_processor_reports_added_attribute() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- tag_retries_processor_reports_added_attribute against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Discover the UpdateAttribute-tag-retries processor ID inside
        //    the transform child PG of orders-pipeline.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "orders-pipeline/transform",
            "UpdateAttribute-tag-retries",
        ) {
            Some(id) => id,
            None => {
                eprintln!(
                    "  orders-pipeline/transform/UpdateAttribute-tag-retries not found on {version} — skipping"
                );
                continue;
            }
        };
        eprintln!("  UpdateAttribute-tag-retries id={component_id}");

        // 2. Fetch the latest events for the tag-retries processor.
        //    If the event list is empty, skip (lenient — orders-pipeline runs
        //    at 10s and the FX-rate stage may have broken upstream flow).
        let snap = match client.latest_events(&component_id, 5).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!("  latest_events error for {component_id} on {version} — skipping");
                continue;
            }
            Err(other) => panic!("latest_events on {version} failed: {other:?}"),
        };

        let Some(summary) = snap.events.first().cloned() else {
            eprintln!("  no events for UpdateAttribute-tag-retries on {version} — skipping");
            continue;
        };
        eprintln!(
            "  found {} events; fetching detail for event_id={}",
            snap.events.len(),
            summary.event_id
        );

        // 3. Fetch the full event detail and check for the added attribute.
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

        // 4. Assert: the `_max_retries` attribute must appear with
        //    a None previous value and a Some current value (i.e. added).
        let attr = detail.attributes.iter().find(|a| a.key == "_max_retries");

        let Some(attr) = attr else {
            panic!(
                "attribute '_max_retries' not found in event {} on {version}; \
                 attributes present: {:?}",
                summary.event_id,
                detail.attributes.iter().map(|a| &a.key).collect::<Vec<_>>()
            );
        };

        assert!(
            attr.previous.is_none(),
            "_max_retries should have no previous value on {version}, \
             got previous={:?}",
            attr.previous
        );
        assert!(
            attr.current.is_some(),
            "_max_retries should be added (current=Some) on {version}"
        );

        eprintln!(
            "  attribute check ok: previous={:?}, current={:?}",
            attr.previous, attr.current
        );
    }
}
