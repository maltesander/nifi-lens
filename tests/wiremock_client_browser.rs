//! Wiremock tests: Phase 3 browser client wrappers.

use nifi_lens::client::{NifiClient, NodeKind, RecursiveSnapshot};
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
