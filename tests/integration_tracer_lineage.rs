//! Integration: Tracer lineage happy path.
//!
//! Gated with `#[ignore]` — only runs via `./integration-tests/run.sh`
//! which boots the Docker fixture. The test submits a lineage query for a
//! flowfile from the `orders-pipeline/transform` processors, polls until the
//! query finishes, asserts that at least one event is returned, then cleans
//! up via `delete_lineage`.

use nifi_lens::client::events::{ProvenancePollResult, ProvenanceQuery};
use nifi_lens::client::tracer::LineagePoll;
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Walk the browser tree and return the component `id` of the first
/// `Processor` node whose `name` matches `processor_name` and whose parent
/// chain ends with the PG path `pg_path` (slash-separated, root-to-leaf).
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

        // 1. Fetch the root PG status to locate a processor in orders-pipeline/transform.
        let pg_snapshot = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let processor_id = find_processor_by_name_in_pg(
            &pg_snapshot.nodes,
            "orders-pipeline/transform",
            "UpdateRecord-cancel-old",
        )
        .unwrap_or_else(|| {
            panic!("UpdateRecord-cancel-old not found in orders-pipeline/transform on {version}")
        });

        eprintln!("  processor_id = {processor_id}");

        // 2. Submit a provenance query to find a flowfile UUID from the processor.
        let prov_query = ProvenanceQuery {
            component_id: Some(processor_id.clone()),
            flow_file_uuid: None,
            start_time_iso: None,
            end_time_iso: None,
            max_results: 10,
        };

        let prov_handle = client
            .submit_provenance_query(&prov_query)
            .await
            .unwrap_or_else(|e| panic!("submit_provenance_query on {version} failed: {e:?}"));

        eprintln!("  provenance_query_id = {}", prov_handle.query_id);

        // 3. Poll the provenance query until finished to get a flowfile UUID.
        let flowfile_uuid = {
            let mut uuid: Option<String> = None;
            for attempt in 0..20 {
                let poll = client
                    .poll_provenance_query(&prov_handle)
                    .await
                    .unwrap_or_else(|e| {
                        panic!("poll_provenance_query attempt {attempt} on {version} failed: {e:?}")
                    });

                match poll {
                    ProvenancePollResult::Finished { events, .. } => {
                        if let Some(event) = events.first() {
                            uuid = Some(event.flow_file_uuid.clone());
                        }
                        break;
                    }
                    ProvenancePollResult::Running { percent } => {
                        eprintln!("  provenance attempt {attempt}: {percent}% …");
                        tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;
                    }
                }
            }
            uuid.unwrap_or_else(|| panic!("No provenance events found for processor on {version}"))
        };

        eprintln!("  flowfile_uuid = {flowfile_uuid}");

        // 4. Clean up the provenance query.
        client
            .delete_provenance_query(&prov_handle)
            .await
            .unwrap_or_else(|e| panic!("delete_provenance_query on {version} failed: {e:?}"));

        // 5. Submit a lineage query for the extracted flowfile UUID.
        let (query_id, cluster_node_id) = client
            .submit_lineage(&flowfile_uuid)
            .await
            .unwrap_or_else(|e| panic!("submit_lineage on {version} failed: {e:?}"));

        eprintln!("  lineage_query_id = {query_id}, cluster_node_id = {cluster_node_id:?}");

        // 6. Poll until finished (max 20 attempts × 500 ms = 10 s).
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

        // 7. Assert the query finished and returned at least one event.
        assert!(
            snapshot.finished,
            "snapshot.finished must be true on {version}"
        );
        assert!(
            !snapshot.events.is_empty(),
            "snapshot.events must not be empty on {version} (expected lineage chain)"
        );

        // 8. Clean up the lineage query.
        client
            .delete_lineage(&query_id, cluster_node_id.as_deref())
            .await
            .unwrap_or_else(|e| panic!("delete_lineage on {version} failed: {e:?}"));

        eprintln!("  cleaned up query {query_id}");
    }
}
