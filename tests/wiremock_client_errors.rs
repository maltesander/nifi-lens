//! Wiremock tests: error paths for NifiClient::connect.

use nifi_lens::NifiLensError;
use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

fn ctx_for(url: String) -> ResolvedContext {
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
    }
}

#[tokio::test]
async fn login_401_surfaces_unauthorized_with_hint() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let err = NifiClient::connect(&ctx_for(server.uri()))
        .await
        .expect_err("connect should fail");
    assert!(
        matches!(err, NifiLensError::NifiUnauthorized { .. }),
        "expected NifiUnauthorized, got {err:?}"
    );
    // Spot-check the hint is present in the user-visible display string.
    let msg = err.to_string();
    assert!(
        msg.contains("rejected the credentials") && msg.contains("password_env"),
        "expected hint about credentials in display message, got: {msg}"
    );
}

#[tokio::test]
async fn about_500_surfaces_error() {
    let server = MockServer::start().await;

    Mock::given(method("POST"))
        .and(path("/nifi-api/access/token"))
        .respond_with(ResponseTemplate::new(200).set_body_string("stub-jwt-token"))
        .mount(&server)
        .await;

    // Version detection happens inside login() (the library calls /flow/about
    // automatically). So a 500 on /flow/about will surface during connect,
    // either as LoginFailed (if the library treats version detection as part
    // of login) or as AboutFailed (if it's a separate error path). Accept
    // either variant.
    Mock::given(method("GET"))
        .and(path("/nifi-api/flow/about"))
        .respond_with(ResponseTemplate::new(500))
        .mount(&server)
        .await;

    let err = NifiClient::connect(&ctx_for(server.uri()))
        .await
        .expect_err("connect should fail");
    assert!(
        matches!(
            err,
            NifiLensError::LoginFailed { .. } | NifiLensError::AboutFailed { .. }
        ),
        "expected LoginFailed or AboutFailed, got {err:?}"
    );
}
