//! Live integration tests for the access-policies-audit feature.
//! Run with `--ignored` against the integration-tests Docker fixture
//! (must have been seeded — `seed_access_fixture` adds the `ops-team`
//! group and component-level policies the assertions rely on).

#[path = "common/mod.rs"]
mod common;

use common::access_helpers::{lookup_pg_id_by_name, lookup_user_id_by_identity};
use common::versions::{context_for, port_for};
use nifi_lens::client::NifiClient;
use nifi_lens::client::NodeKind;
use nifi_lens::client::access::{
    fetch_component_access, fetch_component_access_with_audit, fetch_identity_grants,
};
use nifi_lens::cluster::AccessAuditState;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use nifi_lens::view::browser::state::access_modal::{Axis, AxisOutcome};
use nifi_lens::view::browser::state::identity_modal::{IdentityKind, ResourceBucket};

/// Build a `ResolvedContext` for `version` from the standard integration
/// env vars. Mirrors the inline context construction in other
/// `integration_*` tests.
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

async fn connect_2_9_0() -> NifiClient {
    let ctx = it_context("2.9.0");
    NifiClient::connect(&ctx)
        .await
        .unwrap_or_else(|e| panic!("connect to 2.9.0 failed: {e:?}"))
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn access_matrix_shows_ops_team_on_orders_pipeline() {
    let client = connect_2_9_0().await;
    let pg_id = lookup_pg_id_by_name(&client, "orders-pipeline").await;

    let result = fetch_component_access(&client, NodeKind::ProcessGroup, &pg_id).await;

    let view = result
        .outcomes
        .get(&Axis::ViewComponent)
        .expect("view axis");
    assert!(
        matches!(view, AxisOutcome::Direct { groups, .. }
            if groups.iter().any(|g| g.identity == "ops-team")),
        "view axis must have ops-team as Direct, got {view:?}"
    );

    let modify = result
        .outcomes
        .get(&Axis::ModifyComponent)
        .expect("modify axis");
    assert!(
        matches!(modify, AxisOutcome::Direct { groups, .. }
            if groups.iter().any(|g| g.identity == "ops-team")),
        "modify axis must have ops-team as Direct, got {modify:?}"
    );

    let data = result.outcomes.get(&Axis::ViewData).expect("data axis");
    assert!(
        matches!(data, AxisOutcome::Direct { groups, .. }
            if groups.iter().any(|g| g.identity == "ops-team")),
        "data axis must have ops-team as Direct, got {data:?}"
    );

    let operate = result.outcomes.get(&Axis::Operate).expect("operate axis");
    assert!(
        matches!(operate, AxisOutcome::Direct { groups, .. }
            if groups.iter().any(|g| g.identity == "ops-team")),
        "operate axis must have ops-team as Direct, got {operate:?}"
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn drill_in_bob_includes_versioned_clean_grant() {
    let client = connect_2_9_0().await;
    let bob_id = lookup_user_id_by_identity(&client, "bob").await;
    let versioned_clean_id = lookup_pg_id_by_name(&client, "versioned-clean").await;

    let result = fetch_identity_grants(&client, IdentityKind::User, &bob_id)
        .await
        .expect("fetch bob grants");

    assert!(
        result
            .grants
            .iter()
            .any(|g| g.bucket == ResourceBucket::ProcessGroups
                && g.resource.contains(&versioned_clean_id)),
        "bob must have a ProcessGroups grant referencing versioned-clean ({versioned_clean_id}); got {:?}",
        result.grants,
    );
}

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn access_audit_state_promotes_to_supported_after_first_call() {
    let client = connect_2_9_0().await;
    let pg_id = lookup_pg_id_by_name(&client, "orders-pipeline").await;

    let (_, audit) = fetch_component_access_with_audit(
        &client,
        NodeKind::ProcessGroup,
        &pg_id,
        AccessAuditState::Unknown,
    )
    .await;

    assert_eq!(
        audit,
        AccessAuditState::Supported,
        "managed-authorizer fixture must register as Supported"
    );
}
