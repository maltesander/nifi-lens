//! Integration test: connect against a real NiFi container booted by
//! `integration-tests/run.sh`. `#[ignore]`-gated so `cargo test` alone
//! does not touch Docker.

use nifi_lens::client::NifiClient;
use nifi_lens::config::{ResolvedContext, VersionStrategy};

#[tokio::test]
#[ignore]
async fn connect_detects_version_and_reads_about() {
    let url = std::env::var("NIFILENS_IT_URL").expect("NIFILENS_IT_URL must be set");
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    let ctx = ResolvedContext {
        name: "it".into(),
        url,
        username,
        password,
        version_strategy: VersionStrategy::Closest,
        insecure_tls: false,
        ca_cert_path: Some(ca_path.into()),
    };

    let client = NifiClient::connect(&ctx)
        .await
        .expect("connect should succeed");
    assert_eq!(client.detected_version().major, 2, "expected NiFi 2.x");

    let about = client.about().await.expect("about should succeed");
    // The about.version is the raw NiFi version from /flow/about; it may
    // differ from client.detected_version() because the library's version
    // strategy may map "2.9.0" → "2.8.0" (closest supported).
    assert!(
        about.version.starts_with("2."),
        "expected 2.x version, got {}",
        about.version
    );
}
