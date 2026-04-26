//! Integration tests for the parameter-context feature. Runs against the live
//! fixture (`./integration-tests/run.sh`) on both NiFi 2.6.0 (floor) and
//! 2.9.0 (ceiling).
//!
//! These tests exercise the data path — `parameter_context_bindings_batch`,
//! `fetch_chain`, `browser_processor_detail` — not the TUI surface (which is
//! covered by snapshot tests at the unit level). The fixture must contain two
//! parameter contexts and one process group created by the seeder's
//! `fixture::parameterized` module:
//!
//!   - `fixture-pc-base`: `kafka_bootstrap`, `retry_max=3`, sensitive
//!     `db_password`.
//!   - `fixture-pc-prod`: `retry_max=5` (override), `region`; inherits base.
//!   - `parameterized-pipeline`: bound to `fixture-pc-prod`.
//!   - `LogAttribute-parameterized`: `Log Payload = "connecting to
//!     #{kafka_bootstrap}"`, `Log Prefix = "##{literal_text}"`.
//!
//! The marker PG is `nifilens-fixture-v7`.

use std::sync::Arc;

use nifi_lens::client::parameter_context::{ChainFetchResult, fetch_chain};
use nifi_lens::client::{NifiClient, NodeKind};
use nifi_lens::config::{ResolvedAuth, ResolvedContext, VersionStrategy};
use tokio::sync::RwLock;

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

/// Find a process group by name from the recursive root status snapshot.
async fn find_pg_id_by_name(client: &NifiClient, pg_name: &str) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::ProcessGroup) && n.name == pg_name)
        .map(|n| n.id.clone())
}

/// Find a processor by name from the recursive root status snapshot.
async fn find_processor_id_by_name(client: &NifiClient, proc_name: &str) -> Option<String> {
    let snap = client.root_pg_status().await.ok()?;
    snap.nodes
        .iter()
        .find(|n| matches!(n.kind, NodeKind::Processor) && n.name == proc_name)
        .map(|n| n.id.clone())
}

// ─── Test 1: bindings batch finds parameterized-pipeline bound to pc-prod ────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn parameter_context_bindings_batch_reports_parameterized_pipeline_binding() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- parameter_context_bindings_batch on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        // Collect all PG ids from the fixture.
        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let all_pg_ids: Vec<String> = snap.process_group_ids.clone();
        assert!(
            !all_pg_ids.is_empty(),
            "fixture on {version} must have at least one PG"
        );

        let map = client.parameter_context_bindings_batch(&all_pg_ids).await;

        // Find the parameterized-pipeline PG id.
        let pg_id = find_pg_id_by_name(&client, "parameterized-pipeline")
            .await
            .unwrap_or_else(|| panic!("parameterized-pipeline PG not found on {version}"));

        // The map must contain an entry for this PG with a non-None binding.
        let binding = map
            .by_pg_id
            .get(&pg_id)
            .unwrap_or_else(|| {
                panic!(
                    "parameterized-pipeline pg_id={pg_id} not in bindings map on {version}; \
                     map has {} entries",
                    map.by_pg_id.len()
                )
            })
            .as_ref()
            .unwrap_or_else(|| {
                panic!(
                    "parameterized-pipeline has None binding on {version}; expected fixture-pc-prod"
                )
            });

        assert_eq!(
            binding.name, "fixture-pc-prod",
            "parameterized-pipeline must be bound to fixture-pc-prod on {version}, \
             got {:?}",
            binding.name
        );
    }
}

// ─── Test 2: fetch_chain resolves two-context chain with correct parameters ──

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn fetch_chain_resolves_prod_base_chain_for_parameterized_pipeline() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- fetch_chain resolves prod+base chain on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        // Find parameterized-pipeline and its bound context.
        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let all_pg_ids: Vec<String> = snap.process_group_ids.clone();

        let map = client.parameter_context_bindings_batch(&all_pg_ids).await;

        let pg_id = find_pg_id_by_name(&client, "parameterized-pipeline")
            .await
            .unwrap_or_else(|| panic!("parameterized-pipeline not found on {version}"));

        let binding = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("parameterized-pipeline has no binding on {version}"));

        let bound_id = binding.id.clone();

        // Resolve the chain.
        let arc_client = Arc::new(RwLock::new(client));
        let result = fetch_chain(arc_client, &bound_id).await;

        let nodes = match result {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => {
                panic!("fetch_chain BoundFailed on {version}: {e}")
            }
        };

        assert_eq!(
            nodes.len(),
            2,
            "expected two nodes in the chain (prod + base) on {version}, got {}; \
             nodes: {:?}",
            nodes.len(),
            nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
        );

        // First node must be fixture-pc-prod (directly bound).
        let prod = &nodes[0];
        assert_eq!(
            prod.name, "fixture-pc-prod",
            "first chain node must be fixture-pc-prod on {version}, got {:?}",
            prod.name
        );
        assert!(
            prod.fetch_error.is_none(),
            "prod node has fetch_error on {version}"
        );
        // Prod defines: retry_max=5, region=eu-west-1.
        assert!(
            prod.parameters.iter().any(|p| p.name == "retry_max"),
            "fixture-pc-prod must define retry_max on {version}"
        );
        assert!(
            prod.parameters.iter().any(|p| p.name == "region"),
            "fixture-pc-prod must define region on {version}"
        );

        // Second node must be fixture-pc-base (ancestor).
        let base = &nodes[1];
        assert_eq!(
            base.name, "fixture-pc-base",
            "second chain node must be fixture-pc-base on {version}, got {:?}",
            base.name
        );
        assert!(
            base.fetch_error.is_none(),
            "base node has fetch_error on {version}"
        );
        // Base defines: kafka_bootstrap, retry_max=3, db_password (sensitive).
        assert!(
            base.parameters.iter().any(|p| p.name == "kafka_bootstrap"),
            "fixture-pc-base must define kafka_bootstrap on {version}"
        );
    }
}

// ─── Test 3: retry_max is overridden (shadowed) in the resolved flat list ────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn resolved_chain_retry_max_is_overridden_by_prod() {
    use nifi_lens::view::browser::state::parameter_context_modal::resolve;

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- resolved chain override check on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids)
            .await;

        let pg_id = find_pg_id_by_name(&client, "parameterized-pipeline")
            .await
            .unwrap_or_else(|| panic!("parameterized-pipeline not found on {version}"));

        let bound_id = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("no binding on {version}"))
            .id
            .clone();

        let arc_client = Arc::new(RwLock::new(client));
        let nodes = match fetch_chain(arc_client, &bound_id).await {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => panic!("BoundFailed on {version}: {e}"),
        };

        let resolved = resolve(&nodes, None);

        // retry_max must be won by prod (value "5") and shadowed by base (value "3").
        let retry = resolved
            .iter()
            .find(|r| r.winner.name == "retry_max")
            .unwrap_or_else(|| panic!("retry_max not in resolved list on {version}"));

        assert_eq!(
            retry.winner.value.as_deref(),
            Some("5"),
            "retry_max winner value must be 5 (prod) on {version}, got {:?}",
            retry.winner.value
        );
        assert_eq!(
            retry.winner_context, "fixture-pc-prod",
            "retry_max winner_context must be fixture-pc-prod on {version}, got {:?}",
            retry.winner_context
        );
        assert_eq!(
            retry.shadowed.len(),
            1,
            "retry_max must have exactly one shadowed entry (base) on {version}"
        );
        assert_eq!(
            retry.shadowed[0].1, "fixture-pc-base",
            "retry_max shadowed entry must be from fixture-pc-base on {version}"
        );
        assert!(
            !retry.shadowed.is_empty(),
            "retry_max must have shadowed entry — confirming [O] flag"
        );
    }
}

// ─── Test 4: db_password is sensitive — value withheld by fetch_chain ────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn resolved_chain_db_password_is_sensitive_and_value_withheld() {
    use nifi_lens::view::browser::state::parameter_context_modal::resolve;

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- db_password sensitive check on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids)
            .await;

        let pg_id = find_pg_id_by_name(&client, "parameterized-pipeline")
            .await
            .unwrap_or_else(|| panic!("parameterized-pipeline not found on {version}"));

        let bound_id = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("no binding on {version}"))
            .id
            .clone();

        let arc_client = Arc::new(RwLock::new(client));
        let nodes = match fetch_chain(arc_client, &bound_id).await {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => panic!("BoundFailed on {version}: {e}"),
        };

        let resolved = resolve(&nodes, None);

        let pwd = resolved
            .iter()
            .find(|r| r.winner.name == "db_password")
            .unwrap_or_else(|| panic!("db_password not in resolved list on {version}"));

        assert!(
            pwd.winner.sensitive,
            "db_password must be sensitive on {version}"
        );
        assert!(
            pwd.winner.value.is_none(),
            "db_password value must be withheld (None) by fetch_chain on {version}, \
             got {:?}",
            pwd.winner.value
        );
    }
}

// ─── Test 5: used-by inverted map lists parameterized-pipeline for pc-prod ───

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn bindings_map_inverted_shows_parameterized_pipeline_uses_pc_prod() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- inverted bindings (used-by) check on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids)
            .await;

        // Build an inverted map: context_name → Vec<pg_name>
        // We need PG names as well, so pair id→name from the snap.
        let pg_name_by_id: std::collections::HashMap<String, String> = snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::ProcessGroup))
            .map(|n| (n.id.clone(), n.name.clone()))
            .collect();

        // For fixture-pc-prod, at least one bound PG must be "parameterized-pipeline".
        let pc_prod_binders: Vec<&str> = map
            .by_pg_id
            .iter()
            .filter_map(|(pg_id, binding)| {
                let b = binding.as_ref()?;
                if b.name == "fixture-pc-prod" {
                    pg_name_by_id.get(pg_id).map(|n| n.as_str())
                } else {
                    None
                }
            })
            .collect();

        assert!(
            pc_prod_binders.contains(&"parameterized-pipeline"),
            "fixture-pc-prod must have parameterized-pipeline as a binder on {version}; \
             actual binders: {pc_prod_binders:?}"
        );
    }
}

// ─── Test 6: Log Payload property contains #{kafka_bootstrap} ────────────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn processor_log_payload_property_contains_param_reference() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- Log Payload param-ref check on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let proc_id = find_processor_id_by_name(&client, "LogAttribute-parameterized")
            .await
            .unwrap_or_else(|| panic!("LogAttribute-parameterized not found on {version}"));

        let detail = client
            .browser_processor_detail(&proc_id)
            .await
            .unwrap_or_else(|e| panic!("browser_processor_detail on {version} failed: {e:?}"));

        // Verify the "Log Payload" property contains the #{kafka_bootstrap} reference.
        let log_payload = detail
            .properties
            .iter()
            .find(|(key, _)| key == "Log Payload")
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "Log Payload property not found on LogAttribute-parameterized on {version}; \
                     properties: {:?}",
                    detail.properties.iter().map(|(k, _)| k).collect::<Vec<_>>()
                )
            });

        assert!(
            log_payload.contains("#{kafka_bootstrap}"),
            "Log Payload must contain #{{kafka_bootstrap}} on {version}, got: {log_payload:?}"
        );
    }
}

// ─── Test 7: Log Prefix property is the escape case — no #{...} reference ────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn processor_log_prefix_property_is_escape_not_param_reference() {
    use nifi_lens::view::browser::render::{ParamRefScan, scan_param_refs};

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- Log Prefix escape check on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let proc_id = find_processor_id_by_name(&client, "LogAttribute-parameterized")
            .await
            .unwrap_or_else(|| panic!("LogAttribute-parameterized not found on {version}"));

        let detail = client
            .browser_processor_detail(&proc_id)
            .await
            .unwrap_or_else(|e| panic!("browser_processor_detail on {version} failed: {e:?}"));

        // NiFi 2.6.0 uses "Log prefix" (lower-case p); 2.8.0+ uses "Log Prefix".
        // Accept either casing by doing a case-insensitive search.
        let log_prefix_value = detail
            .properties
            .iter()
            .find(|(key, _)| key.eq_ignore_ascii_case("Log Prefix"))
            .map(|(_, v)| v.as_str())
            .unwrap_or_else(|| {
                panic!(
                    "Log Prefix property not found on LogAttribute-parameterized on {version}; \
                     properties: {:?}",
                    detail.properties.iter().map(|(k, _)| k).collect::<Vec<_>>()
                )
            });

        // The stored value is "##{literal_text}" — the escape sequence.
        assert!(
            log_prefix_value.starts_with("##"),
            "Log Prefix must be an escape (##{{...}}) on {version}, got: {log_prefix_value:?}"
        );

        // The param-ref scanner must report None — no cross-link annotation expected.
        let scan = scan_param_refs(log_prefix_value);
        assert_eq!(
            scan,
            ParamRefScan::None,
            "scan_param_refs must return None for the escape #{{{{literal_text}}}} on {version}; \
             got: {scan:?}"
        );
    }
}
