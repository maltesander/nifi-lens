//! Wiremock tests: Phase 3 browser client wrappers.

use nifi_lens::client::{
    ConnectionDetail, NifiClient, NodeKind, ProcessGroupDetail, ProcessorDetail, RecursiveSnapshot,
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
    let path = format!("tests/fixtures/browser/{name}");
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&contents).expect("parse fixture JSON")
}

#[tokio::test]
async fn browser_tree_parses_nested_pgs_into_flat_recursive_snapshot() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("recursive_tree.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/process-groups/root/status"))
        .and(query_param("recursive", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snap: RecursiveSnapshot = client.browser_tree().await.expect("ok");

    // One root PG + one root processor + one connection + one child PG +
    // its one processor + one input port + one output port = 7 nodes.
    assert_eq!(snap.nodes.len(), 7);

    // Index 0 must be the root PG with no parent.
    assert_eq!(snap.nodes[0].kind, NodeKind::ProcessGroup);
    assert_eq!(snap.nodes[0].id, "root");
    assert_eq!(snap.nodes[0].parent_idx, None);

    // Every non-root node must have a parent pointing upward.
    for (i, n) in snap.nodes.iter().enumerate().skip(1) {
        let p = n
            .parent_idx
            .unwrap_or_else(|| panic!("node {i} missing parent"));
        assert!(p < i, "arena parent index {p} not strictly less than {i}");
    }

    // Exactly one child PG and it contains three nodes.
    let child_pg_idx = snap
        .nodes
        .iter()
        .position(|n| n.kind == NodeKind::ProcessGroup && n.id == "ingest")
        .expect("ingest PG present");
    let child_count = snap
        .nodes
        .iter()
        .filter(|n| n.parent_idx == Some(child_pg_idx))
        .count();
    assert_eq!(
        child_count, 3,
        "ingest must contain proc + input + output port"
    );
}

#[tokio::test]
async fn browser_pg_detail_combines_pg_and_cs_list() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let fixture = load_fixture("process_group.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/process-groups/ingest"))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture["process_group_entity"].clone()),
        )
        .mount(&server)
        .await;
    Mock::given(method("GET"))
        .and(path(
            "/nifi-api/flow/process-groups/ingest/controller-services",
        ))
        .respond_with(
            ResponseTemplate::new(200).set_body_json(fixture["controller_services_entity"].clone()),
        )
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let detail: ProcessGroupDetail = client.browser_pg_detail("ingest").await.expect("ok");
    assert_eq!(detail.id, "ingest");
    assert_eq!(detail.name, "ingest");
    assert_eq!(detail.parent_group_id.as_deref(), Some("root"));
    assert_eq!(detail.running, 3);
    assert_eq!(detail.stopped, 0);
    assert_eq!(detail.invalid, 0);
    assert_eq!(detail.disabled, 0);
    assert_eq!(detail.active_threads, 1);
    assert_eq!(detail.flow_files_queued, 4);
    assert_eq!(detail.bytes_queued, 2048);
    assert_eq!(detail.queued_display, "4 / 2 KB");
    assert_eq!(detail.controller_services.len(), 2);
    assert_eq!(detail.controller_services[0].name, "http-pool");
    assert_eq!(detail.controller_services[0].state, "ENABLED");
    assert_eq!(
        detail.controller_services[0].type_short,
        "StandardRestrictedSSLContextService"
    );
    assert_eq!(detail.controller_services[1].name, "kafka-brokers");
    assert_eq!(detail.controller_services[1].state, "DISABLED");
}

#[tokio::test]
async fn browser_processor_detail_carries_properties_and_validation_errors() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("processor.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/processors/put-kafka-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let d: ProcessorDetail = client
        .browser_processor_detail("put-kafka-1")
        .await
        .expect("ok");
    assert_eq!(d.id, "put-kafka-1");
    assert_eq!(d.name, "PutKafka");
    assert!(d.type_name.contains("PublishKafka_2_6"));
    assert!(d.bundle.contains("nifi-kafka-2-6-nar"));
    assert_eq!(d.run_status, "RUNNING");
    assert_eq!(d.scheduling_strategy, "TIMER_DRIVEN");
    assert_eq!(d.scheduling_period, "1 sec");
    assert_eq!(d.concurrent_tasks, 2);
    assert_eq!(d.run_duration_ms, 25);
    assert_eq!(d.penalty_duration, "30 sec");
    assert_eq!(d.yield_duration, "1 sec");
    assert_eq!(d.bulletin_level, "WARN");
    assert_eq!(d.properties.len(), 6);
    assert_eq!(d.validation_errors.len(), 1);
    assert!(d.validation_errors[0].contains("Kafka Key"));
}

#[tokio::test]
async fn browser_connection_detail_carries_source_dest_relationships_thresholds() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("connection.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/connections/conn-enrich"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let d: ConnectionDetail = client
        .browser_connection_detail("conn-enrich")
        .await
        .expect("ok");
    assert_eq!(d.id, "conn-enrich");
    assert_eq!(d.name, "enrich → publish");
    assert_eq!(d.source_id, "proc-enrich");
    assert_eq!(d.source_name, "EnrichAttribute");
    assert_eq!(d.source_type, "PROCESSOR");
    assert_eq!(d.source_group_id, "ingest");
    assert_eq!(d.destination_id, "proc-publish");
    assert_eq!(d.destination_name, "PublishKafka");
    assert_eq!(d.destination_type, "PROCESSOR");
    assert_eq!(d.destination_group_id, "publish");
    assert_eq!(d.selected_relationships, vec!["success".to_string()]);
    assert_eq!(
        d.available_relationships,
        vec![
            "success".to_string(),
            "failure".to_string(),
            "retry".to_string(),
        ]
    );
    assert_eq!(d.back_pressure_object_threshold, 10000);
    assert_eq!(d.back_pressure_data_size_threshold, "1 GB");
    assert_eq!(d.flow_file_expiration, "0 sec");
    assert_eq!(d.load_balance_strategy, "DO_NOT_LOAD_BALANCE");
    assert_eq!(d.fill_percent, 55);
    assert_eq!(d.flow_files_queued, 5500);
    assert_eq!(d.bytes_queued, 52_428_800);
    assert_eq!(d.queued_display, "5,500 / 50 MB");
}
