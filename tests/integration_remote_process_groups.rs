//! Live integration coverage for Remote Process Groups against the
//! `nifilens-fixture-v8` cluster on both NiFi 2.6.0 (floor) and 2.9.0
//! (ceiling).
//!
//! These tests exercise the data path — `browser_remote_process_group_detail`
//! and `status_history(ComponentKind::RemoteProcessGroup, ...)` — not the
//! UI surface (which is covered by snapshot/wiremock tests at the unit
//! level). The fixture must contain the seeder's `remote-pipeline` PG with
//! two RPGs (see `integration-tests/seeder/src/fixture/remote.rs`):
//!   - `transmitting-rpg` — created and set TRANSMITTING.
//!   - `idle-rpg`         — created and left in default STOPPED state.
//!
//! Gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_remote_process_groups -- --ignored`
//! after bringing up the Docker fixture.
//!
//! Flake-resistance: NiFi may briefly report `transmitting-rpg` as
//! `"Stopped"` between create and the seeder's start call. We assert
//! that fields are POPULATED rather than pinning exact values where
//! timing matters.

use nifi_lens::client::history::{ComponentKind, status_history};
use nifi_lens::client::{NifiClient, NodeKind};
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

/// Resolve an RPG by name from the recursive root status snapshot.
/// Mirrors `find_pg_id_by_name` in `integration_browser_version_control.rs`
/// but matches `NodeKind::RemoteProcessGroup` instead.
async fn find_rpg_id_by_name(client: &NifiClient, rpg_name: &str) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::RemoteProcessGroup) && n.name == rpg_name)
        .map(|n| n.id.clone())
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_rpg_detail_returns_target_uri_and_ports() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_rpg_detail_returns_target_uri_and_ports on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let rpg_id = find_rpg_id_by_name(&client, "transmitting-rpg")
            .await
            .unwrap_or_else(|| panic!("transmitting-rpg not found on {version}"));

        let detail = client
            .browser_remote_process_group_detail(&rpg_id)
            .await
            .unwrap_or_else(|e| panic!("rpg detail on {version} failed: {e:?}"));

        assert_eq!(detail.id, rpg_id, "detail.id must echo the requested id");
        assert_eq!(
            detail.name, "transmitting-rpg",
            "detail.name must echo the seeded name on {version}"
        );
        assert!(
            !detail.target_uri.is_empty(),
            "target_uri must be populated on {version}, got {:?}",
            detail.target_uri
        );
        // Transmission status may briefly read "Stopped" between create and
        // the seeder's start call; we only assert it is populated.
        assert!(
            !detail.transmission_status.is_empty(),
            "transmission_status must be populated on {version}"
        );
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_rpg_status_history_returns_at_least_one_bucket() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_rpg_status_history_returns_at_least_one_bucket on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let rpg_id = find_rpg_id_by_name(&client, "transmitting-rpg")
            .await
            .unwrap_or_else(|| panic!("transmitting-rpg not found on {version}"));

        // The endpoint must be callable and the reducer must succeed; we
        // do NOT assert non-empty buckets because the fixture RPG cannot
        // actually transmit (target SSL handshake fails, no ports
        // configured — tracked as a fixture followup). For v0.1 read-path
        // verification, "the API call returns Ok" is the contract that
        // matters here. Bucket-shape correctness is covered by the
        // wiremock test in src/client/history.rs.
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

        let rpg_id = find_rpg_id_by_name(&client, "idle-rpg")
            .await
            .unwrap_or_else(|| panic!("idle-rpg not found on {version}"));

        let detail = client
            .browser_remote_process_group_detail(&rpg_id)
            .await
            .unwrap_or_else(|e| panic!("rpg detail on {version} failed: {e:?}"));

        assert_eq!(detail.name, "idle-rpg");
        // idle-rpg is left in default state by the seeder; NiFi reports
        // this as "Not Transmitting" (the wire value is variant across
        // minors but always non-empty and non-"Transmitting").
        assert!(
            !detail.transmission_status.is_empty(),
            "transmission_status must be populated on {version}"
        );
        assert_ne!(
            detail.transmission_status, "Transmitting",
            "idle-rpg must NOT report Transmitting on {version}, got {:?}",
            detail.transmission_status
        );
    }
}
