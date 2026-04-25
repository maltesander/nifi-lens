//! Versioned-flow fixture pipelines.
//!
//! Creates two PGs and commits them to the NiFi Registry:
//!  - `versioned-clean` — committed and untouched → `UP_TO_DATE`.
//!  - `versioned-modified` — committed, then a single property mutated
//!    after the fact → `LOCALLY_MODIFIED`.
//!
//! Each PG has a tiny GenerateFlowFile → LogAttribute pipeline. The
//! topology is intentionally minimal — we only need *something* in
//! version control. The Tracer modal exercises the diff, not the data.

use std::collections::HashMap;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_pg, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::custom_text_property_key;
use crate::fixture::healthy::{create_connection_in_pg, create_processor};
use crate::fixture::registry::RegistryIds;

/// Seed the two versioned-flow PGs under `parent_pg_id`. Commits each to
/// the NiFi Registry, then mutates one property on `versioned-modified`
/// so it surfaces as `LOCALLY_MODIFIED`.
pub async fn seed(
    client: &DynamicClient,
    parent_pg_id: &str,
    registry_ids: &RegistryIds,
    detected_version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding versioned-clean and versioned-modified");

    let clean_pg_id = create_versioned_pg_with_pipeline(
        client,
        parent_pg_id,
        "versioned-clean",
        "vc-clean",
        detected_version,
    )
    .await?;
    commit_to_registry(
        client,
        &clean_pg_id,
        registry_ids,
        "versioned-clean",
        "Initial commit",
    )
    .await?;

    let modified_pg_id = create_versioned_pg_with_pipeline(
        client,
        parent_pg_id,
        "versioned-modified",
        "vc-mod",
        detected_version,
    )
    .await?;
    commit_to_registry(
        client,
        &modified_pg_id,
        registry_ids,
        "versioned-modified",
        "Initial commit",
    )
    .await?;

    // Mutate the LogAttribute processor's "Log Level" so the PG
    // surfaces as LOCALLY_MODIFIED on the next /versions/process-groups
    // /{id} fetch.
    let log_attr_name = "vc-mod-LogAttribute";
    mutate_processor_property(client, &modified_pg_id, log_attr_name, "Log Level", "WARN").await?;

    Ok(())
}

/// Build a child PG containing a GenerateFlowFile → LogAttribute
/// pipeline. Returns the new PG's id.
async fn create_versioned_pg_with_pipeline(
    client: &DynamicClient,
    parent_pg_id: &str,
    pg_name: &str,
    component_prefix: &str,
    version: &semver::Version,
) -> Result<String> {
    // Create the PG.
    let body = make_pg(pg_name);
    let created = client
        .processgroups()
        .create_process_group(parent_pg_id, None, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create PG {pg_name}"),
            source: Box::new(e),
        })?;
    let pg_id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("created PG {pg_name} has no id"),
        })?;

    // GenerateFlowFile → LogAttribute pipeline.
    let gen_name = format!("{component_prefix}-GenerateFlowFile");
    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            &gen_name,
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[
                (custom_text_property_key(version), "versioned fixture flow"),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("10 sec"),
            vec![],
        ),
        &gen_name,
    )
    .await?;

    let log_name = format!("{component_prefix}-LogAttribute");
    let log_id = create_processor(
        client,
        &pg_id,
        make_processor(
            &log_name,
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "INFO")]),
            None,
            vec!["success"],
        ),
        &log_name,
    )
    .await?;

    create_connection_in_pg(
        client,
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &log_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    Ok(pg_id)
}

/// Commit a PG to the NiFi Registry under `flow_name`.
///
/// The PG's revision is fetched fresh before commit — creating
/// processors and connections inside the PG bumps its revision past 0,
/// and NiFi 2.6.0 strictly enforces revision match (returns 400 "not
/// the most up-to-date revision" otherwise). 2.9.0 is more lenient,
/// but we always send the truthful value.
async fn commit_to_registry(
    client: &DynamicClient,
    pg_id: &str,
    registry_ids: &RegistryIds,
    flow_name: &str,
    comments: &str,
) -> Result<()> {
    // Fetch the PG's current revision — creating processors/connections
    // inside the PG has bumped it past 0.
    let pg_entity = client
        .processgroups()
        .get_process_group(pg_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("get PG {pg_id} for revision lookup before commit"),
            source: Box::new(e),
        })?;
    let revision = pg_entity
        .revision
        .clone()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("PG {pg_id} has no revision"),
        })?;

    let mut versioned_flow = types::VersionedFlowDto::default();
    versioned_flow.action = Some("COMMIT".to_string());
    versioned_flow.registry_id = Some(registry_ids.client_id.clone());
    versioned_flow.bucket_id = Some(registry_ids.bucket_id.clone());
    versioned_flow.flow_name = Some(flow_name.to_string());
    versioned_flow.description = Some("nifilens fixture: versioned PG".to_string());
    versioned_flow.comments = Some(comments.to_string());

    let mut body = types::StartVersionControlRequestEntity::default();
    body.process_group_revision = Some(revision);
    body.versioned_flow = Some(versioned_flow);

    client
        .versions()
        .save_to_flow_registry(pg_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("save_to_flow_registry pg={pg_id} flow={flow_name}"),
            source: Box::new(e),
        })?;

    Ok(())
}

/// Find the named processor inside `pg_id`, then PUT a single-property
/// update so the parent PG diverges from its committed flow.
async fn mutate_processor_property(
    client: &DynamicClient,
    pg_id: &str,
    processor_name: &str,
    property_key: &str,
    new_value: &str,
) -> Result<()> {
    // List processors in the PG (no descendants).
    let listing = client
        .processgroups()
        .get_processors(pg_id, Some(false))
        .await
        .map_err(|e| SeederError::Api {
            message: format!("list processors in pg={pg_id}"),
            source: Box::new(e),
        })?;

    let processor = listing
        .processors
        .as_ref()
        .and_then(|list| {
            list.iter().find(|p| {
                p.component
                    .as_ref()
                    .and_then(|c| c.name.as_ref())
                    .map(|n| n == processor_name)
                    .unwrap_or(false)
            })
        })
        .ok_or_else(|| SeederError::Invariant {
            message: format!("processor {processor_name} not found in pg={pg_id}"),
        })?;

    let processor_id = processor
        .component
        .as_ref()
        .and_then(|c| c.id.clone())
        .or_else(|| processor.id.clone())
        .ok_or_else(|| SeederError::Invariant {
            message: format!("processor {processor_name} has no id"),
        })?;
    let revision = processor
        .revision
        .clone()
        .ok_or_else(|| SeederError::Invariant {
            message: format!("processor {processor_name} has no revision"),
        })?;

    // Build the update body with just the changed property.
    let mut properties: HashMap<String, Option<String>> = HashMap::new();
    properties.insert(property_key.to_string(), Some(new_value.to_string()));
    let mut config = types::ProcessorConfigDto::default();
    config.properties = Some(properties);

    let mut dto = types::ProcessorDto::default();
    dto.id = Some(processor_id.clone());
    dto.config = Some(config);

    let mut body = types::ProcessorEntity::default();
    body.id = Some(processor_id.clone());
    body.component = Some(dto);
    body.revision = Some(revision);

    client
        .processors()
        .update_processor(&processor_id, &body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("update processor {processor_name}"),
            source: Box::new(e),
        })?;

    Ok(())
}
