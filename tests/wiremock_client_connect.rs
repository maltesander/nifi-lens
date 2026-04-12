//! Wiremock test: happy path for NifiClient::connect.

use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn connect_happy_path_detects_version() {
    let server = MockServer::start().await;

    // Login returns a stub JWT (raw string body, not JSON).
    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stub-jwt-token"))
        .mount(&server)
        .await;

    // /flow/about returns a minimal AboutEntity. The outer `about` key is the
    // envelope the library deserializes; the version string must be parseable
    // as semver for NifiClient::connect to succeed.
    let about_body = serde_json::json!({
        "about": {
            "version": "2.9.0",
            "title": "NiFi",
            "uri": server.uri(),
        }
    });
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/about"))
        .respond_with(ResponseTemplate::new(200).set_body_json(about_body))
        .mount(&server)
        .await;

    let ctx = ResolvedContext {
        name: "wiremock".into(),
        url: server.uri(),
        auth: ResolvedAuth::Password {
            username: "admin".into(),
            password: "anything".into(),
        },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: true, // wiremock is plaintext HTTP
        ca_cert_path: None,
        proxied_entities_chain: None,
    };

    let client = NifiClient::connect(&ctx)
        .await
        .expect("connect should succeed");
    assert_eq!(client.context_name(), "wiremock");
    // nifi-rust-client 0.7.0 adds V2_9_0 to the dynamic set. When the server
    // reports "2.9.0" and VersionStrategy::Closest is used, the library now
    // maps to V2_9_0 exactly. detected_version() therefore reflects the
    // resolved client version (2.9.0), matching the raw server version.
    assert_eq!(client.detected_version().major, 2);
    assert_eq!(client.detected_version().minor, 9);
    assert_eq!(client.detected_version().patch, 0);

    // about() returns the version string from the raw /flow/about response,
    // which is "2.9.0" as supplied in our stub.
    let snapshot = client.about().await.expect("about should succeed");
    assert_eq!(snapshot.version, "2.9.0");
}
