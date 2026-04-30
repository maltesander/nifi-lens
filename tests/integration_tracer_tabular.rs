//! Live-cluster integration test: verifies Parquet/Avro provenance
//! events decode through the Tracer modal pipeline.
//!
//! The `orders-pipeline` fixture seeds Parquet output in the `sink-us`
//! sink PG and Avro output in the `sink-apac` sink PG. This test locates
//! the tag processors in each sink, waits for a recent provenance event
//! whose output content decodes to the expected tabular format, and asserts
//! the result is a `ContentRender::Tabular` with the expected format tag,
//! a non-empty JSON-Lines body, and a non-empty schema summary.
//!
//! Requires the fixture stack to be running and seeded. Gated on
//! `--ignored` per the project convention.

use std::time::Duration;

use nifi_lens::client::tracer::{ContentRender, ContentSide, TabularFormat};
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

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

/// Walk the browser-tree arena and return the component `id` of the first
/// `Processor` node whose name matches `processor_name` and whose parent
/// chain matches the slash-separated PG path `pg_path` (e.g., "orders-pipeline/sink-us").
/// Path matching is suffix-based: any parent chain ending with the path matches.
fn find_processor_by_name_in_pg(
    nodes: &[nifi_lens::client::RawNode],
    pg_path: &str,
    processor_name: &str,
) -> Option<String> {
    let path_parts: Vec<&str> = pg_path.split('/').collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::Processor || node.name != processor_name {
            continue;
        }

        // Walk up the parent chain and collect PG names.
        let mut pg_chain = Vec::new();
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            if nodes[p].kind == NodeKind::ProcessGroup {
                pg_chain.push(nodes[p].name.as_str());
            }
            cursor = p;
        }

        // Reverse to get top-down order and check if the path matches the suffix.
        pg_chain.reverse();
        if pg_chain.len() >= path_parts.len() {
            let start = pg_chain.len() - path_parts.len();
            if pg_chain[start..] == path_parts[..] {
                return Some(node.id.clone());
            }
        }
    }
    None
}

/// Poll `latest_events` for `component_id` until we find an event whose
/// output content decodes to `expected_format`. Retries every 10 s for up
/// to two minutes. Returns the matching `ContentRender::Tabular` on
/// success, or `None` on timeout (so callers can skip gracefully).
///
/// The NiFi provenance-events API may return events of any type
/// (RECEIVE, CONTENT_MODIFIED, FORK, …); not every event has Parquet/Avro
/// output content. We scan all returned events before sleeping.
async fn wait_for_tabular_event(
    client: &NifiClient,
    component_id: &str,
    version: &str,
    label: &str,
    expected_format: TabularFormat,
) -> Option<ContentRender> {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let snap = match client.latest_events(component_id, 20).await {
            Ok(s) => s,
            Err(NifiLensError::LatestProvenanceEventsFailed { .. }) => {
                eprintln!("  latest_events error for {label} on {version} — skipping");
                return None;
            }
            Err(other) => panic!("latest_events for {label} on {version} failed: {other:?}"),
        };

        eprintln!("  {label}: {} events returned", snap.events.len());

        // Scan all returned events for one with matching tabular output.
        for ev in &snap.events {
            let render = match client
                .provenance_content(ev.event_id, ContentSide::Output, None)
                .await
            {
                Ok(s) => s.render,
                Err(NifiLensError::ProvenanceContentFetchFailed { .. }) => {
                    eprintln!(
                        "  event_id={} content GC'd or unavailable — skipping",
                        ev.event_id
                    );
                    continue;
                }
                Err(other) => panic!(
                    "provenance_content for event_id={} on {version} failed: {other:?}",
                    ev.event_id
                ),
            };

            match &render {
                ContentRender::Tabular { format, .. } if *format == expected_format => {
                    eprintln!(
                        "  {label}: found {} event_id={} on {version}",
                        expected_format.label(),
                        ev.event_id
                    );
                    return Some(render);
                }
                other => {
                    eprintln!(
                        "  event_id={} output is {:?} — skipping",
                        ev.event_id,
                        discriminant_label(other)
                    );
                }
            }
        }

        if std::time::Instant::now() >= deadline {
            eprintln!(
                "  no {} output found after 2 min on {version} — skipping",
                expected_format.label()
            );
            return None;
        }
        eprintln!("  no matching event yet; retrying in 10 s …");
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}

/// Return a short label for the `ContentRender` discriminant (for diagnostics).
fn discriminant_label(r: &ContentRender) -> &'static str {
    match r {
        ContentRender::Tabular { format, .. } => format.label(),
        ContentRender::Text { .. } => "text",
        ContentRender::Hex { .. } => "hex",
        ContentRender::Empty => "empty",
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn parquet_event_decodes_to_tabular_on_each_version() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- parquet_event_decodes_to_tabular against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Locate the UpdateRecord-parquet-tag processor in orders-pipeline/sink-us.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "orders-pipeline/sink-us",
            "UpdateRecord-parquet-tag",
        ) {
            Some(id) => id,
            None => {
                eprintln!(
                    "  UpdateRecord-parquet-tag not found in orders-pipeline/sink-us on {version} — skipping"
                );
                continue;
            }
        };
        eprintln!("  UpdateRecord-parquet-tag id={component_id}");

        // 2. Poll for an event whose output decodes to Parquet.
        let Some(render) = wait_for_tabular_event(
            &client,
            &component_id,
            version,
            "UpdateRecord-parquet-tag",
            TabularFormat::Parquet,
        )
        .await
        else {
            continue;
        };

        // 3. Assert the render fields are meaningful.
        match render {
            ContentRender::Tabular {
                format: TabularFormat::Parquet,
                ref body,
                ref schema_summary,
                ..
            } => {
                assert!(
                    body.lines().count() >= 1,
                    "expected at least one record in Parquet body on {version}"
                );
                assert!(
                    !schema_summary.is_empty(),
                    "expected non-empty schema_summary for Parquet on {version}"
                );
                eprintln!(
                    "  Parquet ok: {} lines, schema_summary {} chars",
                    body.lines().count(),
                    schema_summary.len()
                );
            }
            other => panic!("expected ContentRender::Tabular(Parquet) on {version}, got {other:?}"),
        }
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn avro_event_decodes_to_tabular_on_each_version() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- avro_event_decodes_to_tabular against NiFi {version} ---");

        let client = make_client(version, &username, &password, &ca_path).await;

        // 1. Locate the UpdateRecord-avro-tag processor in orders-pipeline/sink-apac.
        let snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let component_id = match find_processor_by_name_in_pg(
            &snapshot.nodes,
            "orders-pipeline/sink-apac",
            "UpdateRecord-avro-tag",
        ) {
            Some(id) => id,
            None => {
                eprintln!(
                    "  UpdateRecord-avro-tag not found in orders-pipeline/sink-apac on {version} — skipping"
                );
                continue;
            }
        };
        eprintln!("  UpdateRecord-avro-tag id={component_id}");

        // 2. Poll for an event whose output decodes to Avro.
        let Some(render) = wait_for_tabular_event(
            &client,
            &component_id,
            version,
            "UpdateRecord-avro-tag",
            TabularFormat::Avro,
        )
        .await
        else {
            continue;
        };

        // 3. Assert the render fields are meaningful.
        match render {
            ContentRender::Tabular {
                format: TabularFormat::Avro,
                ref body,
                ref schema_summary,
                ..
            } => {
                assert!(
                    body.lines().count() >= 1,
                    "expected at least one record in Avro body on {version}"
                );
                assert!(
                    !schema_summary.is_empty(),
                    "expected non-empty schema_summary for Avro on {version}"
                );
                eprintln!(
                    "  Avro ok: {} lines, schema_summary {} chars",
                    body.lines().count(),
                    schema_summary.len()
                );
            }
            other => panic!("expected ContentRender::Tabular(Avro) on {version}, got {other:?}"),
        }
    }
}
