//! Healthy-pipeline fixture: nested process groups, rich JSON ingest,
//! cross-PG connection, enrichment, INFO-level logging.
//!
//! Topology:
//!
//! ```text
//! healthy-pipeline/
//! ├── ingest/ (child PG)
//! │   ├── GenerateFlowFile (1 sec, Custom Text = HEALTHY_INGEST_CUSTOM_TEXT)
//! │   ├── UpdateAttribute-ingest
//! │   └── output port "ingest-out"
//! ├── enrich/ (child PG)
//! │   ├── input port "enrich-in"
//! │   ├── ConvertRecord (fixture-json-reader -> fixture-json-writer)
//! │   ├── UpdateAttribute-enrich
//! │   ├── UpdateAttribute-cleanup
//! │   └── LogAttribute-INFO
//! └── (parent-level connection: ingest/ingest-out -> enrich/enrich-in)
//! ```

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_between, create_connection_in_pg, create_input_port,
    create_output_port, create_processor, start_input_port, start_output_port, start_processor,
    wait_for_valid,
};
use crate::fixture::custom_text_property_key;
use crate::fixture::payload::HEALTHY_INGEST_CUSTOM_TEXT;
use crate::fixture::services::ServiceIds;

/// Build the complete healthy-pipeline topology under `parent_pg_id` and
/// start every component in it.
pub async fn seed(
    client: &DynamicClient,
    parent_pg_id: &str,
    service_ids: &ServiceIds,
    version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding healthy-pipeline");

    // Parent PG.
    let healthy_pg_id = create_child_pg(client, parent_pg_id, "healthy-pipeline").await?;

    // Ingest child PG.
    let ingest_pg_id = create_child_pg(client, &healthy_pg_id, "ingest").await?;
    let gen_id = create_processor(
        client,
        &ingest_pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            // NiFi 2.8.0 added a validation rule that rejects
            // (Custom Text + Unique FlowFiles=true). Setting it to
            // false satisfies both 2.6.0 and 2.8.0.
            props(&[
                (
                    custom_text_property_key(version),
                    HEALTHY_INGEST_CUSTOM_TEXT,
                ),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("1 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;
    let ingest_ua_id = create_processor(
        client,
        &ingest_pg_id,
        make_processor(
            "UpdateAttribute-ingest",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("stage", "ingest"),
                ("fixture.ingest.timestamp", "${now():toNumber()}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-ingest",
    )
    .await?;
    let ingest_out_id = create_output_port(client, &ingest_pg_id, "ingest-out").await?;

    create_connection_in_pg(
        client,
        &ingest_pg_id,
        &gen_id,
        "PROCESSOR",
        &ingest_ua_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &ingest_pg_id,
        &ingest_ua_id,
        "PROCESSOR",
        &ingest_out_id,
        "OUTPUT_PORT",
        vec!["success"],
    )
    .await?;

    // Enrich child PG.
    let enrich_pg_id = create_child_pg(client, &healthy_pg_id, "enrich").await?;
    let enrich_in_id = create_input_port(client, &enrich_pg_id, "enrich-in").await?;
    let convert_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "ConvertRecord",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &service_ids.json_reader_id),
                ("Record Writer", &service_ids.json_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord",
    )
    .await?;
    let enrich_ua_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "UpdateAttribute-enrich",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("stage", "enrich"),
                (
                    "severity",
                    "${random():mod(3):equals(0):ifElse('INFO','WARN')}",
                ),
                ("fixture.enrich.timestamp", "${now():toNumber()}"),
                ("fixture.tag", "synthetic-enriched"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-enrich",
    )
    .await?;
    let log_attr_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "LogAttribute-INFO",
            "org.apache.nifi.processors.standard.LogAttribute",
            // LogAttribute uses legacy display-name property keys in
            // NiFi 2.x. "Log Prefix" differs in capitalization between
            // 2.6.0 (`Log prefix`) and 2.8.0 (`Log Prefix`), so we omit
            // it — the default (empty prefix) is fine for the fixture.
            props(&[("Log Level", "info"), ("Log Payload", "true")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-INFO",
    )
    .await?;
    let cleanup_ua_id = create_processor(
        client,
        &enrich_pg_id,
        make_processor(
            "UpdateAttribute-cleanup",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            // Delete Attributes Expression is a regex over attribute
            // names. Escaping the dot is required.
            props(&[(
                "Delete Attributes Expression",
                "fixture\\.ingest\\.timestamp",
            )]),
            None,
            vec![],
        ),
        "UpdateAttribute-cleanup",
    )
    .await?;

    // enrich_in -> convert_record
    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &enrich_in_id,
        "INPUT_PORT",
        &convert_id,
        "PROCESSOR",
        vec![],
    )
    .await?;
    // convert_record -> enrich_ua (success)
    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &convert_id,
        "PROCESSOR",
        &enrich_ua_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    // enrich_ua -> cleanup_ua
    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &enrich_ua_id,
        "PROCESSOR",
        &cleanup_ua_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    // cleanup_ua -> log_attr
    create_connection_in_pg(
        client,
        &enrich_pg_id,
        &cleanup_ua_id,
        "PROCESSOR",
        &log_attr_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // Parent-level cross-PG connection from ingest-out -> enrich-in. The
    // connection lives on the healthy-pipeline PG, but each port's
    // `group_id` must point to the child PG that owns it.
    create_connection_between(
        client,
        &healthy_pg_id,
        &ingest_pg_id,
        &ingest_out_id,
        "OUTPUT_PORT",
        &enrich_pg_id,
        &enrich_in_id,
        "INPUT_PORT",
        vec![],
    )
    .await?;

    // Start everything. Downstream first so nothing backs up on startup.
    // Enrich first.
    wait_for_valid(client, &log_attr_id, "LogAttribute-INFO").await?;
    start_processor(client, &log_attr_id).await?;
    wait_for_valid(client, &cleanup_ua_id, "UpdateAttribute-cleanup").await?;
    start_processor(client, &cleanup_ua_id).await?;
    wait_for_valid(client, &enrich_ua_id, "UpdateAttribute-enrich").await?;
    start_processor(client, &enrich_ua_id).await?;
    wait_for_valid(client, &convert_id, "ConvertRecord").await?;
    start_processor(client, &convert_id).await?;
    start_input_port(client, &enrich_in_id).await?;

    // Ingest second.
    wait_for_valid(client, &ingest_ua_id, "UpdateAttribute-ingest").await?;
    start_processor(client, &ingest_ua_id).await?;
    start_output_port(client, &ingest_out_id).await?;
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("healthy-pipeline seeded and running");
    Ok(())
}
