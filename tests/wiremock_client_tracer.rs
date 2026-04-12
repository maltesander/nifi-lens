//! Wiremock tests: Phase 4 tracer client wrappers.

use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedContext, VersionStrategy};
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
        username: "admin".into(),
        password: "anything".into(),
        version_strategy: VersionStrategy::Closest,
        insecure_tls: true,
        ca_cert_path: None,
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
