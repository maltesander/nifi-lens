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
//! (Added by Task 8.)

use futures::future::join_all;

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
