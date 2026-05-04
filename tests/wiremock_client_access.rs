//! Wiremock tests: access-policy client wrappers.

use nifi_lens::client::access::{
    AccessFetchResult, fetch_axis, fetch_component_access, fetch_identity_grants,
};
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::view::browser::state::access_modal::{Axis, AxisOutcome};
use nifi_lens::view::browser::state::identity_modal::{IdentityKind, ResourceBucket};
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

#[tokio::test]
async fn fetch_axis_returns_direct_when_response_resource_matches() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/policies/read/processors/abc-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "policy-1",
            "component": {
                "id": "policy-1",
                "resource": "/processors/abc-123",
                "action": "read",
                "users": [
                    { "id": "u1", "component": { "id": "u1", "identity": "alice@corp" } }
                ],
                "userGroups": [
                    { "id": "g1", "component": { "id": "g1", "identity": "ops-team" } }
                ]
            }
        })))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewComponent, NodeKind::Processor, "abc-123")
        .await
        .expect("call must not error");

    match outcome {
        AxisOutcome::Direct { users, groups } => {
            assert_eq!(users.len(), 1);
            assert_eq!(users[0].identity, "alice@corp");
            assert_eq!(groups.len(), 1);
            assert_eq!(groups[0].identity, "ops-team");
            // member_count is not available from the /policies endpoint;
            // the response uses TenantEntity (TenantDto component) for both
            // users and user_groups — TenantDto has no users/members field.
            assert_eq!(groups[0].member_count, None);
        }
        other => panic!("expected Direct, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_axis_flags_inherited_when_response_resource_differs() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/policies/read/processors/abc-123"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "component": {
                "resource": "/process-groups/parent-pg",
                "action": "read",
                "users": [{ "id": "u1", "component": { "id": "u1", "identity": "alice@corp" } }],
                "userGroups": []
            }
        })))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewComponent, NodeKind::Processor, "abc-123")
        .await
        .unwrap();

    match outcome {
        AxisOutcome::Inherited { source, users, .. } => {
            assert_eq!(source, "/process-groups/parent-pg");
            assert_eq!(users[0].identity, "alice@corp");
        }
        other => panic!("expected Inherited, got {other:?}"),
    }
}

#[tokio::test]
async fn fetch_axis_returns_none_on_404() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/policies/read/processors/abc-123"))
        .respond_with(ResponseTemplate::new(404))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewComponent, NodeKind::Processor, "abc-123")
        .await
        .unwrap();
    assert_eq!(outcome, AxisOutcome::None);
}

#[tokio::test]
async fn fetch_axis_returns_forbidden_on_403() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/policies/read/processors/abc-123"))
        .respond_with(ResponseTemplate::new(403))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewComponent, NodeKind::Processor, "abc-123")
        .await
        .unwrap();
    assert_eq!(outcome, AxisOutcome::Forbidden);
}

#[tokio::test]
async fn fetch_axis_returns_error_on_5xx() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    Mock::given(method("GET"))
        .and(path("/nifi-api/policies/read/processors/abc-123"))
        .respond_with(ResponseTemplate::new(503))
        .mount(&server)
        .await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewComponent, NodeKind::Processor, "abc-123")
        .await
        .unwrap();
    assert!(matches!(outcome, AxisOutcome::Error(_)));
}

#[tokio::test]
async fn fetch_axis_returns_not_applicable_without_request() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;

    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let outcome = fetch_axis(&client, Axis::ViewData, NodeKind::ControllerService, "cs-1")
        .await
        .unwrap();
    assert_eq!(outcome, AxisOutcome::NotApplicable);
    // Only the login + about + cluster-summary calls from NifiClient::connect
    // should have fired; the fetcher must not have issued any policy request.
    let reqs = server.received_requests().await.unwrap();
    let connect_paths = ["/access/token", "/flow/about", "/flow/cluster/summary"];
    assert!(
        reqs.iter()
            .all(|r| connect_paths.iter().any(|p| r.url.path().contains(p))),
        "unexpected policy request fired: {reqs:?}"
    );
}

#[tokio::test]
async fn fetch_component_access_fans_out_five_calls_for_processor() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    for path_str in [
        "/nifi-api/policies/read/processors/abc-123",
        "/nifi-api/policies/write/processors/abc-123",
        "/nifi-api/policies/read/data/processors/abc-123",
        "/nifi-api/policies/write/operate/processors/abc-123",
        "/nifi-api/policies/write/policies/processors/abc-123",
    ] {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "component": {
                    "resource": path_str.trim_start_matches("/nifi-api/policies/read").trim_start_matches("/nifi-api/policies/write"),
                    "users": [],
                    "userGroups": []
                }
            })))
            .mount(&server)
            .await;
    }
    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let result: AccessFetchResult =
        fetch_component_access(&client, NodeKind::Processor, "abc-123").await;
    assert_eq!(result.outcomes.len(), 5);
    // Filter out connect-bootstrap calls; the 5 fetcher calls must all have fired.
    let policy_calls: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().starts_with("/nifi-api/policies/"))
        .collect();
    assert_eq!(policy_calls.len(), 5);
}

#[tokio::test]
async fn fetch_component_access_skips_inapplicable_axes_for_cs() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    for path_str in [
        "/nifi-api/policies/read/controller-services/cs-1",
        "/nifi-api/policies/write/controller-services/cs-1",
        "/nifi-api/policies/write/policies/controller-services/cs-1",
    ] {
        Mock::given(method("GET"))
            .and(path(path_str))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "component": { "resource": "/controller-services/cs-1", "users": [], "userGroups": [] }
            })))
            .mount(&server)
            .await;
    }
    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let _ = fetch_component_access(&client, NodeKind::ControllerService, "cs-1").await;
    // ViewData and Operate axes do not apply → only 3 policy requests.
    let policy_calls: Vec<_> = server
        .received_requests()
        .await
        .unwrap()
        .into_iter()
        .filter(|r| r.url.path().starts_with("/nifi-api/policies/"))
        .collect();
    assert_eq!(policy_calls.len(), 3);
}

// ── unit tests: observe_audit_state (pure function, no wiremock) ──────────────

use nifi_lens::client::access::observe_audit_state;
use nifi_lens::cluster::AccessAuditState;

#[test]
fn observe_audit_state_promotes_unknown_to_supported_on_200() {
    assert_eq!(
        observe_audit_state(
            AccessAuditState::Unknown,
            &AxisOutcome::Direct {
                users: vec![],
                groups: vec![],
            }
        ),
        AccessAuditState::Supported,
    );
}

#[test]
fn observe_audit_state_marks_unsupported_on_unauthorizer_409() {
    let outcome =
        AxisOutcome::Error("Status { status: 409, body: \"No authorizer configured\" }".into());
    assert_eq!(
        observe_audit_state(AccessAuditState::Unknown, &outcome),
        AccessAuditState::Unsupported,
    );
}

#[test]
fn observe_audit_state_does_not_demote_supported_on_403() {
    // Per-axis 403 means caller lacks read on /policies/{...}; it is
    // NOT a global auth-disabled signal.
    let outcome = AxisOutcome::Forbidden;
    assert_eq!(
        observe_audit_state(AccessAuditState::Supported, &outcome),
        AccessAuditState::Supported,
    );
}

#[test]
fn observe_audit_state_unknown_to_unsupported_on_blanket_403() {
    // From an unsecured (HTTP) NiFi the very first call returns
    // Forbidden — represented as Error with the canonical body string.
    let outcome = AxisOutcome::Error(
        "Status { status: 403, body: \"Access is denied. Contact the system administrator.\" }"
            .into(),
    );
    assert_eq!(
        observe_audit_state(AccessAuditState::Unknown, &outcome),
        AccessAuditState::Unsupported,
    );
}

#[tokio::test]
async fn fetch_identity_grants_groups_into_buckets() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    Mock::given(method("GET"))
        .and(path("/nifi-api/tenants/users/u1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "u1",
            "component": {
                "id": "u1",
                "identity": "alice@corp",
                "userGroups": [{ "id": "g1", "component": {"id":"g1","identity":"ops-team"} }],
                "accessPolicies": [
                    { "component": { "action": "read", "resource": "/processors/abc" } },
                    { "component": { "action": "write", "resource": "/process-groups/orders" } },
                    { "component": { "action": "read", "resource": "/flow" } }
                ]
            }
        })))
        .mount(&server)
        .await;
    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let result = fetch_identity_grants(&client, IdentityKind::User, "u1")
        .await
        .unwrap();
    assert_eq!(result.identity, "alice@corp");
    assert_eq!(result.group_memberships, vec!["ops-team".to_string()]);
    assert_eq!(result.grants.len(), 3);
    let buckets: Vec<_> = result.grants.iter().map(|g| g.bucket).collect();
    assert!(buckets.contains(&ResourceBucket::Processors));
    assert!(buckets.contains(&ResourceBucket::ProcessGroups));
    assert!(buckets.contains(&ResourceBucket::Global));
}

#[tokio::test]
async fn fetch_identity_grants_for_group() {
    let server = MockServer::start().await;
    stub_login_and_about(&server).await;
    Mock::given(method("GET"))
        .and(path("/nifi-api/tenants/user-groups/g1"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "g1",
            "component": {
                "id": "g1",
                "identity": "ops-team",
                "users": [{"id":"u1"}, {"id":"u2"}],
                "accessPolicies": [
                    { "component": { "action": "write", "resource": "/process-groups/orders" } }
                ]
            }
        })))
        .mount(&server)
        .await;
    let client = NifiClient::connect(&ctx(server.uri())).await.unwrap();
    let result = fetch_identity_grants(&client, IdentityKind::UserGroup, "g1")
        .await
        .unwrap();
    assert_eq!(result.identity, "ops-team");
    assert_eq!(result.grants.len(), 1);
}
