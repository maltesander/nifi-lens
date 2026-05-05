//! Integration test: Events watch sub-mode against the live Docker
//! fixture. Asserts that `spawn_watch` emits at least one
//! [`EventsPayload::WatchMatch`] within 60 s when watching
//! `transform/UpdateRecord-fx-rate` with a permissive `filename`
//! predicate.
//!
//! `#[ignore]`-gated — only runs via `./integration-tests/run.sh`,
//! which boots the docker-compose fixture and seeds the headline
//! pipeline (including the `transform/UpdateRecord-fx-rate`
//! processor that drives the demo narrative). The test loops over
//! every version in `FIXTURE_VERSIONS` to mirror the rest of the
//! integration suite.
//!
//! Assumption: the seeder's order pipeline keeps producing
//! flowfiles through `transform/UpdateRecord-fx-rate` regardless of
//! `--break-after`. After the break is applied the flowfiles route
//! to `deadletter` instead of the success path, but the processor
//! still emits provenance events so a permissive predicate
//! (`filename =~ /.+/`) keeps matching.

use std::sync::Arc;
use std::time::Duration;

use nifi_lens::client::{NifiClient, NodeKind, Predicate, ProvenanceQuery};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::event::{AppEvent, EventsPayload, ViewPayload};
use nifi_lens::view::events::worker::spawn_watch;
use tokio::sync::{RwLock, mpsc};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Walk the browser tree and return the component `id` of the first
/// `Processor` node whose `name` matches `processor_name` and whose
/// parent chain ends with the PG path `pg_path` (slash-separated,
/// root-to-leaf). Mirrors the helper in
/// `tests/integration_tracer_lineage.rs`.
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

        let mut ancestor_names: Vec<&str> = Vec::new();
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            cursor = p;
            if matches!(nodes[cursor].kind, NodeKind::ProcessGroup) {
                ancestor_names.push(nodes[cursor].name.as_str());
            }
        }
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

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn watch_emits_match_for_fx_rate_processor_in_fixture() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_events_watch running against NiFi {version} ---");

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

        // Resolve the headline-narrative processor's component id by
        // walking the PG tree. `transform/UpdateRecord-fx-rate` lives
        // under the `orders-pipeline/transform` PG path.
        let pg_snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let processor_id = find_processor_by_name_in_pg(
            &pg_snapshot.nodes,
            "orders-pipeline/transform",
            "UpdateRecord-fx-rate",
        )
        .unwrap_or_else(|| {
            panic!("UpdateRecord-fx-rate not found in orders-pipeline/transform on {version}")
        });

        eprintln!("  processor_id = {processor_id}");

        // Permissive predicate: any non-empty `filename` attribute.
        // GenerateFlowFile-produced flowfiles always carry a filename,
        // so this matches every real provenance event for the
        // processor regardless of the break state.
        let predicate = Predicate::parse("filename =~ /.+/").expect("predicate parse");

        let narrow = ProvenanceQuery {
            component_id: Some(processor_id.clone()),
            event_types: vec![],
            max_results: 1000,
            ..Default::default()
        };

        let client = Arc::new(RwLock::new(client));
        let (tx, mut rx) = mpsc::channel::<AppEvent>(64);

        let handle = spawn_watch(
            client.clone(),
            tx,
            narrow,
            predicate,
            None,
            Duration::from_secs(2),
            16,
        );

        let mut got_match = false;
        let deadline = std::time::Instant::now() + Duration::from_secs(60);
        while std::time::Instant::now() < deadline {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            let recv_budget = std::cmp::min(remaining, Duration::from_secs(5));
            match tokio::time::timeout(recv_budget, rx.recv()).await {
                Ok(Some(AppEvent::Data(ViewPayload::Events(EventsPayload::WatchMatch {
                    summary,
                    attrs,
                })))) => {
                    eprintln!(
                        "  WatchMatch: event_id={} attrs={}",
                        summary.event_id,
                        attrs.len()
                    );
                    got_match = true;
                    break;
                }
                Ok(Some(AppEvent::Data(ViewPayload::Events(EventsPayload::WatchTick {
                    events_per_sec_ewma,
                    scanned,
                    matched,
                    ..
                })))) => {
                    eprintln!(
                        "  WatchTick: ewma={events_per_sec_ewma:.2} \
                         scanned={scanned} matched={matched}"
                    );
                }
                Ok(Some(AppEvent::Data(ViewPayload::Events(EventsPayload::WatchFailed {
                    error,
                    retry_in_ms,
                })))) => {
                    eprintln!("  WatchFailed: error={error} retry_in_ms={retry_in_ms}");
                }
                Ok(_) => continue,
                Err(_) => continue,
            }
        }

        handle.abort();
        // Brief grace so the RAII Drop's spawn_cleanup DELETE
        // can fire before the runtime shuts down.
        tokio::time::sleep(Duration::from_millis(300)).await;

        assert!(
            got_match,
            "expected at least one WatchMatch within 60s on {version}"
        );
    }
}
