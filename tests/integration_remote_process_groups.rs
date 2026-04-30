//! Live integration coverage for Remote Process Groups against the
//! `nifilens-fixture-v8` cluster on both NiFi 2.6.0 (floor) and 2.9.0
//! (ceiling).
//!
//! These tests exercise the data path — `browser_remote_process_group_detail`
//! and `status_history(ComponentKind::RemoteProcessGroup, ...)` — not the
//! UI surface (which is covered by snapshot/wiremock tests at the unit
//! level). The fixture must contain the seeder's orders-pipeline with
//! two regional sinks owning RPGs (see
//! `integration-tests/seeder/src/fixture/orders/`):
//!   - `orders-pipeline/sink-eu/rpg-eu` — TRANSMITTING.
//!   - `orders-pipeline/sink-apac/rpg-apac` — left in default STOPPED state
//!     (`transmissionStatus = "NotTransmitting"`).
//!
//! Lookup is scoped by parent PG (`sink-eu` / `sink-apac`), NOT by the
//! RPG's own name. NiFi discards the user-supplied `name` on RPG
//! creation and replaces it with the target's flow name (always
//! `"NiFi Flow"` in our fixture since both RPGs target the floor NiFi),
//! so user-supplied `rpg-eu` / `rpg-apac` names are not visible from
//! the API. Scoping by parent PG also disambiguates from any other
//! RPGs that may coexist in the cluster during fixture migration.
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

/// Resolve the unique RPG owned by a named PG under the orders-pipeline
/// subtree (`nifilens-fixture-v8 → orders-pipeline → <sink_pg_name>`).
///
/// Returns the RPG node's id and its observed `transmissionStatus`. Each
/// regional sink PG (`sink-eu`, `sink-apac`) owns exactly one RPG in the
/// fixture, so a single lookup unambiguously identifies the target. We
/// scope by parent PG because NiFi overwrites the user-given RPG name
/// (`rpg-eu`, `rpg-apac`) with the target's flow name on create, leaving
/// PG path as the only stable identifier.
async fn find_orders_rpg_under_sink(
    client: &NifiClient,
    sink_pg_name: &str,
) -> Option<(String, String)> {
    let snap = client.root_pg_status().await.ok()?;

    // marker → orders-pipeline → <sink_pg_name>: locate the sink PG by
    // its parent chain so we don't accidentally match a same-named PG
    // elsewhere in the cluster.
    let nodes = &snap.nodes;
    let marker_idx = nodes.iter().position(|n| {
        matches!(n.kind, NodeKind::ProcessGroup) && n.name == "nifilens-fixture-v8"
    })?;
    let orders_idx = nodes.iter().position(|n| {
        matches!(n.kind, NodeKind::ProcessGroup)
            && n.name == "orders-pipeline"
            && n.parent_idx == Some(marker_idx)
    })?;
    let sink_idx = nodes.iter().position(|n| {
        matches!(n.kind, NodeKind::ProcessGroup)
            && n.name == sink_pg_name
            && n.parent_idx == Some(orders_idx)
    })?;
    let sink_pg_id = nodes[sink_idx].id.clone();

    nodes
        .iter()
        .find_map(|n| match (&n.kind, &n.status_summary) {
            (
                NodeKind::RemoteProcessGroup,
                NodeStatusSummary::RemoteProcessGroup {
                    transmission_status,
                    ..
                },
            ) if n.group_id == sink_pg_id => Some((n.id.clone(), transmission_status.clone())),
            _ => None,
        })
}

const TRANSMITTING: &str = "Transmitting";
const NOT_TRANSMITTING: &str = "NotTransmitting";
const REMOTE_TARGET_URI: &str = "https://nifi-2-6-0:8443/nifi";
const SINK_EU: &str = "sink-eu";
const SINK_APAC: &str = "sink-apac";

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn live_rpg_detail_returns_target_uri_and_ports() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- live_rpg_detail_returns_target_uri_and_ports on NiFi {version} ---");
        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let (rpg_id, transmission_status) = find_orders_rpg_under_sink(&client, SINK_EU)
            .await
            .unwrap_or_else(|| panic!("no RPG under orders-pipeline/{SINK_EU} on {version}"));
        assert_eq!(
            transmission_status, TRANSMITTING,
            "orders-pipeline/{SINK_EU} RPG must be Transmitting on {version}"
        );

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

        let (rpg_id, _transmission_status) = find_orders_rpg_under_sink(&client, SINK_EU)
            .await
            .unwrap_or_else(|| panic!("no RPG under orders-pipeline/{SINK_EU} on {version}"));

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

        let (rpg_id, transmission_status) = find_orders_rpg_under_sink(&client, SINK_APAC)
            .await
            .unwrap_or_else(|| panic!("no RPG under orders-pipeline/{SINK_APAC} on {version}"));
        assert_eq!(
            transmission_status, NOT_TRANSMITTING,
            "orders-pipeline/{SINK_APAC} RPG must be NotTransmitting on {version}"
        );

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
