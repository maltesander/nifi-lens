//! Versioned-flow fixture pipelines.
//!
//! Creates two PGs and commits them to the NiFi Registry:
//!  - `versioned-clean` — committed and untouched → `UP_TO_DATE`.
//!  - `versioned-modified` — committed, then a richer set of mutations
//!    applied after the fact → `LOCALLY_MODIFIED`. The mutations are
//!    chosen to surface multiple diff entry types in the
//!    version-control modal: PROPERTY_CHANGED on two different
//!    components, COMPONENT_ADDED for a new processor and a new
//!    connection, and COMPONENT_REMOVED for the original gen→log
//!    connection.
//!
//! Each PG has a tiny GenerateFlowFile → LogAttribute pipeline at
//! commit time. The topology is intentionally minimal — we only need
//! *something* in version control. The Tracer modal exercises the
//! diff, not the data.

use std::collections::HashMap;

use nifi_rust_client::dynamic::{DynamicClient, types};

use crate::entities::{make_pg, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::custom_text_property_key;
use crate::fixture::healthy::{create_connection_in_pg, create_processor};
use crate::fixture::registry::RegistryIds;

/// Identifiers captured from the initial gen → log pipeline of a
/// versioned PG so post-commit mutations can target each component
/// directly without re-resolving by name.
pub struct VersionedPgIds {
    pub pg_id: String,
    pub gen_id: String,
    /// Not yet consumed by the post-commit mutation block, but kept on
    /// the struct so future additions (e.g. mutating LogAttribute
    /// directly by id) don't have to re-resolve it by name.
    #[allow(dead_code)]
    pub log_id: String,
    pub conn_gen_to_log_id: String,
}

/// Seed the two versioned-flow PGs under `parent_pg_id`. Commits each
/// to the NiFi Registry, then applies a richer mutation block to
/// `versioned-modified` so it surfaces as `LOCALLY_MODIFIED` with a
/// diff covering PROPERTY_CHANGED, COMPONENT_ADDED, and
/// COMPONENT_REMOVED entries.
pub async fn seed(
    client: &DynamicClient,
    parent_pg_id: &str,
    registry_ids: &RegistryIds,
    detected_version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding versioned-clean and versioned-modified");

    let clean_ids = create_versioned_pg_with_pipeline(
        client,
        parent_pg_id,
        "versioned-clean",
        "vc-clean",
        detected_version,
    )
    .await?;
    commit_to_registry(
        client,
        &clean_ids.pg_id,
        registry_ids,
        "versioned-clean",
        "Initial commit",
    )
    .await?;
    // versioned-clean stays untouched after commit so it remains
    // UP_TO_DATE; the modal uses it to render the empty-diff state.

    let modified_ids = create_versioned_pg_with_pipeline(
        client,
        parent_pg_id,
        "versioned-modified",
        "vc-mod",
        detected_version,
    )
    .await?;
    commit_to_registry(
        client,
        &modified_ids.pg_id,
        registry_ids,
        "versioned-modified",
        "Initial commit",
    )
    .await?;

    // === versioned-modified post-commit mutations ===
    // Surfaces PROPERTY_CHANGED ×2 on different components,
    // COMPONENT_ADDED for a new processor and a new connection, and
    // COMPONENT_REMOVED for the original gen→log connection. Combined
    // this gives the modal a rich diff to display.

    // 1. Property change on LogAttribute: Log Level INFO → WARN.
    mutate_processor_property(
        client,
        &modified_ids.pg_id,
        "vc-mod-LogAttribute",
        "Log Level",
        "WARN",
    )
    .await?;

    // 2. Property change on GenerateFlowFile: Batch Size 1 → 5.
    mutate_processor_property(
        client,
        &modified_ids.pg_id,
        "vc-mod-GenerateFlowFile",
        "Batch Size",
        "5",
    )
    .await?;

    // 3. Add a new processor `vc-mod-UpdateAttribute-added`. Auto-
    //    terminate `success` so the new processor stays valid.
    let added_id = create_processor(
        client,
        &modified_ids.pg_id,
        make_processor(
            "vc-mod-UpdateAttribute-added",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[("post-commit-marker", "added-after-version-commit")]),
            None,
            vec!["success"],
        ),
        "vc-mod-UpdateAttribute-added",
    )
    .await?;

    // 4. Add a new connection from GenerateFlowFile (success) → the
    //    newly added processor. NiFi supports multiple outbound
    //    connections from the same relationship, so this coexists
    //    with the original gen→log connection until step 5 deletes
    //    the latter.
    create_connection_in_pg(
        client,
        &modified_ids.pg_id,
        &modified_ids.gen_id,
        "PROCESSOR",
        &added_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // 5. Delete the original gen → log connection so the diff also
    //    shows a COMPONENT_REMOVED Connection entry.
    delete_connection_by_id(client, &modified_ids.conn_gen_to_log_id).await?;

    Ok(())
}

/// Build a child PG containing a GenerateFlowFile → LogAttribute
/// pipeline. Returns the new PG's id along with the processor and
/// connection ids needed for downstream post-commit mutations.
async fn create_versioned_pg_with_pipeline(
    client: &DynamicClient,
    parent_pg_id: &str,
    pg_name: &str,
    component_prefix: &str,
    version: &semver::Version,
) -> Result<VersionedPgIds> {
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

    let conn_gen_to_log_id = create_connection_in_pg(
        client,
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &log_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    Ok(VersionedPgIds {
        pg_id,
        gen_id,
        log_id,
        conn_gen_to_log_id,
    })
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

/// Fetch a connection's current revision and delete it. Used to surface
/// a COMPONENT_REMOVED entry in the version-control diff.
async fn delete_connection_by_id(client: &DynamicClient, conn_id: &str) -> Result<()> {
    let entity = client
        .connections()
        .get_connection(conn_id)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("get connection {conn_id} for revision lookup before delete"),
            source: Box::new(e),
        })?;
    let version = entity
        .revision
        .as_ref()
        .and_then(|r| r.version)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("connection {conn_id} has no revision"),
        })?;
    let version_str = version.to_string();
    client
        .connections()
        .delete_connection(
            conn_id,
            Some(version_str.as_str()),
            Some("nifilens-fixture-seeder"),
            None,
        )
        .await
        .map_err(|e| SeederError::Api {
            message: format!("delete connection {conn_id}"),
            source: Box::new(e),
        })?;
    Ok(())
}
