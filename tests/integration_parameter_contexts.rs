//! Integration tests for the parameter-context feature. Runs against the live
//! fixture (`./integration-tests/run.sh`) on both NiFi 2.6.0 (floor) and
//! 2.9.0 (ceiling).
//!
//! These tests exercise the data path — `parameter_context_bindings_batch`,
//! `fetch_chain`, `browser_processor_detail` — not the TUI surface (which is
//! covered by snapshot tests at the unit level). The fixture must contain
//! the orders-pipeline 5-context hierarchy created by the seeder's
//! `fixture::parameter_contexts` module:
//!
//!   - `fixture-pc-platform` (root):   `kafka_bootstrap`,
//!     `audit_log_endpoint`, sensitive `db_password`.
//!   - `fixture-pc-orders` (inherits platform):  `usd_rate`,
//!     `region_filter="EU,US,APAC"`, `currency_default="USD"`,
//!     `retry_max="5"`.
//!   - `fixture-pc-region-eu` (inherits orders): `region_filter="EU"`
//!     (override), `compliance_tag="GDPR-2024"`.
//!   - `fixture-pc-region-us` (inherits orders): `region_filter="US"`
//!     (override), `compliance_tag="SOC2"`.
//!   - `fixture-pc-region-apac` (inherits orders):
//!     `region_filter="APAC"` (override), `compliance_tag="PDPA-2023"`.
//!
//! Bindings (from `orders/mod.rs`):
//!   - `orders-pipeline/ingest`     → `fixture-pc-platform`     (depth 1)
//!   - `orders-pipeline/transform`  → `fixture-pc-orders`       (depth 2)
//!   - `orders-pipeline/sink-eu`    → `fixture-pc-region-eu`    (depth 3)
//!   - `orders-pipeline/sink-us`    → `fixture-pc-region-us`    (depth 3)
//!   - `orders-pipeline/sink-apac`  → `fixture-pc-region-apac`  (depth 3)
//!
//! Note: when the fixture is seeded with `--break-after 0s`, the orders
//! `usd_rate` parameter is mutated from `"1.0827"` to `"oops"` — the
//! headline failure narrative. These tests therefore avoid asserting on
//! `usd_rate`'s value (only its presence) so they pass against either
//! state.
//!
//! The marker PG is `nifilens-fixture-v8`.

use std::sync::Arc;

use nifi_lens::client::parameter_context::{ChainFetchResult, fetch_chain};
use nifi_lens::client::{NifiClient, NodeKind, RawNode};
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

/// Resolve a PG id by walking the arena and matching the slash-separated
/// suffix of the PG's full path-from-root. Mirrors the helper used by the
/// other orders-pipeline integration tests (Tasks 18-22). Required because
/// the legacy `parameterized-pipeline` fixture still exists alongside the
/// orders-pipeline (Phase 9 deletes it), and there are now multiple PGs
/// whose names alone are ambiguous in nested fixtures (e.g. `transform`).
fn find_pg_id_by_path(nodes: &[RawNode], target_pg_path: &str) -> Option<String> {
    let parts: Vec<&str> = target_pg_path.split('/').collect();

    for (i, node) in nodes.iter().enumerate() {
        if node.kind != NodeKind::ProcessGroup {
            continue;
        }

        let mut chain = vec![node.name.as_str()];
        let mut cursor = i;
        while let Some(p) = nodes[cursor].parent_idx {
            cursor = p;
            if matches!(nodes[cursor].kind, NodeKind::ProcessGroup) {
                chain.push(nodes[cursor].name.as_str());
            }
        }
        chain.reverse();

        if chain.len() >= parts.len() {
            let start = chain.len() - parts.len();
            if chain[start..] == parts[..] {
                return Some(node.id.clone());
            }
        }
    }
    None
}

/// Find a processor by its parent PG path-suffix and processor name.
/// Mirrors the slash-separated path lookup used by the other
/// orders-pipeline integration tests so we never hit the legacy
/// `parameterized-pipeline` processor of the same name.
fn find_processor_id_by_path(
    nodes: &[RawNode],
    parent_pg_path: &str,
    proc_name: &str,
) -> Option<String> {
    let parent_id = find_pg_id_by_path(nodes, parent_pg_path)?;
    nodes
        .iter()
        .find(|n| {
            matches!(n.kind, NodeKind::Processor) && n.name == proc_name && n.group_id == parent_id
        })
        .map(|n| n.id.clone())
}

// ─── Test 1: bindings batch reports orders-pipeline/transform → pc-orders ────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn parameter_context_bindings_batch_reports_orders_transform_binding() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- bindings_batch on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));
        let all_pg_ids: Vec<String> = snap.process_group_ids.clone();
        assert!(
            !all_pg_ids.is_empty(),
            "fixture on {version} must have at least one PG"
        );

        let map = client
            .parameter_context_bindings_batch(&all_pg_ids, 4)
            .await;

        // Verify all five orders-pipeline child PGs are bound to the
        // expected parameter context. This exercises the full chain-depth
        // matrix (1 / 2 / 3) in one batch call.
        let expected: &[(&str, &str)] = &[
            ("orders-pipeline/ingest", "fixture-pc-platform"),
            ("orders-pipeline/transform", "fixture-pc-orders"),
            ("orders-pipeline/sink-eu", "fixture-pc-region-eu"),
            ("orders-pipeline/sink-us", "fixture-pc-region-us"),
            ("orders-pipeline/sink-apac", "fixture-pc-region-apac"),
        ];
        for (pg_path, expected_ctx_name) in expected {
            let pg_id = find_pg_id_by_path(&snap.nodes, pg_path)
                .unwrap_or_else(|| panic!("fixture {pg_path} PG not found on {version}"));

            let binding = map
                .by_pg_id
                .get(&pg_id)
                .unwrap_or_else(|| {
                    panic!(
                        "{pg_path} pg_id={pg_id} not in bindings map on {version}; \
                         map has {} entries",
                        map.by_pg_id.len()
                    )
                })
                .as_ref()
                .unwrap_or_else(|| {
                    panic!("{pg_path} has None binding on {version}; expected {expected_ctx_name}")
                });

            assert_eq!(
                binding.name, *expected_ctx_name,
                "{pg_path} must be bound to {expected_ctx_name} on {version}, got {:?}",
                binding.name
            );
        }
    }
}

// ─── Test 2: fetch_chain resolves the depth-2 chain for transform ────────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn fetch_chain_resolves_orders_platform_chain_for_transform() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- fetch_chain depth-2 (orders→platform) on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids, 4)
            .await;

        let pg_id = find_pg_id_by_path(&snap.nodes, "orders-pipeline/transform")
            .unwrap_or_else(|| panic!("orders-pipeline/transform not found on {version}"));

        let bound_id = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("orders-pipeline/transform has no binding on {version}"))
            .id
            .clone();

        let arc_client = Arc::new(RwLock::new(client));
        let nodes = match fetch_chain(arc_client, &bound_id).await {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => {
                panic!("fetch_chain BoundFailed on {version}: {e}")
            }
        };

        assert_eq!(
            nodes.len(),
            2,
            "expected two nodes in the chain (orders + platform) on {version}, got {}; \
             nodes: {:?}",
            nodes.len(),
            nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
        );

        let orders = &nodes[0];
        assert_eq!(
            orders.name, "fixture-pc-orders",
            "first chain node must be fixture-pc-orders on {version}, got {:?}",
            orders.name
        );
        assert!(
            orders.fetch_error.is_none(),
            "orders node has fetch_error on {version}"
        );
        // Orders defines: usd_rate, region_filter, currency_default, retry_max.
        // We assert presence rather than value because `usd_rate` is mutated
        // to "oops" by `--break-after`; checking presence stays stable.
        for expected in ["usd_rate", "region_filter", "currency_default", "retry_max"] {
            assert!(
                orders.parameters.iter().any(|p| p.name == expected),
                "fixture-pc-orders must define {expected} on {version}"
            );
        }

        let platform = &nodes[1];
        assert_eq!(
            platform.name, "fixture-pc-platform",
            "second chain node must be fixture-pc-platform on {version}, got {:?}",
            platform.name
        );
        assert!(
            platform.fetch_error.is_none(),
            "platform node has fetch_error on {version}"
        );
        // Platform defines: kafka_bootstrap, audit_log_endpoint, db_password.
        for expected in ["kafka_bootstrap", "audit_log_endpoint", "db_password"] {
            assert!(
                platform.parameters.iter().any(|p| p.name == expected),
                "fixture-pc-platform must define {expected} on {version}"
            );
        }
    }
}

// ─── Test 3: fetch_chain resolves the depth-3 chain for sink-eu ──────────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn fetch_chain_resolves_depth_three_chain_for_sink_eu() {
    for &version in FIXTURE_VERSIONS {
        eprintln!("--- fetch_chain depth-3 (region-eu→orders→platform) on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids, 4)
            .await;

        let pg_id = find_pg_id_by_path(&snap.nodes, "orders-pipeline/sink-eu")
            .unwrap_or_else(|| panic!("orders-pipeline/sink-eu not found on {version}"));

        let bound_id = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("sink-eu has no binding on {version}"))
            .id
            .clone();

        let arc_client = Arc::new(RwLock::new(client));
        let nodes = match fetch_chain(arc_client, &bound_id).await {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => {
                panic!("fetch_chain BoundFailed on {version}: {e}")
            }
        };

        assert_eq!(
            nodes.len(),
            3,
            "expected three nodes in the chain (region-eu + orders + platform) on \
             {version}, got {}; nodes: {:?}",
            nodes.len(),
            nodes.iter().map(|n| &n.name).collect::<Vec<_>>()
        );

        // BFS order: directly-bound first.
        assert_eq!(
            nodes[0].name, "fixture-pc-region-eu",
            "chain[0] on {version} must be fixture-pc-region-eu, got {:?}",
            nodes[0].name
        );
        assert_eq!(
            nodes[1].name, "fixture-pc-orders",
            "chain[1] on {version} must be fixture-pc-orders, got {:?}",
            nodes[1].name
        );
        assert_eq!(
            nodes[2].name, "fixture-pc-platform",
            "chain[2] on {version} must be fixture-pc-platform, got {:?}",
            nodes[2].name
        );

        for n in &nodes {
            assert!(
                n.fetch_error.is_none(),
                "{} has fetch_error on {version}",
                n.name
            );
        }

        // Region-eu defines region_filter (override) + compliance_tag.
        let region = &nodes[0];
        for expected in ["region_filter", "compliance_tag"] {
            assert!(
                region.parameters.iter().any(|p| p.name == expected),
                "fixture-pc-region-eu must define {expected} on {version}"
            );
        }

        // Spot-check the compliance_tag value — this distinguishes the
        // three regional contexts from each other.
        let compliance = region
            .parameters
            .iter()
            .find(|p| p.name == "compliance_tag")
            .expect("compliance_tag must be present in region-eu");
        assert_eq!(
            compliance.value.as_deref(),
            Some("GDPR-2024"),
            "fixture-pc-region-eu compliance_tag value mismatch on {version}, got {:?}",
            compliance.value
        );
    }
}

// ─── Test 4: region_filter is overridden at depth 3 (the [O] flag) ───────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn resolved_chain_region_filter_is_overridden_by_region_eu() {
    use nifi_lens::view::browser::state::parameter_context_modal::resolve;

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- region_filter override at depth-3 on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        let map = client
            .parameter_context_bindings_batch(&snap.process_group_ids, 4)
            .await;

        let pg_id = find_pg_id_by_path(&snap.nodes, "orders-pipeline/sink-eu")
            .unwrap_or_else(|| panic!("orders-pipeline/sink-eu not found on {version}"));

        let bound_id = map
            .by_pg_id
            .get(&pg_id)
            .and_then(|b| b.as_ref())
            .unwrap_or_else(|| panic!("sink-eu has no binding on {version}"))
            .id
            .clone();

        let arc_client = Arc::new(RwLock::new(client));
        let nodes = match fetch_chain(arc_client, &bound_id).await {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => panic!("BoundFailed on {version}: {e}"),
        };

        let resolved = resolve(&nodes, None);

        // region_filter must be won by region-eu (value "EU") and shadowed
        // by orders (value "EU,US,APAC"). The override at depth-3 is the
        // headline coverage for the [O] flag UX in the parameter-context
        // modal.
        let region = resolved
            .iter()
            .find(|r| r.winner.name == "region_filter")
            .unwrap_or_else(|| panic!("region_filter not in resolved list on {version}"));

        assert_eq!(
            region.winner.value.as_deref(),
            Some("EU"),
            "region_filter winner value must be EU (region-eu) on {version}, got {:?}",
            region.winner.value
        );
        assert_eq!(
            region.winner_context, "fixture-pc-region-eu",
            "region_filter winner_context must be fixture-pc-region-eu on {version}, got {:?}",
            region.winner_context
        );
        assert_eq!(
            region.shadowed.len(),
            1,
            "region_filter must have exactly one shadowed entry (orders) on {version}; \
             got: {:?}",
            region.shadowed.iter().map(|(_, n)| n).collect::<Vec<_>>()
        );
        assert_eq!(
            region.shadowed[0].1, "fixture-pc-orders",
            "region_filter shadowed entry must be from fixture-pc-orders on {version}"
        );
        assert_eq!(
            region.shadowed[0].0.value.as_deref(),
            Some("EU,US,APAC"),
            "region_filter shadowed value must be EU,US,APAC on {version}, got {:?}",
            region.shadowed[0].0.value
        );
    }
}

// ─── Test 5: db_password is sensitive — value withheld by fetch_chain ────────

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
            .parameter_context_bindings_batch(&snap.process_group_ids, 4)
            .await;

        // Resolve through the transform binding so `db_password` is
        // reached via the orders→platform inheritance edge — that's
        // the chain-depth-2 sensitive-resolution path the fixture is
        // designed to exercise.
        let pg_id = find_pg_id_by_path(&snap.nodes, "orders-pipeline/transform")
            .unwrap_or_else(|| panic!("orders-pipeline/transform not found on {version}"));

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
        // Sourced from the platform tier (chain-depth-2 from transform).
        assert_eq!(
            pwd.winner_context, "fixture-pc-platform",
            "db_password winner_context must be fixture-pc-platform on {version}, got {:?}",
            pwd.winner_context
        );
    }
}

// ─── Test 6: used-by inverted map for the orders-pipeline contexts ───────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn bindings_map_inverted_shows_orders_pipeline_binders() {
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
            .parameter_context_bindings_batch(&snap.process_group_ids, 4)
            .await;

        // Build an `pg_id → name` lookup so we can phrase the assertion
        // failure messages with PG names.
        let pg_name_by_id: std::collections::HashMap<String, String> = snap
            .nodes
            .iter()
            .filter(|n| matches!(n.kind, NodeKind::ProcessGroup))
            .map(|n| (n.id.clone(), n.name.clone()))
            .collect();

        let collect_binders = |ctx_name: &str| -> Vec<&str> {
            map.by_pg_id
                .iter()
                .filter_map(|(pg_id, binding)| {
                    let b = binding.as_ref()?;
                    if b.name == ctx_name {
                        pg_name_by_id.get(pg_id).map(|n| n.as_str())
                    } else {
                        None
                    }
                })
                .collect()
        };

        // Each context's binder set must contain the expected orders-pipeline
        // child PG. We don't assert exact equality — the parameterized-pipeline
        // legacy fixture still co-exists alongside (Phase 9 deletes it) and
        // may also bind to one of these contexts.
        let cases: &[(&str, &str)] = &[
            ("fixture-pc-platform", "ingest"),
            ("fixture-pc-orders", "transform"),
            ("fixture-pc-region-eu", "sink-eu"),
            ("fixture-pc-region-us", "sink-us"),
            ("fixture-pc-region-apac", "sink-apac"),
        ];
        for (ctx_name, expected_pg_name) in cases {
            let binders = collect_binders(ctx_name);
            assert!(
                binders.contains(expected_pg_name),
                "{ctx_name} must have {expected_pg_name} as a binder on {version}; \
                 actual binders: {binders:?}"
            );
        }
    }
}

// ─── Test 7: processor property contains a #{...} parameter reference ────────

#[tokio::test(flavor = "current_thread")]
#[ignore]
async fn processor_property_contains_param_reference() {
    use nifi_lens::view::browser::render::{ParamRefScan, scan_param_refs};

    for &version in FIXTURE_VERSIONS {
        eprintln!("--- param-ref check on UpdateAttribute-tag-retries on NiFi {version} ---");

        let ctx = it_context(version);
        let client = NifiClient::connect(&ctx)
            .await
            .unwrap_or_else(|e| panic!("connect to {version} failed: {e:?}"));

        let snap = client
            .root_pg_status()
            .await
            .unwrap_or_else(|e| panic!("root_pg_status on {version} failed: {e:?}"));

        // `UpdateAttribute-tag-retries` lives in `orders-pipeline/transform`
        // and carries three dynamic properties that reference parameters:
        // `_max_retries = "#{retry_max}"` (orders-tier),
        // `_audit_endpoint = "#{audit_log_endpoint}"` (platform-tier,
        // resolved via the orders→platform inheritance edge), and
        // `region_filter = "#{region_filter}"` (orders-tier; gets
        // overridden in regional contexts at downstream sinks).
        let proc_id = find_processor_id_by_path(
            &snap.nodes,
            "orders-pipeline/transform",
            "UpdateAttribute-tag-retries",
        )
        .unwrap_or_else(|| {
            panic!(
                "UpdateAttribute-tag-retries not found in orders-pipeline/transform on {version}"
            )
        });

        let detail = client
            .browser_processor_detail(&proc_id)
            .await
            .unwrap_or_else(|e| panic!("browser_processor_detail on {version} failed: {e:?}"));

        // Verify each of the three referencing properties carries a
        // `#{...}` param ref in its raw stored value AND that
        // `scan_param_refs` agrees (the scanner is what drives the
        // trailing `→` cross-link annotation in the Browser detail
        // panel).
        let expected: &[(&str, &str)] = &[
            ("_max_retries", "#{retry_max}"),
            ("_audit_endpoint", "#{audit_log_endpoint}"),
            ("region_filter", "#{region_filter}"),
        ];

        for (prop_name, needle) in expected {
            let value = detail
                .properties
                .iter()
                .find(|(key, _)| key == prop_name)
                .map(|(_, v)| v.as_str())
                .unwrap_or_else(|| {
                    panic!(
                        "{prop_name} property not found on UpdateAttribute-tag-retries on \
                         {version}; properties: {:?}",
                        detail.properties.iter().map(|(k, _)| k).collect::<Vec<_>>()
                    )
                });

            assert!(
                value.contains(needle),
                "{prop_name} must contain {needle} on {version}, got: {value:?}"
            );

            // The scanner must NOT return `None`/`Escaped` — these are
            // legitimate `#{name}` references and must drive the
            // cross-link annotation.
            let scan = scan_param_refs(value);
            assert!(
                !matches!(scan, ParamRefScan::None),
                "scan_param_refs must not be None for property {prop_name}={value:?} on {version}; \
                 got: {scan:?}"
            );
        }
    }
}
