//! Parameter-context client helpers.
//!
//! # Batch binding fetch (cluster-store use)
//!
//! `parameter_context_bindings_batch` fans out `GET /process-groups/{id}`
//! per PG and extracts the bound `ParameterContextReferenceEntity`. Per-PG
//! failures are logged at `warn!` and the failing PG is omitted from the
//! result. The function always succeeds at the outer level.
//!
//! # Chain-fetch (modal worker use)
//!
//! `fetch_chain` resolves the full inheritance chain for a bound parameter
//! context. Starting from `bound_context_id`, it performs a BFS over
//! `inherited_parameter_contexts`, fetching each node in parallel per
//! frontier level. Cycle protection is enforced via a `visited` set and a
//! hard depth cap (`MAX_CHAIN_DEPTH`). The bound context failing to fetch
//! yields `ChainFetchResult::BoundFailed`; ancestor fetch failures are
//! logged and represented as error nodes in the chain.

use std::collections::HashSet;
use std::sync::Arc;

use futures::future::join_all;
use tokio::sync::RwLock;

use crate::client::NifiClient;
use crate::cluster::snapshot::{ParameterContextBindingsMap, ParameterContextRef};

impl NifiClient {
    /// Fan-out batch fetch used by the cluster-store
    /// `ParameterContextBindings` fetcher. For every `pg_id`, calls
    /// `GET /process-groups/{id}` and extracts the
    /// `parameterContext` binding (or `None` when the PG has no bound
    /// context). Per-PG failures are logged at `warn!` and the PG is
    /// omitted from the resulting map. Always succeeds at the outer level.
    pub async fn parameter_context_bindings_batch(
        &self,
        pg_ids: &[String],
    ) -> ParameterContextBindingsMap {
        let context = self.context_name().to_string();
        let futs = pg_ids.iter().map(|id| {
            let pg_id = id.clone();
            async move {
                let res = self.inner.processgroups().get_process_group(&pg_id).await;
                (pg_id, res)
            }
        });
        let results = join_all(futs).await;

        let mut by_pg_id = std::collections::BTreeMap::new();
        for (pg_id, res) in results {
            match res {
                Ok(entity) => {
                    // `parameterContext` sits directly on `ProcessGroupEntity`,
                    // not inside `component`.
                    let binding = entity.parameter_context.as_ref().and_then(|pc_ref| {
                        let id = pc_ref.id.clone()?;
                        let name = pc_ref
                            .component
                            .as_ref()
                            .and_then(|c| c.name.clone())
                            .unwrap_or_else(|| id.clone());
                        Some(ParameterContextRef { id, name })
                    });
                    by_pg_id.insert(pg_id, binding);
                }
                Err(err) => {
                    tracing::warn!(
                        context = %context,
                        pg_id = %pg_id,
                        error = %err,
                        "parameter_context_bindings: per-PG fetch failed"
                    );
                }
            }
        }
        ParameterContextBindingsMap { by_pg_id }
    }
}

// ─── Chain-fetch ──────────────────────────────────────────────────────────────

/// A single resolved parameter with its metadata.
///
/// Sensitive parameters have `value` set to `None`; the caller must never
/// attempt to display or log the raw value for such entries.
#[derive(Debug, Clone, PartialEq)]
pub struct ParameterEntry {
    /// The parameter name.
    pub name: String,
    /// The plain-text value, or `None` when the parameter is sensitive.
    pub value: Option<String>,
    /// Optional description.
    pub description: Option<String>,
    /// Whether the parameter is marked sensitive in NiFi.
    pub sensitive: bool,
    /// Whether the parameter was sourced from a `ParameterProvider`.
    pub provided: bool,
}

/// One node in a resolved parameter-context inheritance chain.
///
/// Nodes are ordered by BFS discovery: the directly-bound context is first,
/// followed by each level of `inherited_parameter_contexts`. Within a level
/// the order matches the order NiFi returned.
#[derive(Debug, Clone)]
pub struct ParameterContextNode {
    /// The NiFi-assigned id for this context.
    pub id: String,
    /// The display name of this context.
    pub name: String,
    /// Parameters declared directly in this context (not from ancestors).
    pub parameters: Vec<ParameterEntry>,
    /// Ordered list of ids for directly-inherited contexts (BFS children).
    pub inherited_ids: Vec<String>,
    /// Non-`None` when this node could not be fetched; the value is the
    /// error message. `parameters` and `inherited_ids` will be empty.
    pub fetch_error: Option<String>,
}

/// Outcome of a `fetch_chain` call.
#[derive(Debug)]
pub enum ChainFetchResult {
    /// All reachable nodes were fetched (some may carry a `fetch_error`).
    Loaded(Vec<ParameterContextNode>),
    /// The directly-bound context itself failed to fetch.
    BoundFailed(String),
}

/// Maximum BFS depth when resolving an inheritance chain. Guards against
/// cycles that slip past the `visited` set on concurrent mutations, and
/// keeps worst-case fetches bounded.
const MAX_CHAIN_DEPTH: usize = 16;

/// Resolves the full inheritance chain for a parameter context by id.
///
/// Starts from `bound_context_id`, then follows `inherited_parameter_contexts`
/// level by level (BFS), fetching each level in parallel. Deduplicates by id.
/// Returns `ChainFetchResult::BoundFailed` if the root context cannot be
/// fetched; ancestor failures are logged and emitted as error nodes.
pub async fn fetch_chain(
    client: Arc<RwLock<NifiClient>>,
    bound_context_id: &str,
) -> ChainFetchResult {
    let mut visited: HashSet<String> = HashSet::new();
    let mut nodes: Vec<ParameterContextNode> = Vec::new();
    let mut frontier: Vec<String> = vec![bound_context_id.to_string()];
    let mut depth = 0;

    while !frontier.is_empty() {
        if depth >= MAX_CHAIN_DEPTH {
            tracing::warn!(depth = depth, "parameter_context chain depth cap hit");
            break;
        }
        depth += 1;

        let unique: Vec<String> = frontier
            .into_iter()
            .filter(|id| visited.insert(id.clone()))
            .collect();

        let fetches = unique.iter().map(|id| {
            let client = client.clone();
            let id = id.clone();
            async move {
                let guard = client.read().await;
                let res = guard
                    .inner
                    .parametercontexts()
                    .get_parameter_context(&id, None)
                    .await;
                (id, res)
            }
        });
        let results = join_all(fetches).await;

        let mut next_frontier: Vec<String> = Vec::new();
        for (id, res) in results {
            match res {
                Ok(entity) => {
                    let component = entity.component;
                    let name = component
                        .as_ref()
                        .and_then(|c| c.name.clone())
                        .unwrap_or_else(|| id.clone());

                    let parameters: Vec<ParameterEntry> = component
                        .as_ref()
                        .and_then(|c| c.parameters.as_ref())
                        .map(|ps| {
                            ps.iter()
                                .filter_map(|pe| pe.parameter.as_ref())
                                .map(|p| {
                                    let sensitive = p.sensitive.unwrap_or(false);
                                    ParameterEntry {
                                        name: p.name.clone().unwrap_or_default(),
                                        value: if sensitive { None } else { p.value.clone() },
                                        description: p.description.clone(),
                                        sensitive,
                                        provided: p.provided.unwrap_or(false),
                                    }
                                })
                                .collect()
                        })
                        .unwrap_or_default();

                    let inherited_ids: Vec<String> = component
                        .as_ref()
                        .and_then(|c| c.inherited_parameter_contexts.as_ref())
                        .map(|v| v.iter().filter_map(|r| r.id.clone()).collect())
                        .unwrap_or_default();

                    for inh in &inherited_ids {
                        if !visited.contains(inh) {
                            next_frontier.push(inh.clone());
                        }
                    }
                    nodes.push(ParameterContextNode {
                        id,
                        name,
                        parameters,
                        inherited_ids,
                        fetch_error: None,
                    });
                }
                Err(err) => {
                    if id == bound_context_id {
                        return ChainFetchResult::BoundFailed(err.to_string());
                    }
                    tracing::warn!(
                        ctx_id = %id,
                        error = %err,
                        "parameter_context chain: per-node fetch failed"
                    );
                    nodes.push(ParameterContextNode {
                        id: id.clone(),
                        name: id,
                        parameters: vec![],
                        inherited_ids: vec![],
                        fetch_error: Some(err.to_string()),
                    });
                }
            }
        }

        frontier = next_frontier;
    }

    ChainFetchResult::Loaded(nodes)
}

// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod parameter_context_bindings_batch_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a `NifiClient` pointed at the wiremock server. Mounts a
    /// `/nifi-api/flow/about` stub (version 2.6.0) so `detect_version`
    /// succeeds, mirroring the pattern used in `client::tracer`.
    async fn test_client(server: &MockServer) -> NifiClient {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"about": {"version": "2.6.0", "title": "NiFi"}}),
                ),
            )
            .mount(server)
            .await;

        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic client");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        NifiClient::from_parts(inner, "test", version)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn batch_returns_map_for_pgs_with_and_without_context() {
        let server = MockServer::start().await;

        // PG with a bound parameter context.
        Mock::given(method("GET"))
            .and(path("/nifi-api/process-groups/pg-with"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pg-with",
                "parameterContext": {
                    "id": "ctx-1",
                    "component": { "id": "ctx-1", "name": "ctx-prod" }
                }
            })))
            .mount(&server)
            .await;

        // PG without a bound context.
        Mock::given(method("GET"))
            .and(path("/nifi-api/process-groups/pg-without"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pg-without",
                "component": { "id": "pg-without", "name": "no-pc" }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let pg_ids = vec!["pg-with".to_string(), "pg-without".to_string()];
        let map = client.parameter_context_bindings_batch(&pg_ids).await;

        assert_eq!(map.by_pg_id.len(), 2);
        let with = map.by_pg_id.get("pg-with").expect("pg-with present");
        let with_ref = with.as_ref().expect("pg-with has a binding");
        assert_eq!(with_ref.id, "ctx-1");
        assert_eq!(with_ref.name, "ctx-prod");
        assert!(
            map.by_pg_id
                .get("pg-without")
                .expect("pg-without present")
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn batch_logs_and_skips_per_pg_failures() {
        let server = MockServer::start().await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/process-groups/pg-good"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "pg-good",
                "parameterContext": {
                    "id": "ctx-x",
                    "component": { "id": "ctx-x", "name": "x" }
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/process-groups/pg-bad"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let map = client
            .parameter_context_bindings_batch(&["pg-good".into(), "pg-bad".into()])
            .await;

        assert!(map.by_pg_id.contains_key("pg-good"));
        assert!(!map.by_pg_id.contains_key("pg-bad"));
    }
}

#[cfg(test)]
mod chain_fetch_tests {
    use super::*;
    use wiremock::matchers::{method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> Arc<RwLock<NifiClient>> {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(
                ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"about": {"version": "2.6.0", "title": "NiFi"}}),
                ),
            )
            .mount(server)
            .await;

        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic client");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        Arc::new(RwLock::new(NifiClient::from_parts(inner, "test", version)))
    }

    #[test]
    fn entry_helper_constructs_sensitive_value() {
        let p = ParameterEntry {
            name: "db.password".into(),
            value: None,
            description: None,
            sensitive: true,
            provided: false,
        };
        assert!(p.sensitive);
        assert!(p.value.is_none());
    }

    #[test]
    fn node_helper_constructs_with_inherited_ids() {
        let n = ParameterContextNode {
            id: "ctx".into(),
            name: "name".into(),
            parameters: vec![],
            inherited_ids: vec!["a".into(), "b".into()],
            fetch_error: None,
        };
        assert_eq!(n.inherited_ids.len(), 2);
        assert!(n.fetch_error.is_none());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_chain_loads_two_deep_chain() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        // ctx-prod: has retry_max=5, inherits ctx-base, and a sensitive
        // db_password parameter (value must not be returned by our helper).
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-prod"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-prod",
                "component": {
                    "id": "ctx-prod",
                    "name": "ctx-prod",
                    "parameters": [
                        { "parameter": { "name": "retry_max", "value": "5", "sensitive": false } },
                        { "parameter": { "name": "db_password", "value": "secret", "sensitive": true } }
                    ],
                    "inheritedParameterContexts": [
                        { "id": "ctx-base", "component": { "id": "ctx-base", "name": "ctx-base" } }
                    ]
                }
            })))
            .mount(&server)
            .await;

        // ctx-base: has retry_max=3 (shadowed by ctx-prod), db_password (also shadowed).
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-base"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-base",
                "component": {
                    "id": "ctx-base",
                    "name": "ctx-base",
                    "parameters": [
                        { "parameter": { "name": "retry_max", "value": "3", "sensitive": false } },
                        { "parameter": { "name": "db_password", "value": "base-secret", "sensitive": true } }
                    ],
                    "inheritedParameterContexts": []
                }
            })))
            .mount(&server)
            .await;

        let result = fetch_chain(client, "ctx-prod").await;
        let nodes = match result {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => panic!("expected Loaded, got BoundFailed: {e}"),
        };

        assert_eq!(nodes.len(), 2, "expected two nodes in the chain");

        // First node is the directly-bound context.
        let prod = &nodes[0];
        assert_eq!(prod.id, "ctx-prod");
        assert_eq!(prod.name, "ctx-prod");
        assert_eq!(prod.inherited_ids, vec!["ctx-base"]);
        assert!(prod.fetch_error.is_none());

        // retry_max is not sensitive — value is present.
        let retry = prod
            .parameters
            .iter()
            .find(|p| p.name == "retry_max")
            .expect("retry_max present");
        assert_eq!(retry.value.as_deref(), Some("5"));
        assert!(!retry.sensitive);

        // db_password is sensitive — value must be None.
        let pwd = prod
            .parameters
            .iter()
            .find(|p| p.name == "db_password")
            .expect("db_password present");
        assert!(pwd.value.is_none());
        assert!(pwd.sensitive);

        // Second node is the ancestor.
        let base = &nodes[1];
        assert_eq!(base.id, "ctx-base");
        assert!(base.fetch_error.is_none());
        assert!(base.inherited_ids.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_chain_returns_bound_failed_when_root_unreachable() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let result = fetch_chain(client, "ctx-missing").await;
        assert!(
            matches!(result, ChainFetchResult::BoundFailed(_)),
            "expected BoundFailed for unreachable root"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn fetch_chain_ancestor_failure_is_error_node() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;

        // Root succeeds, inherits ctx-broken.
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-root"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-root",
                "component": {
                    "id": "ctx-root",
                    "name": "ctx-root",
                    "parameters": [],
                    "inheritedParameterContexts": [
                        { "id": "ctx-broken", "component": { "id": "ctx-broken", "name": "ctx-broken" } }
                    ]
                }
            })))
            .mount(&server)
            .await;

        // Ancestor returns 500.
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-broken"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let result = fetch_chain(client, "ctx-root").await;
        let nodes = match result {
            ChainFetchResult::Loaded(n) => n,
            ChainFetchResult::BoundFailed(e) => panic!("expected Loaded, got BoundFailed: {e}"),
        };

        assert_eq!(nodes.len(), 2);
        let broken = nodes
            .iter()
            .find(|n| n.id == "ctx-broken")
            .expect("ctx-broken node");
        assert!(
            broken.fetch_error.is_some(),
            "ancestor failure should be an error node"
        );
    }
}
