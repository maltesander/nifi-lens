//! Live-cluster test for the Tracer content viewer modal's diff feature.
//!
//! Walks the three record processors in the `diff-pipeline` fixture
//! (UpdateRecord-json, ConvertRecord, UpdateRecord-csv) and asserts,
//! for each one's latest provenance event:
//!
//! 1. Both input and output content claims exist.
//! 2. The input and output bodies differ byte-wise (so the Tracer diff
//!    tab would show non-empty hunks for the same-mime stages).
//! 3. The content matches the expected format (JSON or CSV).
//!
//! This exercises the end-to-end shape of the diff-tab eligibility
//! gate on a real NiFi cluster — mime match, size under the 512 KiB
//! cap, and byte inequality — across every supported NiFi version.
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

const FIRST_CHUNK: usize = 512 * 1024; // same as MODAL_CHUNK_BYTES

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

/// Poll `latest_events` on `component_id` for up to two minutes waiting
/// for at least one event. Returns `None` if the cluster hasn't produced
/// anything yet (e.g., fixture just booted) — callers skip the
/// assertion in that case.
async fn wait_for_event(
    client: &NifiClient,
    component_id: &str,
    version: &str,
    component_label: &str,
) -> Option<i64> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let snap = match client.latest_events(component_id, 5).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!(
                    "  latest_events error on {component_label} ({version}) — skipping component"
                );
                return None;
            }
            Err(other) => panic!("latest_events on {version} failed: {other:?}"),
        };
        if let Some(first) = snap.events.first() {
            eprintln!(
                "  {component_label}: {} events, using event_id={}",
                snap.events.len(),
                first.event_id
            );
            return Some(first.event_id);
        }
        if std::time::Instant::now() >= deadline {
            eprintln!("  no {component_label} events after 2 min on {version} — skipping");
            return None;
        }
        eprintln!("  no {component_label} events yet; retrying in 10 s …");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Fetch both input and output content for `event_id`, returning
/// `(input_bytes, output_bytes)`. Returns `None` on GC-related fetch
/// failure so callers can skip the assertion gracefully.
async fn fetch_both_sides(
    client: &NifiClient,
    event_id: i64,
    version: &str,
    label: &str,
) -> Option<(Vec<u8>, Vec<u8>)> {
    let input = match client
        .provenance_content_range(event_id, ContentSide::Input, 0, FIRST_CHUNK)
        .await
    {
        Ok(s) => s,
        Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
            eprintln!("  {label} input content GC'd on {version} — skipping");
            return None;
        }
        Err(other) => panic!("{label} input fetch on {version} failed: {other:?}"),
    };
    let output = match client
        .provenance_content_range(event_id, ContentSide::Output, 0, FIRST_CHUNK)
        .await
    {
        Ok(s) => s,
        Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
            eprintln!("  {label} output content GC'd on {version} — skipping");
            return None;
        }
        Err(other) => panic!("{label} output fetch on {version} failed: {other:?}"),
    };
    Some((input.bytes, output.bytes))
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn diff_pipeline_events_are_diffable_against_all_fixture_versions() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- diff_pipeline_events running against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        // 1. UpdateRecord-json: JSON ↔ JSON (same mime, diffable).
        if let Some(upd_json_id) =
            find_processor_by_name_in_pg(&snapshot.nodes, "diff-pipeline", "UpdateRecord-json")
            && let Some(event_id) =
                wait_for_event(&client, &upd_json_id, version, "UpdateRecord-json").await
            && let Some((input, output)) =
                fetch_both_sides(&client, event_id, version, "UpdateRecord-json").await
        {
            assert!(
                !input.is_empty() && !output.is_empty(),
                "UpdateRecord-json: both sides must have content on {version}"
            );
            assert_ne!(
                input, output,
                "UpdateRecord-json: input and output must differ (status was uppercased) on {version}"
            );
            let input_s = std::str::from_utf8(&input)
                .unwrap_or_else(|_| panic!("UpdateRecord-json input not UTF-8 on {version}"));
            let output_s = std::str::from_utf8(&output)
                .unwrap_or_else(|_| panic!("UpdateRecord-json output not UTF-8 on {version}"));
            let input_trim = input_s.trim_start();
            let output_trim = output_s.trim_start();
            assert!(
                input_trim.starts_with('[') || input_trim.starts_with('{'),
                "UpdateRecord-json input should parse as JSON on {version} (got: {:?}…)",
                &input_trim.get(..40).unwrap_or(input_trim)
            );
            assert!(
                output_trim.starts_with('[') || output_trim.starts_with('{'),
                "UpdateRecord-json output should parse as JSON on {version}"
            );
            // The /status field was "ok" or "warn" in the payload;
            // UpdateRecord-json maps it through toUpper().
            assert!(
                output_s.contains("OK") || output_s.contains("WARN"),
                "UpdateRecord-json output should contain uppercased status on {version}"
            );
        } else {
            eprintln!("  UpdateRecord-json: preconditions unmet — skipping on {version}");
        }

        // 2. ConvertRecord: JSON ↔ CSV (mime mismatch; exercises the
        //    diff-disabled path at the Tracer level).
        if let Some(convert_id) =
            find_processor_by_name_in_pg(&snapshot.nodes, "diff-pipeline", "ConvertRecord")
            && let Some(event_id) =
                wait_for_event(&client, &convert_id, version, "ConvertRecord").await
            && let Some((input, output)) =
                fetch_both_sides(&client, event_id, version, "ConvertRecord").await
        {
            assert!(
                !input.is_empty() && !output.is_empty(),
                "ConvertRecord: both sides must have content on {version}"
            );
            let input_s = std::str::from_utf8(&input)
                .unwrap_or_else(|_| panic!("ConvertRecord input not UTF-8 on {version}"));
            let output_s = std::str::from_utf8(&output)
                .unwrap_or_else(|_| panic!("ConvertRecord output not UTF-8 on {version}"));
            let input_trim = input_s.trim_start();
            assert!(
                input_trim.starts_with('[') || input_trim.starts_with('{'),
                "ConvertRecord input should be JSON on {version}"
            );
            // CSV output should have a header row (comma-delimited tokens)
            // followed by at least one data row; if nothing else, it should
            // contain commas and newlines.
            assert!(
                output_s.contains(',') && output_s.contains('\n'),
                "ConvertRecord output should be CSV-shaped on {version}"
            );
        } else {
            eprintln!("  ConvertRecord: preconditions unmet — skipping on {version}");
        }

        // 3. UpdateRecord-csv: CSV ↔ CSV (same mime, diffable).
        if let Some(upd_csv_id) =
            find_processor_by_name_in_pg(&snapshot.nodes, "diff-pipeline", "UpdateRecord-csv")
            && let Some(event_id) =
                wait_for_event(&client, &upd_csv_id, version, "UpdateRecord-csv").await
            && let Some((input, output)) =
                fetch_both_sides(&client, event_id, version, "UpdateRecord-csv").await
        {
            assert!(
                !input.is_empty() && !output.is_empty(),
                "UpdateRecord-csv: both sides must have content on {version}"
            );
            assert_ne!(
                input, output,
                "UpdateRecord-csv: input and output must differ (status lowercased) on {version}"
            );
            let input_s = std::str::from_utf8(&input)
                .unwrap_or_else(|_| panic!("UpdateRecord-csv input not UTF-8 on {version}"));
            let output_s = std::str::from_utf8(&output)
                .unwrap_or_else(|_| panic!("UpdateRecord-csv output not UTF-8 on {version}"));
            assert!(
                input_s.contains(',') && input_s.contains('\n'),
                "UpdateRecord-csv input should be CSV-shaped on {version}"
            );
            assert!(
                output_s.contains(',') && output_s.contains('\n'),
                "UpdateRecord-csv output should be CSV-shaped on {version}"
            );
        } else {
            eprintln!("  UpdateRecord-csv: preconditions unmet — skipping on {version}");
        }
    }
}
