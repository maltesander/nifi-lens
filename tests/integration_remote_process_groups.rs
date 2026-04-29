//! Live integration coverage for Remote Process Groups against the
//! `nifilens-fixture-v8` cluster on both NiFi 2.6.0 (floor) and 2.9.0
//! (ceiling).
//!
//! These tests exercise the data path — `browser_remote_process_group_detail`
//! and `status_history(ComponentKind::RemoteProcessGroup, ...)` — not the
//! UI surface (which is covered by snapshot/wiremock tests at the unit
//! level). The fixture must contain the seeder's `remote-pipeline` PG with
//! two RPGs (see `integration-tests/seeder/src/fixture/remote.rs`):
//!   - one set to TRANSMITTING (`transmissionStatus = "Transmitting"`).
//!   - one left in default STOPPED state (`transmissionStatus = "NotTransmitting"`).
//!
//! Lookup is by `transmissionStatus`, NOT by user-given name. NiFi
//! discards the user-supplied `name` on RPG creation and replaces it
//! with the target's flow name (always `"NiFi Flow"` in our fixture
//! since both RPGs target the floor NiFi). The two RPGs are otherwise
//! indistinguishable from the API surface — only their transmission
//! state differs.
//!
//! Gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_remote_process_groups -- --ignored`
//! after bringing up the Docker fixture.

use nifi_lens::client::history::{ComponentKind, status_history};
use nifi_lens::client::{NifiClient, NodeKind, NodeStatusSummary};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

fn it_context(version: &str) -> ResolvedContext {
    let username = std::env::var("NIFILENS_IT_USERNAME").expect("NIFILENS_IT_USERNAME must be set");
    let password = std::env::var("NIFILENS_IT_PASSWORD").expect("NIFILENS_IT_PASSWORD must be set");
    let ca_path =
        std::env::var("NIFILENS_IT_CA_CERT_PATH").expect("NIFILENS_IT_CA_CERT_PATH must be set");

    ResolvedContext {
        name: context_for(version),
        url: format!("https://localhost:{}", port_for(version)),
        auth: ResolvedAuth::Password { username, password },
        version_strategy: VersionStrategy::Closest,
        insecure_tls: false,
        ca_cert_path: Some(ca_path.into()),
        proxied_entities_chain: None,
        proxy_url: None,
        http_proxy_url: None,
        https_proxy_url: None,
    }
}

/// Resolve an RPG by `transmissionStatus` from the recursive root status
/// snapshot. NiFi overwrites the RPG's user-given name with the target's
/// flow name on create, so the two fixture RPGs both report `name =
/// "NiFi Flow"` and can only be told apart by their transmission state.
async fn find_rpg_id_by_transmission_status(
    client: &NifiClient,
    transmission_status: &str,
) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| {
            matches!(n.kind, NodeKind::RemoteProcessGroup)
                && match &n.status_summary {
                    NodeStatusSummary::RemoteProcessGroup {
                        transmission_status: ts,
                        ..
                    } => ts == transmission_status,
                    _ => false,
                }
        })
        .map(|n| n.id.clone())
}

const TRANSMITTING: &str = "Transmitting";
const NOT_TRANSMITTING: &str = "NotTransmitting";
const REMOTE_TARGET_URI: &str = "https://nifi-2-6-0:8443/nifi";

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_rpg_detail_returns_target_uri_and_ports() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_rpg_detail_returns_target_uri_and_ports on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let rpg_id = find_rpg_id_by_transmission_status(&client, TRANSMITTING)
            .await
            .unwrap_or_else(|| panic!("no Transmitting RPG found on {version}"));

        let detail = client
            .browser_remote_process_group_detail(&rpg_id)
            .await
            .unwrap_or_else(|e| panic!("rpg detail on {version} failed: {e:?}"));

        assert_eq!(detail.id, rpg_id, "detail.id must echo the requested id");
        assert_eq!(
            detail.target_uri, REMOTE_TARGET_URI,
            "detail.target_uri must echo the seeded URI on {version}"
        );
        assert!(
            detail.target_secure,
            "target_secure must be true for an HTTPS target on {version}"
        );
        assert_eq!(
            detail.transmission_status, TRANSMITTING,
            "transmission_status must be Transmitting on {version}"
        );
        assert_eq!(
            detail.validation_status, "VALID",
            "fixture RPG must validate on {version}, got {:?}",
            detail.validation_status
        );
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_rpg_status_history_endpoint_is_callable() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_rpg_status_history_endpoint_is_callable on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let rpg_id = find_rpg_id_by_transmission_status(&client, TRANSMITTING)
            .await
            .unwrap_or_else(|| panic!("no Transmitting RPG found on {version}"));

        // The endpoint must be callable and the reducer must succeed.
        // We do NOT assert non-empty buckets because the fixture RPG
        // doesn't have port mappings or actual data flow, so NiFi's
        // VolatileComponentStatusRepository may not have observed it yet.
        // Bucket-shape correctness (including metric-key extraction) is
        // covered by the wiremock test in `src/client/history.rs`.
        let _series = status_history(&client, ComponentKind::RemoteProcessGroup, &rpg_id)
            .await
            .unwrap_or_else(|e| panic!("rpg status_history on {version} failed: {e:?}"));
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_idle_rpg_detail_reports_not_transmitting() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_idle_rpg_detail_reports_not_transmitting on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let rpg_id = find_rpg_id_by_transmission_status(&client, NOT_TRANSMITTING)
            .await
            .unwrap_or_else(|| panic!("no NotTransmitting RPG found on {version}"));

        let detail = client
            .browser_remote_process_group_detail(&rpg_id)
            .await
            .unwrap_or_else(|e| panic!("rpg detail on {version} failed: {e:?}"));

        assert_eq!(detail.id, rpg_id);
        assert_eq!(detail.target_uri, REMOTE_TARGET_URI);
        assert_eq!(
            detail.transmission_status, NOT_TRANSMITTING,
            "idle RPG must report NotTransmitting on {version}"
        );
    }
}
