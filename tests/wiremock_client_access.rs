//! Wiremock tests: access-policy client wrappers.

use nifi_lens::client::access::fetch_axis;
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::view::browser::state::access_modal::{Axis, AxisOutcome};
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
