//! Parameterized-pipeline fixture: exercises parameter context binding,
//! `#{name}` parameter references, and the `##{escape}` literal case.
//!
//! Topology:
//!
//! ```text
//! [fixture-pc-base]                         ← parameter context
//!   kafka_bootstrap = broker:9092
//!   retry_max = 3
//!   db_password = changeit (sensitive)
//!
//! [fixture-pc-prod]  inherits fixture-pc-base   ← parameter context
//!   retry_max = 5          (overrides base)
//!   region = eu-west-1
//!
//! parameterized-pipeline/   bound to fixture-pc-prod
//! ├── LogAttribute-parameterized
//! │     Log Payload  = "connecting to #{kafka_bootstrap}"
//! │     Log Prefix   = "##{literal_text}"   ← escape case (no annotation expected)
//! └── UpdateAttribute-parameterized
//!       broker       = "#{kafka_bootstrap}"   ← property-value cross-link
//!       max_retries  = "#{retry_max}"         ← property-value cross-link
//! ```
//!
//! The processors are intentionally not started — they have no incoming
//! connections, which is fine for the fixture's purpose of testing the
//! Browser parameter-context modal and `#{name}` cross-link annotations.

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::healthy::{create_child_pg, create_processor};

/// Seeded parameter context IDs, returned so callers can assert on them.
// T19 integration tests consume both fields; allow on the struct for now.
#[allow(dead_code)]
pub struct ParameterContextIds {
    pub base_id: String,
    pub prod_id: String,
}

/// Create `fixture-pc-base` and `fixture-pc-prod` (inheriting from base).
/// Returns the IDs of both contexts.
pub async fn seed_parameter_contexts(client: &DynamicClient) -> Result<ParameterContextIds> {
    tracing::info!("seeding fixture-pc-base parameter context");
    let base_id = create_parameter_context(
        client,
        "fixture-pc-base",
        "Base parameters shared across all environments",
        vec![
            make_param("kafka_bootstrap", "broker:9092", false),
            make_param("retry_max", "3", false),
            make_sensitive_param("db_password", "changeit"),
        ],
        vec![],
    )
    .await?;

    tracing::info!(%base_id, "seeding fixture-pc-prod parameter context");
    let base_ref = make_context_ref(&base_id);
    let prod_id = create_parameter_context(
        client,
        "fixture-pc-prod",
        "Production overrides — inherits from fixture-pc-base",
        vec![
            make_param("retry_max", "5", false),
            make_param("region", "eu-west-1", false),
        ],
        vec![base_ref],
    )
    .await?;

    tracing::info!(%prod_id, "parameter contexts seeded");
    Ok(ParameterContextIds { base_id, prod_id })
}

/// Create the `parameterized-pipeline` PG under `parent_pg_id`, bind it to
/// `fixture-pc-prod`, and add a `LogAttribute-parameterized` processor that
/// exercises `#{kafka_bootstrap}` (param ref) and `##{literal_text}` (escape).
pub async fn seed_parameterized_pipeline(
    client: &DynamicClient,
    parent_pg_id: &str,
    pc_ids: &ParameterContextIds,
    version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding parameterized-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "parameterized-pipeline").await?;

    // Bind the PG to fixture-pc-prod.  The binding requires a full GET+PUT
    // round-trip because `update_process_group` needs the current revision.
    bind_parameter_context(client, &pg_id, &pc_ids.prod_id).await?;

    // Add a LogAttribute processor that references the parameter context.
    // "Log Payload" key is stable across all supported NiFi versions.
    // "Log Prefix" was `Log prefix` in 2.6.0 and `Log Prefix` from 2.8.0;
    // see log_prefix_property_key() for the version-guarded selection.
    let prefix_key = log_prefix_property_key(version);
    let mut properties = std::collections::HashMap::new();
    properties.insert(
        "Log Payload".to_string(),
        "connecting to #{kafka_bootstrap}".to_string(),
    );
    properties.insert(prefix_key.to_string(), "##{literal_text}".to_string());
    properties.insert("Log Level".to_string(), "info".to_string());

    let mut config = types::ProcessorConfigDto::default();
    config.properties = Some(properties.into_iter().map(|(k, v)| (k, Some(v))).collect());
    config.auto_terminated_relationships = Some(vec!["success".to_string()]);

    let mut component = types::ProcessorDto::default();
    component.name = Some("LogAttribute-parameterized".to_string());
    component.r#type = Some("org.apache.nifi.processors.standard.LogAttribute".to_string());
    component.config = Some(config);

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut body = types::ProcessorEntity::default();
    body.component = Some(component);
    body.revision = Some(revision);

    let created = client
        .processgroups()
        .create_processor(&pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: "create LogAttribute-parameterized".into(),
            source: Box::new(e),
        })?;
    let proc_id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: "LogAttribute-parameterized has no id after create".into(),
        })?;

    tracing::info!(
        %proc_id,
        "LogAttribute-parameterized created"
    );

    // Add an UpdateAttribute processor with dynamic properties that use
    // parameter references.  This exercises the `#{name}` cross-link
    // annotation on property values in the Browser properties modal.
    //
    // Dynamic properties on UpdateAttribute are user-defined; they are set
    // the same way as any other property (no special API call needed).
    let ua_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-parameterized",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("broker", "#{kafka_bootstrap}"),
                ("max_retries", "#{retry_max}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-parameterized",
    )
    .await?;

    tracing::info!(
        %ua_id,
        "parameterized-pipeline seeded (processors not started — no connections)"
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Private helpers
// ---------------------------------------------------------------------------

/// The property key for LogAttribute's "Log Prefix" differs in casing between
/// NiFi 2.6.0 and 2.8.0. We gate on the minor version conservatively.
///
/// - `2.6.x` uses `"Log prefix"` (lower-case p)
/// - `2.8.0+` uses `"Log Prefix"` (upper-case P)
pub(crate) fn log_prefix_property_key(version: &semver::Version) -> &'static str {
    if version.major < 2 || (version.major == 2 && version.minor < 8) {
        "Log prefix"
    } else {
        "Log Prefix"
    }
}

/// POST a new parameter context and return its ID.
async fn create_parameter_context(
    client: &DynamicClient,
    name: &str,
    description: &str,
    parameters: Vec<types::ParameterEntity>,
    inherited_parameter_contexts: Vec<types::ParameterContextReferenceEntity>,
) -> Result<String> {
    let mut dto = types::ParameterContextDto::default();
    dto.name = Some(name.to_string());
    dto.description = Some(description.to_string());
    dto.parameters = Some(parameters);
    if !inherited_parameter_contexts.is_empty() {
        dto.inherited_parameter_contexts = Some(inherited_parameter_contexts);
    }

    let mut revision = types::RevisionDto::default();
    revision.version = Some(0);

    let mut entity = types::ParameterContextEntity::default();
    entity.component = Some(dto);
    entity.revision = Some(revision);

    let created = client
        .parametercontexts()
        .create_parameter_context(&entity)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create parameter context {name}"),
            source: Box::new(e),
        })?;

    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("parameter context {name} has no id after create"),
        })
}

/// Build a plain (non-sensitive) `ParameterEntity`.
fn make_param(name: &str, value: &str, sensitive: bool) -> types::ParameterEntity {
    let mut dto = types::ParameterDto::default();
    dto.name = Some(name.to_string());
    dto.value = Some(value.to_string());
    dto.sensitive = Some(sensitive);

    let mut entity = types::ParameterEntity::default();
    entity.parameter = Some(dto);
    entity
}

/// Build a sensitive `ParameterEntity`.
fn make_sensitive_param(name: &str, value: &str) -> types::ParameterEntity {
    make_param(name, value, true)
}

/// Build a `ParameterContextReferenceEntity` pointing to `context_id`.
fn make_context_ref(context_id: &str) -> types::ParameterContextReferenceEntity {
    let mut dto = types::ParameterContextReferenceDto::default();
    dto.id = Some(context_id.to_string());

    let mut entity = types::ParameterContextReferenceEntity::default();
    entity.id = Some(context_id.to_string());
    entity.component = Some(dto);
    entity
}

/// GET the current PG entity and PUT it back with the parameter context bound.
async fn bind_parameter_context(
    client: &DynamicClient,
    pg_id: &str,
    context_id: &str,
) -> Result<()> {
    // GET to obtain the current revision.
    let current = client
        .processgroups()
        .get_process_group(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("get process group {pg_id} for parameter context binding"),
            source: Box::new(e),
        })?;

    let revision = current
        .revision
        .clone()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("process group {pg_id} has no revision"),
        })?;

    // Rebuild a minimal entity with the parameter context binding.
    let mut component = types::ProcessGroupDto::default();
    component.id = current.component.as_ref().and_then(|c| c.id.clone());
    component.name = current.component.as_ref().and_then(|c| c.name.clone());
    component.parameter_context = Some(make_context_ref(context_id));

    let mut entity = types::ProcessGroupEntity::default();
    entity.id = current.id.clone();
    entity.revision = Some(revision);
    entity.component = Some(component);

    client
        .processgroups()
        .update_process_group(pg_id, &entity)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("bind parameter context {context_id} to process group {pg_id}"),
            source: Box::new(e),
        })?;

    tracing::debug!(%pg_id, %context_id, "parameter context bound to process group");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn log_prefix_key_uses_legacy_on_floor() {
        let v = semver::Version::parse("2.6.0").unwrap();
        assert_eq!(log_prefix_property_key(&v), "Log prefix");
    }

    #[test]
    fn log_prefix_key_uses_modern_from_2_8() {
        let v = semver::Version::parse("2.8.0").unwrap();
        assert_eq!(log_prefix_property_key(&v), "Log Prefix");
    }

    #[test]
    fn log_prefix_key_uses_modern_on_ceiling() {
        let v = semver::Version::parse("2.9.0").unwrap();
        assert_eq!(log_prefix_property_key(&v), "Log Prefix");
    }

    #[test]
    fn make_param_sets_fields() {
        let e = make_param("kafka_bootstrap", "broker:9092", false);
        let dto = e.parameter.unwrap();
        assert_eq!(dto.name.as_deref(), Some("kafka_bootstrap"));
        assert_eq!(dto.value.as_deref(), Some("broker:9092"));
        assert_eq!(dto.sensitive, Some(false));
    }

    #[test]
    fn make_sensitive_param_sets_sensitive_true() {
        let e = make_sensitive_param("db_password", "changeit");
        let dto = e.parameter.unwrap();
        assert_eq!(dto.sensitive, Some(true));
        assert_eq!(dto.value.as_deref(), Some("changeit"));
    }

    #[test]
    fn make_context_ref_sets_id() {
        let r = make_context_ref("ctx-123");
        assert_eq!(r.id.as_deref(), Some("ctx-123"));
        assert_eq!(r.component.unwrap().id.as_deref(), Some("ctx-123"));
    }
}
