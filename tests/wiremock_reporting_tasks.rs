//! Wiremock tests: reporting-tasks client wrapper.

use nifi_lens::client::{NifiClient, ReportingTaskState, ValidationStatus};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use wiremock::matchers::{method, path};
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
    let path = format!("tests/fixtures/reporting_tasks/{name}");
    let contents = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path}: {e}"));
    serde_json::from_str(&contents).expect("parse fixture JSON")
}

#[tokio::test]
async fn happy_path_parses_full_response_and_masks_sensitive() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("full.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/reporting-tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snapshot = client
        .reporting_tasks_snapshot()
        .await
        .expect("snapshot ok");

    assert_eq!(snapshot.tasks.len(), 5);

    let counts = snapshot.counts();
    // 2 RUNNING+VALID, 1 STOPPED, 1 DISABLED, 1 RUNNING+INVALID
    assert_eq!(counts.total, 5);
    assert_eq!(counts.running, 2, "RUNNING + VALID only");
    assert_eq!(counts.stopped, 2, "stopped + disabled");
    assert_eq!(counts.invalid, 1);

    // Sensitive property must be masked to None even though the wire JSON
    // had a non-empty value.
    let prom = snapshot
        .tasks
        .iter()
        .find(|t| t.name == "Prometheus Exporter")
        .expect("task-1 present");
    assert_eq!(
        prom.properties.get("SSL Context Service"),
        Some(&None),
        "sensitive descriptor must mask its value"
    );
    assert_eq!(
        prom.properties.get("Instance ID"),
        Some(&Some("prod-1".to_string()))
    );
    assert_eq!(prom.state, ReportingTaskState::Running);
    assert_eq!(prom.validation_status, ValidationStatus::Valid);
    assert_eq!(prom.scheduling_period, "30s");

    // Validation errors propagate verbatim.
    let disk = snapshot
        .tasks
        .iter()
        .find(|t| t.name == "Disk Monitor")
        .expect("task-4 present");
    assert_eq!(disk.validation_status, ValidationStatus::Invalid);
    assert_eq!(disk.validation_errors.len(), 2);
    assert!(
        disk.validation_errors
            .iter()
            .any(|e| e.contains("Threshold"))
    );
}

#[tokio::test]
async fn empty_response_yields_zero_tasks() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let body = load_fixture("empty.json");
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/reporting-tasks"))
        .respond_with(ResponseTemplate::new(200).set_body_json(body))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let snapshot = client
        .reporting_tasks_snapshot()
        .await
        .expect("snapshot ok");

    assert_eq!(snapshot.tasks.len(), 0);
    assert_eq!(snapshot.counts().total, 0);
}

#[tokio::test]
async fn server_error_propagates_as_typed_error() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/reporting-tasks"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let result = client.reporting_tasks_snapshot().await;
    assert!(result.is_err(), "5xx must surface as Err");
}
