//! Per-PG version-control fetches. Used by the cluster-store
//! `VersionControl` fetcher (fan-out via `version_information_batch`)
//! and the Browser version-control modal worker
//! (`version_information` + `local_modifications`).

use crate::client::{NifiClient, classify_or_fallback};
use crate::cluster::snapshot::{VersionControlMap, VersionControlSummary};
use crate::error::NifiLensError;

/// Post-processed shape of `FlowComparisonEntity`. Sections are sorted by
/// `(component_type, component_name)` for stable rendering.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowComparisonGrouped {
    pub sections: Vec<ComponentDiffSection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ComponentDiffSection {
    pub component_id: String,
    pub component_name: String,
    pub component_type: String,
    /// User-facing label for the section header. For Processors / CS /
    /// Ports this is `component_name` (or `"(unnamed)"` when the wire
    /// payload has no name). For Connections, the
    /// `apply_version_control_modal_loaded` reducer rewrites this to
    /// `"{source_name} → {destination_name}"` after resolving the
    /// connection in the live Browser arena. Pre-populated to
    /// `component_name` here; the reducer overrides for Connections.
    pub display_label: String,
    pub differences: Vec<RenderedDifference>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderedDifference {
    pub kind: String,
    pub description: String,
    pub environmental: bool,
}

impl NifiClient {
    /// Fetch identity + state for one PG. Errors when the request fails
    /// for any reason, including "PG not under version control" (returns
    /// `Err` so callers expecting a versioned PG fail loudly). Use
    /// `version_information_optional` to distinguish "absent" from "fetch
    /// error".
    pub async fn version_information(
        &self,
        pg_id: &str,
    ) -> Result<VersionControlSummary, NifiLensError> {
        match self.version_information_optional(pg_id).await? {
            Some(summary) => Ok(summary),
            None => Err(NifiLensError::VersionInformationFailed {
                context: self.context_name().to_string(),
                pg_id: pg_id.to_string(),
                source: Box::<dyn std::error::Error + Send + Sync>::from(
                    "process group is not under version control",
                ),
            }),
        }
    }

    /// Fetch identity + state for one PG, returning `Ok(None)` when the
    /// PG is not under version control (NiFi returns 200 with a null
    /// `versionControlInformation` payload, or 404 on some versions).
    pub async fn version_information_optional(
        &self,
        pg_id: &str,
    ) -> Result<Option<VersionControlSummary>, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            pg_id,
            "fetching /versions/process-groups/{pg_id}"
        );
        let entity = match self.inner.versions().get_version_information(pg_id).await {
            Ok(e) => e,
            Err(err) if is_not_found(&err) => return Ok(None),
            Err(err) => {
                return Err(classify_or_fallback(
                    self.context_name(),
                    Box::new(err),
                    |source| NifiLensError::VersionInformationFailed {
                        context: self.context_name().to_string(),
                        pg_id: pg_id.to_string(),
                        source,
                    },
                ));
            }
        };
        Ok(entity
            .version_control_information
            .and_then(|dto| summary_from_dto(&dto)))
    }

    /// Fetch `process-groups/{id}/local-modifications`. Returns the
    /// flattened, sorted `FlowComparisonGrouped` ready for rendering.
    pub async fn local_modifications(
        &self,
        pg_id: &str,
    ) -> Result<FlowComparisonGrouped, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            pg_id,
            "fetching /process-groups/{pg_id}/local-modifications"
        );
        let entity = self
            .inner
            .processgroups()
            .get_local_modifications(pg_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LocalModificationsFailed {
                        context: self.context_name().to_string(),
                        pg_id: pg_id.to_string(),
                        source,
                    }
                })
            })?;

        let mut sections: Vec<ComponentDiffSection> = entity
            .component_differences
            .unwrap_or_default()
            .into_iter()
            .map(|cd| ComponentDiffSection {
                component_id: cd.component_id.unwrap_or_default(),
                component_name: cd.component_name.clone().unwrap_or_default(),
                component_type: cd.component_type.unwrap_or_default(),
                display_label: cd.component_name.unwrap_or_default(),
                differences: cd
                    .differences
                    .unwrap_or_default()
                    .into_iter()
                    .map(|d| RenderedDifference {
                        kind: d.difference_type.unwrap_or_default(),
                        description: d.difference.unwrap_or_default(),
                        environmental: d.environmental.unwrap_or(false),
                    })
                    .collect(),
            })
            .collect();

        sections.sort_by(|a, b| {
            a.component_type
                .cmp(&b.component_type)
                .then_with(|| a.component_name.cmp(&b.component_name))
                .then_with(|| a.component_id.cmp(&b.component_id))
        });

        Ok(FlowComparisonGrouped { sections })
    }

    /// Fan-out batch fetch used by the cluster-store fetcher. Returns a
    /// `VersionControlMap` containing only PGs that ARE under version
    /// control. Per-PG errors are logged at `warn!` and the PG is
    /// omitted from the map (a transient per-PG failure should degrade
    /// one row, not poison the whole tick).
    ///
    /// `concurrency` caps the number of concurrent in-flight HTTP
    /// requests via `futures::stream::buffer_unordered`. A value of `0`
    /// is clamped to `1` to keep at least one request in flight.
    pub async fn version_information_batch(
        &self,
        pg_ids: &[String],
        concurrency: usize,
    ) -> VersionControlMap {
        use futures::stream::{self, StreamExt};
        let context = self.context_name().to_string();
        let mut map = VersionControlMap::default();
        let concurrency = concurrency.max(1);
        let mut stream = stream::iter(pg_ids.iter().map(|pg_id| {
            let pg_id = pg_id.clone();
            async move {
                let res = self.version_information_optional(&pg_id).await;
                (pg_id, res)
            }
        }))
        .buffer_unordered(concurrency);
        while let Some((pg_id, res)) = stream.next().await {
            match res {
                Ok(Some(summary)) => {
                    map.by_pg_id.insert(pg_id, summary);
                }
                Ok(None) => {
                    // PG is not under version control; absent from the map.
                }
                Err(err) => {
                    tracing::warn!(
                        context = %context,
                        pg_id = %pg_id,
                        error = %err,
                        "version_information batch: per-PG error, omitting from snapshot"
                    );
                }
            }
        }
        map
    }
}

fn summary_from_dto(
    dto: &nifi_rust_client::dynamic::types::VersionControlInformationDto,
) -> Option<VersionControlSummary> {
    use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
    let state_str = dto.state.as_deref()?;
    let state = VersionControlInformationDtoState::from_wire(state_str)?;
    Some(VersionControlSummary {
        state,
        registry_name: dto.registry_name.clone(),
        bucket_name: dto.bucket_name.clone(),
        branch: dto.branch.clone(),
        flow_id: dto.flow_id.clone(),
        flow_name: dto.flow_name.clone(),
        version: dto.version.clone(),
        state_explanation: dto.state_explanation.clone(),
    })
}

fn is_not_found(err: &nifi_rust_client::NifiError) -> bool {
    matches!(err, nifi_rust_client::NifiError::NotFound { .. })
}

#[cfg(test)]
mod batch_concurrency_tests {
    //! Validates the `futures::stream::buffer_unordered` primitive that
    //! all three fanout fetchers (version_information_batch,
    //! parameter_context_bindings_batch, run_parallel) compose.
    //! Stub-future based — no HTTP, no client.

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    use futures::stream::{self, StreamExt};

    #[tokio::test]
    async fn buffer_unordered_caps_in_flight_at_4() {
        let in_flight = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let inputs: Vec<u32> = (0..50).collect();

        let in_flight_c = in_flight.clone();
        let max_seen_c = max_seen.clone();

        let stream = stream::iter(inputs.into_iter().map(move |i| {
            let in_flight = in_flight_c.clone();
            let max_seen = max_seen_c.clone();
            async move {
                let cur = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                let prev_max = max_seen.load(Ordering::Relaxed);
                if cur > prev_max {
                    max_seen.store(cur, Ordering::Relaxed);
                }
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                in_flight.fetch_sub(1, Ordering::Relaxed);
                i
            }
        }))
        .buffer_unordered(4);

        let _: Vec<u32> = stream.collect().await;
        let max = max_seen.load(Ordering::Relaxed);
        assert!(max <= 4, "max_seen = {max} exceeds cap of 4");
        assert!(
            max >= 2,
            "concurrency should actually fire — max_seen = {max}"
        );
    }
}
