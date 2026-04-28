//! Wiremock tests: browser client wrappers.

use nifi_lens::client::{
    ConnectionDetail, ControllerServiceDetail, NifiClient, NodeKind, ProcessGroupDetail,
    ProcessorDetail,
};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
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
        auth: ResolvedAuth::Password {
            username: "admin".into(),
            password: "anything".into(),
        },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: true,
        ca_cert_path: None,
        proxied_entities_chain: None,
        proxy_url: None,
        http_proxy_url: None,
        https_proxy_url: None,
    }
}

fn load_fixture(name: &str) -> serde_json::Value {
    let path = format!("tests/fixtures/browser/{name}");
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&contents).expect("parse fixture JSON")
}

#[tokio::test]
async fn root_pg_status_parses_nested_pgs_into_flat_node_list() {
    // Task 6 of the central-cluster-store refactor moved the flat
    // arena build-out off `browser_tree` and onto `root_pg_status`:
    // the recursive walker fills `RootPgStatusSnapshot.nodes` so the
    // Browser reducer can rebuild its arena straight from the cluster
    // snapshot. This test pins the shape of `nodes` returned by the
    // same recursive-status fixture the old `browser_tree` test used.
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
    let snap = client.root_pg_status().await.expect("ok");

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

#[tokio::test]
async fn browser_cs_detail_carries_state_and_properties() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("controller_service.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/controller-services/cs-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let d: ControllerServiceDetail = client.browser_cs_detail("cs-1").await.expect("ok");
    assert_eq!(d.id, "cs-1");
    assert_eq!(d.name, "http-pool");
    assert!(d.type_name.contains("StandardRestrictedSSLContextService"));
    assert!(d.bundle.contains("nifi-ssl-context-service-nar"));
    assert_eq!(d.state, "ENABLED");
    assert_eq!(d.parent_group_id.as_deref(), Some("ingest"));
    assert_eq!(d.properties.len(), 2);
    assert!(d.validation_errors.is_empty());
    assert_eq!(d.bulletin_level, "WARN");
}

#[tokio::test]
async fn root_pg_status_error_is_mapped_to_typed_nifilens_error() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/process-groups/root/status"))
        .respond_with(ResponseTemplate::new(500).set_body_string("boom"))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let err = client.root_pg_status().await.unwrap_err();
    let msg = format!("{err}");
    assert!(
        msg.to_lowercase().contains("process-group")
            || msg.to_lowercase().contains("process group"),
        "expected ProcessGroupStatusFailed, got: {msg}"
    );
}

#[tokio::test]
async fn controller_services_snapshot_groups_members_by_parent_group_id() {
    // Task 6: the old `browser_tree` test
    // `browser_tree_includes_controller_services_under_owning_pgs`
    // asserted that CS rows were arena-attached under their owning
    // PG. That splicing now happens in the Browser reducer via
    // `rebuild_arena_from_cluster`. At the client level the contract
    // we need is simpler: every CS member must carry its
    // `parent_group_id` so the reducer has something to splice on.
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let cs_body = load_fixture("root_cs_list.json");
    Mock::given(method("GET"))
        .and(path(
            "/nifi-api/flow/process-groups/root/controller-services",
        ))
        .and(query_param("includeDescendantGroups", "true"))
        .respond_with(ResponseTemplate::new(200).set_body_json(cs_body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let cs_snap = client.controller_services_snapshot().await.expect("ok");

    assert_eq!(cs_snap.members.len(), 2);
    for m in &cs_snap.members {
        assert!(
            !m.parent_group_id.is_empty(),
            "CS member {} must carry a parent_group_id",
            m.id
        );
    }
}

#[tokio::test]
async fn browser_cs_detail_parses_extended_fields() {
    use nifi_lens::client::ReferencingKind;
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    let body = load_fixture("controller_service.json");
    // Note: NiFi's `GET /controller-services/{id}` returns
    // `referencingComponents` by default; the generated client's only
    // query parameter is `uiOnly`, which we intentionally do not pass.
    Mock::given(method("GET"))
        .and(path("/nifi-api/controller-services/cs-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let d: ControllerServiceDetail = client.browser_cs_detail("cs-1").await.expect("ok");

    assert_eq!(
        d.comments,
        "Shared SSL context for all HTTP ingestion processors."
    );
    assert!(d.restricted);
    assert!(!d.deprecated);
    assert!(!d.persists_state);
    assert_eq!(d.referencing_components.len(), 2);

    let a = &d.referencing_components[0];
    assert_eq!(a.id, "proc-a");
    assert_eq!(a.name, "InvokeHTTP");
    assert!(matches!(a.kind, ReferencingKind::Processor));
    assert_eq!(a.state, "RUNNING");
    assert_eq!(a.active_thread_count, 2);
    assert_eq!(a.group_id, "ingest");

    let b = &d.referencing_components[1];
    assert!(matches!(b.kind, ReferencingKind::ControllerService));
}

#[tokio::test]
async fn browser_pg_detail_unauthorized_maps_to_typed_error() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/process-groups/locked"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let err = client.browser_pg_detail("locked").await.unwrap_err();
    let msg = format!("{err}");
    // `classify_or_fallback` downgrades library auth errors to
    // `NifiUnauthorized`, so accept either message shape.
    assert!(
        msg.to_lowercase().contains("unauthorized")
            || msg.contains("process-group")
            || msg.contains("rejected"),
        "expected unauthorized/PG detail error, got: {msg}"
    );
}

#[tokio::test]
async fn browser_port_detail_parses_input_port() {
    use nifi_lens::client::{PortDetail, PortKind};
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    let body = load_fixture("input_port.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/input-ports/in-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let d: PortDetail = client
        .browser_port_detail("in-1", PortKind::Input)
        .await
        .unwrap();

    assert_eq!(d.id, "in-1");
    assert_eq!(d.name, "external-ingest");
    assert_eq!(d.kind, PortKind::Input);
    assert_eq!(d.state, "RUNNING");
    assert_eq!(d.comments, "accepts from edge agents");
    assert_eq!(d.concurrent_tasks, 3);
}

// Task 6 removed `browser_tree_cs_fetch_failure_is_non_fatal`: the
// "CS-only failure is non-fatal" contract now lives in
// `EndpointState::Failed.last_ok` semantics in `ClusterSnapshot`. The
// two endpoints are fetched independently by the cluster store; the
// Browser reducer handles a `Loading`/`Failed` CS slot by simply not
// splicing any CS members into the arena (tested at the reducer level
// in `src/view/browser/state.rs`).

#[tokio::test]
async fn version_information_returns_summary_on_versioned_pg() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/versions/process-groups/pg-1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "versionControlInformation": {
                "groupId": "pg-1",
                "registryId": "reg-1",
                "registryName": "ops-registry",
                "bucketId": "buck-1",
                "bucketName": "ops",
                "flowId": "flow-1",
                "flowName": "ingest",
                "version": "3",
                "branch": "main",
                "state": "STALE",
                "stateExplanation": "A newer version exists"
            }
        })))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let summary = client.version_information("pg-1").await.unwrap();
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
    assert_eq!(summary.state, VersionControlInformationDtoState::Stale);
    assert_eq!(summary.registry_name.as_deref(), Some("ops-registry"));
    assert_eq!(summary.bucket_name.as_deref(), Some("ops"));
    assert_eq!(summary.branch.as_deref(), Some("main"));
    assert_eq!(summary.flow_name.as_deref(), Some("ingest"));
    assert_eq!(summary.version.as_deref(), Some("3"));
    assert_eq!(
        summary.state_explanation.as_deref(),
        Some("A newer version exists")
    );
}

#[tokio::test]
async fn version_information_returns_unversioned_for_null_payload() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/versions/process-groups/pg-2"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "versionControlInformation": null
        })))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let res = client.version_information_optional("pg-2").await.unwrap();
    assert!(res.is_none());
}

#[tokio::test]
async fn local_modifications_groups_by_component() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/process-groups/pg-1/local-modifications"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "componentDifferences": [
                {
                    "componentId": "proc-a",
                    "componentName": "UpdateRecord",
                    "componentType": "Processor",
                    "processGroupId": "pg-1",
                    "differences": [
                        {"differenceType": "PROPERTY_CHANGED",
                         "difference": "Record Reader changed",
                         "environmental": false},
                        {"differenceType": "BUNDLE_CHANGED",
                         "difference": "Bundle upgraded",
                         "environmental": true}
                    ]
                }
            ]
        })))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let grouped = client.local_modifications("pg-1").await.unwrap();
    assert_eq!(grouped.sections.len(), 1);
    let section = &grouped.sections[0];
    assert_eq!(section.component_id, "proc-a");
    assert_eq!(section.display_label, "UpdateRecord");
    assert_eq!(section.component_type, "Processor");
    assert_eq!(section.differences.len(), 2);
    assert_eq!(section.differences[0].kind, "PROPERTY_CHANGED");
    assert!(!section.differences[0].environmental);
    assert!(section.differences[1].environmental);
}
