//! Integration test: connect against real NiFi containers booted by
//! `integration-tests/run.sh`. `#[ignore]`-gated so `cargo test` alone
//! does not touch Docker. Loops over every version in `FIXTURE_VERSIONS`
//! (generated from `integration-tests/versions.toml`).

use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

#[tokio::test]
#[ignore]
async fn connect_detects_version_and_reads_about() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    // `tracing` may not be initialized for integration tests; use plain
    // eprintln so the version shows up in `cargo test --nocapture` output.
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_connect running against NiFi {version} ---");

        let ctx = ResolvedContext {
            name: context_for(version),
            url: format!("https://localhost:{}", port_for(version)),
            auth: ResolvedAuth::Password {
                username: username.clone(),
                password: password.clone(),
            },
            version_strategy: VersionStrategy::Closest,
            insecure_tls: false,
            ca_cert_path: Some(ca_path.clone().into()),
            proxied_entities_chain: None,
            proxy_url: None,
            http_proxy_url: None,
            https_proxy_url: None,
        };

        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));
        assert_eq!(
            client.detected_version().major,
            2,
            "expected NiFi 2.x on {version}"
        );

        let about = client
            .about()
            .await
            .unwrap_or_else(|e| panic!("about on {version} failed: {e:?}"));
        assert!(
            about.version.starts_with("2."),
            "expected 2.x version on {version}, got {}",
            about.version
        );
    }
}
