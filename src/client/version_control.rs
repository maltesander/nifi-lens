//! Per-PG version-control fetches. Used by the cluster-store
//! `VersionControl` fetcher (fan-out via `version_information_batch`)
//! and the Browser version-control modal worker
//! (`version_information` + `local_modifications`).

use futures::future::join_all;

use crate::client::NifiClient;
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
                return Err(NifiLensError::VersionInformationFailed {
                    context: self.context_name().to_string(),
                    pg_id: pg_id.to_string(),
                    source: Box::new(err),
                });
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
            .map_err(|err| NifiLensError::LocalModificationsFailed {
                context: self.context_name().to_string(),
                pg_id: pg_id.to_string(),
                source: Box::new(err),
            })?;

        let mut sections: Vec<ComponentDiffSection> = entity
            .component_differences
            .unwrap_or_default()
            .into_iter()
            .map(|cd| ComponentDiffSection {
                component_id: cd.component_id.unwrap_or_default(),
                component_name: cd.component_name.unwrap_or_default(),
                component_type: cd.component_type.unwrap_or_default(),
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
    pub async fn version_information_batch(&self, pg_ids: &[String]) -> VersionControlMap {
        let context = self.context_name().to_string();
        let futs = pg_ids.iter().map(|id| {
            let pg_id = id.clone();
            async move {
                let res = self.version_information_optional(&pg_id).await;
                (pg_id, res)
            }
        });
        let results = join_all(futs).await;
        let mut map = VersionControlMap::default();
        for (pg_id, res) in results {
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
    let s = format!("{err:?}");
    s.contains("404") || s.contains("NotFound")
}
