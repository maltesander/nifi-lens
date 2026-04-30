//! sink-us/ — bound to fixture-pc-region-us (chain depth 3).
//!
//! Stages:
//!   in-us (input port)
//!   UpdateAttribute-tag-region (compliance = #{compliance_tag}, region = #{region_filter})
//!   ConvertRecord (JSON -> Parquet)        — uses scoped Parquet CSes
//!   UpdateRecord-parquet-tag (adds audit_id; CONTENT_MODIFIED — Parquet↔Parquet diff)
//!   LogAttribute-INFO

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_in_pg, create_input_port, create_processor, start_processor,
    wait_for_valid,
};
use crate::fixture::parameter_contexts::{self, OrdersContextIds};
use crate::fixture::services::create_and_enable_cs_inline;

pub struct SinkUsIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    client: &DynamicClient,
    orders_pg_id: &str,
    contexts: &OrdersContextIds,
) -> Result<SinkUsIds> {
    tracing::info!("seeding orders-pipeline/sink-us");

    let pg_id = create_child_pg(client, orders_pg_id, "sink-us").await?;
    parameter_contexts::bind(client, &pg_id, &contexts.region_us_id).await?;

    // Scoped Parquet reader/writer + JSON reader.
    let json_reader_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-json-reader-us",
        "org.apache.nifi.json.JsonTreeReader",
    )
    .await?;
    let parquet_reader_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-parquet-reader",
        "org.apache.nifi.parquet.ParquetReader",
    )
    .await?;
    let parquet_writer_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-parquet-writer",
        "org.apache.nifi.parquet.ParquetRecordSetWriter",
    )
    .await?;

    let in_port_id = create_input_port(client, &pg_id, "in-us").await?;

    let tag_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-tag-region",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("compliance", "#{compliance_tag}"),
                ("region", "#{region_filter}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-tag-region",
    )
    .await?;

    let convert_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ConvertRecord",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &json_reader_id),
                ("Record Writer", &parquet_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord",
    )
    .await?;

    let upd_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-parquet-tag",
            "org.apache.nifi.processors.standard.UpdateRecord",
            // Overwrite the existing /status field with a literal
            // "AUDITED-PARQUET" tag. We mutate an existing schema
            // field rather than adding a new one (e.g. /audit_id)
            // because ParquetRecordSetWriter inherits the reader's
            // schema and silently drops new fields — the output
            // would be byte-equal to input and produce nothing for
            // the diff modal. UpdateRecord's default replacement
            // strategy is "Literal Value", which means EL-with-field-
            // reference (`${field.value:append(...)}`) silently no-ops;
            // a plain literal forces the rewrite.
            props(&[
                ("Record Reader", &parquet_reader_id),
                ("Record Writer", &parquet_writer_id),
                ("/status", "AUDITED-PARQUET"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-parquet-tag",
    )
    .await?;

    let log_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-INFO",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-INFO",
    )
    .await?;

    create_connection_in_pg(
        client,
        &pg_id,
        &in_port_id,
        "INPUT_PORT",
        &tag_id,
        "PROCESSOR",
        vec![],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &tag_id,
        "PROCESSOR",
        &convert_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &convert_id,
        "PROCESSOR",
        &upd_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &upd_id,
        "PROCESSOR",
        &log_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    for (id, name) in [
        (&log_id, "LogAttribute-INFO"),
        (&upd_id, "UpdateRecord-parquet-tag"),
        (&convert_id, "ConvertRecord"),
        (&tag_id, "UpdateAttribute-tag-region"),
    ] {
        wait_for_valid(client, id, name).await?;
        start_processor(client, id).await?;
    }
    // in-us input port intentionally not started here — orders::seed
    // wires the parent-level connection and starts border ports.

    Ok(SinkUsIds { pg_id, in_port_id })
}
