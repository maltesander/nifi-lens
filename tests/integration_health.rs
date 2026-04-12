//! Integration: Health tab data sources.

use nifi_lens::client::NifiClient;
use nifi_lens::client::health::{
    compute_processor_threads, compute_queue_pressure, extract_repositories,
};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_health_pg_status_and_sysdiag() {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- integration_health running against NiFi {version} ---");

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
        };

        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let pg = client
            .root_pg_status_full()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status_full on {version} failed: {e:?}"));
        assert!(
            !pg.connections.is_empty() || !pg.processors.is_empty(),
            "PG status empty on {version}"
        );

        let diag = client
            .system_diagnostics(true)
            .await
            .unwrap_or_else(|e| panic!("system_diagnostics on {version} failed: {e:?}"));
        assert!(
            diag.aggregate.flowfile_repo.is_some(),
            "no flowfile repo on {version}"
        );

        // Run extraction to verify no panics
        let queues = compute_queue_pressure(&pg, 20);
        let procs = compute_processor_threads(&pg, 20);
        let repos = extract_repositories(&diag);
        let _ = (queues, procs, repos);
    }
}
