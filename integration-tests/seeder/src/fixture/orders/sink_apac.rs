//! sink-apac/ — bound to fixture-pc-region-apac (chain depth 3).
//!
//! Stages:
//!   in-apac (input port)
//!   UpdateAttribute-tag-region
//!   ConvertRecord (JSON -> Avro)
//!   UpdateRecord-avro-tag (CONTENT_MODIFIED — Avro<->Avro diff)
//!   RemoteProcessGroup -> remote-targets/incoming-apac (NOT_TRANSMITTING)

use std::time::Duration;

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    create_child_pg, create_connection_in_pg, create_input_port, create_processor, start_processor,
    wait_for_valid,
};
use crate::fixture::orders::shared::{
    connect_processor_to_remote_input_port, create_remote_process_group,
    wait_for_remote_input_ports,
};
use crate::fixture::parameter_contexts::{self, OrdersContextIds};
use crate::fixture::services::create_and_enable_cs_inline;

pub struct SinkApacIds {
    pub pg_id: String,
    pub in_port_id: String,
}

pub async fn seed(
    client: &DynamicClient,
    orders_pg_id: &str,
    contexts: &OrdersContextIds,
    _incoming_apac_port_id: &str,
) -> Result<SinkApacIds> {
    tracing::info!("seeding orders-pipeline/sink-apac");

    let pg_id = create_child_pg(client, orders_pg_id, "sink-apac").await?;
    parameter_contexts::bind(client, &pg_id, &contexts.region_apac_id).await?;

    let json_reader_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-json-reader-apac",
        "org.apache.nifi.json.JsonTreeReader",
    )
    .await?;
    let avro_reader_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-avro-reader",
        "org.apache.nifi.avro.AvroReader",
    )
    .await?;
    let avro_writer_id = create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-avro-writer",
        "org.apache.nifi.avro.AvroRecordSetWriter",
    )
    .await?;

    let in_port_id = create_input_port(client, &pg_id, "in-apac").await?;

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
                ("Record Writer", &avro_writer_id),
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
            "UpdateRecord-avro-tag",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &avro_reader_id),
                ("Record Writer", &avro_writer_id),
                ("/audit_id", "${UUID()}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-avro-tag",
    )
    .await?;

    let rpg_id = create_remote_process_group(client, &pg_id, "rpg-apac").await?;
    wait_for_remote_input_ports(client, &rpg_id, Duration::from_secs(60)).await?;

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
    connect_processor_to_remote_input_port(client, &pg_id, &upd_id, &rpg_id, "incoming-apac")
        .await?;

    // RPG intentionally left at default (not transmitting) — exercises the
    // muted ■ glyph and NOT_TRANSMITTING status visualisation in tests.

    for (id, name) in [
        (&upd_id, "UpdateRecord-avro-tag"),
        (&convert_id, "ConvertRecord"),
        (&tag_id, "UpdateAttribute-tag-region"),
    ] {
        wait_for_valid(client, id, name).await?;
        start_processor(client, id).await?;
    }
    // in-apac input port intentionally not started here — orders::seed
    // wires the parent-level connection and starts border ports.

    tracing::info!(%pg_id, %rpg_id, "sink-apac seeded (RPG idle)");
    Ok(SinkApacIds { pg_id, in_port_id })
}
