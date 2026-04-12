//! Wiremock tests: Phase 5 health client wrappers.

use nifi_lens::client::{
    ConnectionStatusRow, FullPgStatusSnapshot, NifiClient, ProcessorStatusRow, SystemDiagSnapshot,
};
use nifi_lens::config::{ResolvedContext, VersionStrategy};
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn stub_login_and_about(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stub-jwt-token"))
        .mount(server)
        .await;
    let about = serde_json::json!({
        "about": {
            "version": "2.8.0",
            "title": "NiFi",
            "uri": server.uri(),
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/about"))
        .respond_with(ResponseTemplate::new(200).set_body_json(about))
        .mount(server)
        .await;
}

fn ctx(url: String) -> ResolvedContext {
    ResolvedContext {
        name: "wiremock".into(),
        url,
        username: "admin".into(),
        password: "anything".into(),
        version_strategy: VersionStrategy::Closest,
        insecure_tls: true,
        ca_cert_path: None,
    }
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!("tests/fixtures/health/{name}");
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&contents).expect("parse fixture JSON")
}

#[tokio::test]
async fn root_pg_status_full_extracts_connections_and_processors() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("pg_status_full.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/process-groups/root/status"))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snapshot: FullPgStatusSnapshot = client.root_pg_status_full().await.expect("ok");

    // Both root and child-pg have one connection each.
    assert_eq!(snapshot.connections.len(), 2, "expected 2 connections");

    // Both root and child-pg have one processor each.
    assert_eq!(snapshot.processors.len(), 2, "expected 2 processors");

    // --- Connection assertions ---
    let conn_a: &ConnectionStatusRow = snapshot
        .connections
        .iter()
        .find(|c| c.id == "conn-a")
        .expect("conn-a present");
    assert_eq!(conn_a.source_name, "GenerateFlowFile");
    assert_eq!(conn_a.destination_name, "PutFile");
    assert_eq!(conn_a.percent_use_count, 75);
    assert_eq!(conn_a.percent_use_bytes, 60);
    assert_eq!(conn_a.flow_files_queued, 7500);
    assert_eq!(conn_a.bytes_queued, 10_485_760);
    assert_eq!(conn_a.bytes_in, 2_097_152);
    assert_eq!(conn_a.bytes_out, 1_048_576);
    // predictions: min(300000, 120000) == 120000
    assert_eq!(
        conn_a.predicted_millis_until_backpressure,
        Some(120_000),
        "predicted min of bytes/count backpressure"
    );

    let conn_b: &ConnectionStatusRow = snapshot
        .connections
        .iter()
        .find(|c| c.id == "conn-b")
        .expect("conn-b present");
    assert_eq!(conn_b.percent_use_count, 5);
    assert_eq!(conn_b.bytes_in, 102_400);
    assert_eq!(conn_b.bytes_out, 102_400);
    // No predictions block in fixture.
    assert_eq!(conn_b.predicted_millis_until_backpressure, None);

    // --- Processor assertions ---
    let proc_a: &ProcessorStatusRow = snapshot
        .processors
        .iter()
        .find(|p| p.id == "proc-a")
        .expect("proc-a present");
    assert_eq!(proc_a.name, "GenerateFlowFile");
    assert_eq!(proc_a.active_thread_count, 2);
    assert_eq!(proc_a.run_status, "Running");
    assert_eq!(proc_a.tasks_duration_nanos, 500_000_000);
    // Root PG path should be "NiFi Flow".
    assert_eq!(proc_a.group_path, "NiFi Flow");

    let proc_b: &ProcessorStatusRow = snapshot
        .processors
        .iter()
        .find(|p| p.id == "proc-b")
        .expect("proc-b present");
    assert_eq!(proc_b.name, "RouteOnAttribute");
    assert_eq!(proc_b.active_thread_count, 5);
    assert_eq!(proc_b.tasks_duration_nanos, 1_200_000_000);
    // Child PG path should be "NiFi Flow / child-pipeline".
    assert_eq!(proc_b.group_path, "NiFi Flow / child-pipeline");
}

#[tokio::test]
async fn system_diagnostics_nodewise_returns_per_node_breakdown() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("system_diagnostics_nodewise.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/system-diagnostics"))
        .and(query_param("nodewise", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snapshot: SystemDiagSnapshot = client.system_diagnostics(true).await.expect("ok");

    // Aggregate: content repo at 78%, flowfile repo at 31%, provenance at 91%.
    assert_eq!(snapshot.aggregate.content_repos.len(), 1);
    assert_eq!(snapshot.aggregate.content_repos[0].utilization_percent, 78);
    assert!(
        snapshot.aggregate.flowfile_repo.is_some(),
        "flowfile repo present"
    );
    assert_eq!(
        snapshot
            .aggregate
            .flowfile_repo
            .as_ref()
            .unwrap()
            .utilization_percent,
        31
    );
    assert_eq!(snapshot.aggregate.provenance_repos.len(), 1);
    assert_eq!(
        snapshot.aggregate.provenance_repos[0].utilization_percent,
        91
    );

    // Two nodes.
    assert_eq!(snapshot.nodes.len(), 2, "expected 2 nodes");

    // Node 1: 75% heap (805306368 / 4294967296), 142 GC collections.
    let node1 = snapshot
        .nodes
        .iter()
        .find(|n| n.address == "node1.nifi.local:8443")
        .expect("node-1 present");
    assert_eq!(node1.heap_used_bytes, 805_306_368);
    assert_eq!(node1.heap_max_bytes, 4_294_967_296);
    assert!(!node1.gc.is_empty(), "node-1 GC populated");
    assert_eq!(node1.gc[0].collection_count, 142);
    assert_eq!(node1.total_threads, 88);

    // Node 2: 90% heap (1932735284 / 4294967296), 312 GC collections.
    let node2 = snapshot
        .nodes
        .iter()
        .find(|n| n.address == "node2.nifi.local:8443")
        .expect("node-2 present");
    assert_eq!(node2.heap_used_bytes, 1_932_735_284);
    assert!(!node2.gc.is_empty(), "node-2 GC populated");
    assert_eq!(node2.gc[0].collection_count, 312);
    assert_eq!(node2.total_threads, 132);
}

#[tokio::test]
async fn system_diagnostics_error_maps_to_typed_error() {
    use nifi_lens::error::NifiLensError;

    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/system-diagnostics"))
        .respond_with(ResponseTemplate::new(500).set_body_string("internal server error"))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let result = client.system_diagnostics(true).await;

    assert!(result.is_err(), "expected error on 500");
    let err = result.unwrap_err();
    assert!(
        matches!(err, NifiLensError::SystemDiagnosticsFailed { .. }),
        "expected SystemDiagnosticsFailed, got: {err:?}"
    );
}
