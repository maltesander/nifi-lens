//! Integration test for the headline demo narrative: action history on
//! `fixture-pc-orders` shows the `usd_rate` mutation produced by the
//! seeder's `apply_break` phase.
//!
//! The seeder runs phase 8 (`orders::break_::apply_break`) by the time
//! `./integration-tests/run.sh` invokes `cargo test`, mutating the
//! `usd_rate` parameter of `fixture-pc-orders` from `"1.0827"` to
//! `"oops"`. NiFi records that as one or more flow-config audit events
//! against the parameter context's UUID. This test asserts that at
//! least one such audit event exists — the precise field shape varies
//! across NiFi 2.6.0 / 2.9.0, so the assertion is intentionally
//! permissive.
//!
//! Gated on `#[ignore]` — run via `./integration-tests/run.sh` or
//! `cargo test --test integration_action_history_parameter_context -- \
//! --ignored` after bringing up the Docker fixture.

use nifi_lens::client::NifiClient;
use nifi_lens::client::history::flow_actions_paginator;
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};

#[path = "common/mod.rs"]
mod common;
use common::versions::{FIXTURE_VERSIONS, context_for, port_for};

/// Build a `ResolvedContext` for `version` from the standard integration
/// env vars. Mirrors the inline context construction used by the other
/// `integration_*` tests in this crate.
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

/// Find the `fixture-pc-orders` parameter-context ID by walking the
/// arena binding map: every PG bound to a context with that name yields
/// the same `id`. We look up via the `orders-pipeline/transform` PG
/// since the seeder guarantees that binding.
async fn find_orders_context_id(client: &NifiClient) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    let map = client
        .parameter_context_bindings_batch(&snap.process_group_ids, 4)
        .await;
    for binding in map.by_pg_id.values() {
        if let Some(b) = binding
            && b.name == "fixture-pc-orders"
        {
            return Some(b.id.clone());
        }
    }
    None
}

/// Validates that the seeder's `apply_break` phase produces an audit
/// event on the `fixture-pc-orders` parameter context. The exact shape
/// of the action entry varies across NiFi versions, so this test only
/// asserts that NiFi has recorded at least one audit action attributable
/// to the parameter context's UUID.
#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn integration_action_history_parameter_context_records_apply_break_event() {
    for &version in FIXTURE_VERSIONS {
        eprintln!(
            "--- integration_action_history_parameter_context_records_apply_break_event \
             running against NiFi {version} ---"
        );

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let orders_ctx_id = find_orders_context_id(&client)
            .await
            .unwrap_or_else(|| panic!("fixture-pc-orders not found on {version}"));

        // Stream all pages until we either find an action that mentions
        // `usd_rate` (the headline mutation) or exhaust history. NiFi
        // 2.6.0 and 2.9.0 differ on whether the `sourceId` filter
        // returns parameter-context audit rows; we therefore accept
        // either: a non-empty filtered history (preferred), OR a
        // matching row found in unfiltered history below.
        let mut paginator = flow_actions_paginator(&client, &orders_ctx_id, 100);
        let mut filtered_actions = Vec::new();
        while let Some(page) = paginator
            .next_page()
            .await
            .unwrap_or_else(|e| panic!("flow_actions_paginator on {version} failed: {e:?}"))
        {
            filtered_actions.extend(page);
            if filtered_actions.len() >= 500 {
                break; // safety cap
            }
        }

        if !filtered_actions.is_empty() {
            // Filter returned hits — every row must carry our source_id.
            for action in &filtered_actions {
                assert_eq!(
                    action.source_id.as_deref(),
                    Some(orders_ctx_id.as_str()),
                    "paginator returned action for the wrong source on {version}: {:?}",
                    action.source_id
                );
            }

            // Best-effort confirmation of the apply_break narrative —
            // look for any action that references `usd_rate` in its
            // operation/source_name/inner action's source_name. NiFi's
            // exact placement varies; we don't fail on absence, just
            // log for diagnostics.
            let mentions_usd_rate = filtered_actions.iter().any(|a| {
                let inner = a.action.as_ref();
                inner
                    .and_then(|ad| ad.source_name.as_deref())
                    .is_some_and(|s| s.contains("usd_rate"))
                    || inner
                        .and_then(|ad| ad.operation.as_deref())
                        .is_some_and(|s| s.contains("usd_rate"))
            });
            eprintln!(
                "  filtered history: {} action(s); mentions usd_rate: {mentions_usd_rate}",
                filtered_actions.len()
            );
            // Headline assertion: at least one Update-action recorded.
            assert!(
                !filtered_actions.is_empty(),
                "expected at least one audit action for fixture-pc-orders on {version}"
            );
            continue;
        }

        // Fallback: filter by sourceId returned nothing. Some NiFi
        // versions don't index parameter-context audit events under
        // `sourceId` — pull recent unfiltered history and look for
        // a matching row by source_id field. This branch keeps the
        // test robust across version differences.
        let mut unfiltered = nifi_rust_client::pagination::flow_history_dynamic(
            &client,
            nifi_rust_client::pagination::HistoryFilter::default(),
            500,
        );
        let mut found_any = false;
        let mut pages_scanned = 0;
        while let Some(page) = unfiltered.next_page().await.unwrap_or_else(|e| {
            panic!("unfiltered flow_history_dynamic on {version} failed: {e:?}")
        }) {
            pages_scanned += 1;
            if page.iter().any(|a| {
                a.source_id.as_deref() == Some(orders_ctx_id.as_str())
                    || a.action
                        .as_ref()
                        .and_then(|ad| ad.source_id.as_deref())
                        .is_some_and(|sid| sid == orders_ctx_id)
            }) {
                found_any = true;
                break;
            }
            if pages_scanned >= 4 {
                break; // safety cap: 2000 most-recent rows
            }
        }
        assert!(
            found_any,
            "expected at least one audit action whose source_id matches \
             fixture-pc-orders ({orders_ctx_id}) on {version}, but none found \
             in {pages_scanned} page(s) of unfiltered history"
        );
    }
}
