//! Wiremock tests: Phase 4 tracer client wrappers.

use nifi_lens::client::NifiClient;
use nifi_lens::client::tracer::{ContentRender, ContentSide};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::error::NifiLensError;
use wiremock::matchers::{method, path, query_param};
use wiremock::{Mock, MockServer, ResponseTemplate};

async fn stub_login_and_about(server: &MockServer) {
    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stub-jwt-token"))
        .mount(server)
        .await;
    let about = serde_json::json!({
        "about": { "version": "2.8.0", "title": "NiFi", "uri": server.uri() }
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
    let path = format!("tests/fixtures/tracer/{name}");
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&contents).expect("parse fixture JSON")
}

#[tokio::test]
async fn latest_events_returns_populated_list_in_order() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("latest_events.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/latest/proc-123"))
        .and(query_param("limit", "20"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snap = client.latest_events("proc-123", 20).await.unwrap();

    assert_eq!(snap.component_id, "proc-123");
    assert_eq!(snap.component_label, "PutDatabaseRecord · root-persist");
    assert_eq!(snap.events.len(), 2);
    assert_eq!(snap.events[0].event_id, 42);
    assert_eq!(snap.events[0].event_type, "DROP");
    assert_eq!(snap.events[0].relationship.as_deref(), Some("failure"));
    assert_eq!(snap.events[1].event_id, 41);
}

#[tokio::test]
async fn latest_events_empty_component_is_ok_empty() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = serde_json::json!({
        "latestProvenanceEvents": {
            "componentId": "proc-empty",
            "provenanceEvents": []
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/latest/proc-empty"))
        .and(query_param("limit", "10"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snap = client.latest_events("proc-empty", 10).await.unwrap();

    assert!(snap.events.is_empty());
    assert_eq!(snap.component_id, "proc-empty");
    // When no events, label falls back to component_id.
    assert_eq!(snap.component_label, "proc-empty");
}

#[tokio::test]
async fn latest_events_not_found_maps_to_typed_error() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/latest/no-such"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let err = client.latest_events("no-such", 10).await.unwrap_err();

    assert!(
        matches!(
            &err,
            NifiLensError::LatestProvenanceEventsFailed { component_id, .. }
            if component_id == "no-such"
        ),
        "expected LatestProvenanceEventsFailed, got: {err}"
    );
}

#[tokio::test]
async fn submit_lineage_returns_query_id() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("lineage_running.json");
    Mock::given(method("POST"))
        .and(path("/nifi-api/provenance/lineage"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let (query_id, cluster_node_id) = client
        .submit_lineage("7a2e8b9c-1234-4abc-9def-0123456789ab")
        .await
        .unwrap();

    assert_eq!(query_id, "lineage-query-0001");
    // Standalone fixture has no cluster_node_id in the response.
    assert!(cluster_node_id.is_none());
}

#[tokio::test]
async fn poll_lineage_running_returns_percent() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("lineage_running.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance/lineage/lineage-query-0001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let poll = client
        .poll_lineage("lineage-query-0001", None)
        .await
        .unwrap();

    assert!(
        matches!(poll, nifi_lens::client::tracer::LineagePoll::Running { percent } if percent == 40),
        "expected Running(40), got: {poll:?}"
    );
}

#[tokio::test]
async fn poll_lineage_finished_returns_snapshot_in_chronological_order() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("lineage_finished.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance/lineage/lineage-query-0001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let poll = client
        .poll_lineage("lineage-query-0001", None)
        .await
        .unwrap();

    let snapshot = match poll {
        nifi_lens::client::tracer::LineagePoll::Finished(s) => s,
        other => panic!("expected Finished, got: {other:?}"),
    };

    assert!(snapshot.finished);
    assert_eq!(snapshot.percent_completed, 100);
    assert_eq!(snapshot.events.len(), 3);
    assert_eq!(snapshot.events[0].event_type, "CREATE");
    assert_eq!(snapshot.events[1].event_type, "ATTRIBUTES_MODIFIED");
    assert_eq!(snapshot.events[2].event_type, "DROP");
    assert_eq!(snapshot.events[0].component_type, "GenerateFlowFile");
}

#[tokio::test]
async fn delete_lineage_returns_ok_on_200() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("lineage_finished.json");
    Mock::given(method("DELETE"))
        .and(path("/nifi-api/provenance/lineage/lineage-query-0001"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    client
        .delete_lineage("lineage-query-0001", None)
        .await
        .unwrap();
}

#[tokio::test]
async fn get_provenance_event_populates_detail_and_triples() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    let body = load_fixture("provenance_event.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/42"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;
    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let detail = client.get_provenance_event(42).await.unwrap();
    assert_eq!(detail.summary.event_id, 42);
    assert_eq!(detail.summary.event_type, "DROP");
    assert!(detail.input_available);
    assert!(!detail.output_available);
    assert_eq!(detail.attributes.len(), 3);
    let filename = &detail.attributes[0];
    assert_eq!(filename.key, "filename");
    assert!(!filename.is_changed());
    let db_target = &detail.attributes[1];
    assert_eq!(db_target.key, "db.target");
    assert!(db_target.is_changed());
    assert_eq!(db_target.previous.as_deref(), Some("prod-replica-1"));
    assert_eq!(db_target.current.as_deref(), Some("prod-replica-2"));
}

#[tokio::test]
async fn provenance_content_input_text_json_is_pretty_printed() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/42/content/input"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(br#"{"a":1,"b":2}"#.to_vec())
                .insert_header("content-type", "application/json"),
        )
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snap = client
        .provenance_content(42, ContentSide::Input, None)
        .await
        .unwrap();

    assert_eq!(snap.event_id, 42);
    assert_eq!(snap.side, ContentSide::Input);
    assert_eq!(snap.bytes_fetched, br#"{"a":1,"b":2}"#.len());
    match snap.render {
        ContentRender::Text { pretty } => {
            assert!(pretty.contains('\n'), "expected newlines in pretty output");
            assert!(pretty.contains("\"a\": 1"));
            assert!(pretty.contains("\"b\": 2"));
        }
        other => panic!("expected ContentRender::Text, got: {other:?}"),
    }
}

#[tokio::test]
async fn provenance_content_input_non_utf8_is_hex_dump() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let payload: Vec<u8> = vec![0xff, 0xfe, 0xfd, 0xfc, 0x00, 0x01, 0x02, 0x03];
    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/42/content/input"))
        .respond_with(
            ResponseTemplate::new(200)
                .set_body_bytes(payload.clone())
                .insert_header("content-type", "application/octet-stream"),
        )
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snap = client
        .provenance_content(42, ContentSide::Input, None)
        .await
        .unwrap();

    match snap.render {
        ContentRender::Hex { first_4k } => {
            assert!(
                first_4k.contains("ff fe fd fc"),
                "hex dump should contain 'ff fe fd fc', got: {first_4k}"
            );
        }
        other => panic!("expected ContentRender::Hex, got: {other:?}"),
    }
}

#[tokio::test]
async fn provenance_content_output_not_found_errors() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/provenance-events/42/content/output"))
        .respond_with(ResponseTemplate::new(404).set_body_string("not found"))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let err = client
        .provenance_content(42, ContentSide::Output, None)
        .await
        .unwrap_err();

    assert!(
        matches!(
            &err,
            NifiLensError::ProvenanceContentFetchFailed { event_id: 42, side, .. }
            if *side == "output"
        ),
        "expected ProvenanceContentFetchFailed with event_id=42 and side=output, got: {err}"
    );
}
