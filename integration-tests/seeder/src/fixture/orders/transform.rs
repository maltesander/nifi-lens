//! transform/ child PG — bound to fixture-pc-orders.
//!
//! Stages, in order:
//!   incoming-orders (input port)
//!   LogAttribute-INFO              (front-of-stage; periodic INFO bulletin)
//!   ConvertRecord-csv2json         (CSV reader -> JSON writer)
//!   UpdateRecord-cancel-old        (status PENDING -> CANCELLED on ~1/3 records)
//!   UpdateRecord-mark-deleted      (order_id rewrites for CANCELLED rows)
//!   UpdateRecord-fx-rate           (subtotal_usd = subtotal_local * `#{usd_rate}`)
//!   UpdateAttribute-tag-retries    (sets `_max_retries = #{retry_max}`,
//!                                   `_secret = #{db_password}` — exercises [S])
//!   RouteOnAttribute-region        (region IN [EU, US, APAC])
//!   out-eu / out-us / out-apac / out-failed (output ports)
//!
//! Failure relationship of UpdateRecord-fx-rate routes to out-failed.

use nifi_rust_client::dynamic::DynamicClient;

use crate::entities::{make_processor, props};
use crate::error::Result;
use crate::fixture::common::{
    self, create_connection_in_pg, create_input_port, create_output_port, create_processor,
    start_input_port, start_output_port, start_processor, wait_for_valid,
};
use crate::fixture::parameter_contexts::{self, OrdersContextIds};
use crate::fixture::services::{self, ServiceIds};

pub struct TransformIds {
    pub pg_id: String,
    pub incoming_port_id: String,
    pub out_eu_port_id: String,
    pub out_us_port_id: String,
    pub out_apac_port_id: String,
    pub out_failed_port_id: String,
}

pub async fn seed(
    client: &DynamicClient,
    orders_pg_id: &str,
    contexts: &OrdersContextIds,
    service_ids: &ServiceIds,
    _version: &semver::Version,
) -> Result<TransformIds> {
    tracing::info!("seeding orders-pipeline/transform");

    let pg_id = common::create_child_pg(client, orders_pg_id, "transform").await?;
    parameter_contexts::bind(client, &pg_id, &contexts.orders_id).await?;

    // Scoped CSV reader. The root-level fixture-csv-reader is created
    // DISABLED (services::create_disabled_csv_reader, kept that way to
    // exercise the disabled-CS modal); we need an enabled instance here.
    // Root-level fixture-json-reader/json-writer are reused (both ENABLED).
    let csv_reader_id = services::create_and_enable_cs_inline(
        client,
        &pg_id,
        "orders-csv-reader",
        "org.apache.nifi.csv.CSVReader",
    )
    .await?;

    let incoming_port_id = create_input_port(client, &pg_id, "incoming-orders").await?;

    let log_front = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-INFO",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec![],
        ),
        "LogAttribute-INFO",
    )
    .await?;

    let convert_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ConvertRecord-csv2json",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &csv_reader_id),
                ("Record Writer", &service_ids.json_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord-csv2json",
    )
    .await?;

    let cancel_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-cancel-old",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &service_ids.json_reader_id),
                ("Record Writer", &service_ids.json_writer_id),
                // Cancel any record whose status is PENDING.
                // ~25% of records have PENDING from the generator.
                (
                    "/status",
                    "${field.value:replaceFirst('PENDING', 'CANCELLED')}",
                ),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-cancel-old",
    )
    .await?;

    let mark_deleted_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-mark-deleted",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &service_ids.json_reader_id),
                ("Record Writer", &service_ids.json_writer_id),
                // For CANCELLED rows, rewrite order_id from ORD-xxx to DELETED-xxx.
                (
                    "/order_id",
                    "${field.value:replaceFirst('^ORD-', 'DELETED-')}",
                ),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-mark-deleted",
    )
    .await?;

    let fx_rate_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-fx-rate",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &service_ids.json_reader_id),
                ("Record Writer", &service_ids.json_writer_id),
                // THE BREAKING STAGE
                // RecordPath multiply on a numeric field by a parameter EL.
                // When `#{usd_rate}` resolves to "oops", multiplication fails
                // and the flowfile routes to `failure` (out-failed -> deadletter).
                ("/subtotal_usd", "${field.value:multiply(#{usd_rate})}"),
            ]),
            None,
            vec![], // Do NOT auto-terminate `failure` — we route it.
        ),
        "UpdateRecord-fx-rate",
    )
    .await?;

    let tag_retries_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-tag-retries",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("_max_retries", "#{retry_max}"),
                ("_secret", "#{db_password}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-tag-retries",
    )
    .await?;

    // RouteOnAttribute splits by region. Since this fixture's flowfiles
    // don't carry a per-record `region` attribute (the field is in the
    // record content, not a flowfile attribute), we route via parameter
    // values: the regional contexts override `region_filter`. At the
    // transform/ level, region_filter is "EU,US,APAC", so all three
    // routes fire on every flowfile (NiFi clones the flowfile to each
    // matching downstream).
    let route_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "RouteOnAttribute-region",
            "org.apache.nifi.processors.standard.RouteOnAttribute",
            // Default Routing Strategy ("Route to Property name") emits
            // one relationship per dynamic property — flowfiles clone to
            // every matching downstream. The "Route to 'matched' if all
            // match" strategy would collapse these into a single
            // `matched`/`unmatched` pair, breaking the per-region
            // connections below.
            props(&[
                ("region-eu", "${region_filter:contains('EU')}"),
                ("region-us", "${region_filter:contains('US')}"),
                ("region-apac", "${region_filter:contains('APAC')}"),
            ]),
            None,
            vec!["unmatched"],
        ),
        "RouteOnAttribute-region",
    )
    .await?;

    // Output ports.
    let out_eu_port_id = create_output_port(client, &pg_id, "out-eu").await?;
    let out_us_port_id = create_output_port(client, &pg_id, "out-us").await?;
    let out_apac_port_id = create_output_port(client, &pg_id, "out-apac").await?;
    let out_failed_port_id = create_output_port(client, &pg_id, "out-failed").await?;

    // Connections (intra-PG).
    create_connection_in_pg(
        client,
        &pg_id,
        &incoming_port_id,
        "INPUT_PORT",
        &log_front,
        "PROCESSOR",
        vec![],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &log_front,
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
        &cancel_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &cancel_id,
        "PROCESSOR",
        &mark_deleted_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &mark_deleted_id,
        "PROCESSOR",
        &fx_rate_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    // fx-rate failure -> out-failed
    create_connection_in_pg(
        client,
        &pg_id,
        &fx_rate_id,
        "PROCESSOR",
        &out_failed_port_id,
        "OUTPUT_PORT",
        vec!["failure"],
    )
    .await?;
    // fx-rate success -> tag_retries
    create_connection_in_pg(
        client,
        &pg_id,
        &fx_rate_id,
        "PROCESSOR",
        &tag_retries_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &tag_retries_id,
        "PROCESSOR",
        &route_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    // Route to regional output ports.
    create_connection_in_pg(
        client,
        &pg_id,
        &route_id,
        "PROCESSOR",
        &out_eu_port_id,
        "OUTPUT_PORT",
        vec!["region-eu"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &route_id,
        "PROCESSOR",
        &out_us_port_id,
        "OUTPUT_PORT",
        vec!["region-us"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &route_id,
        "PROCESSOR",
        &out_apac_port_id,
        "OUTPUT_PORT",
        vec!["region-apac"],
    )
    .await?;

    // Start downstream-first.
    start_output_port(client, &out_eu_port_id).await?;
    start_output_port(client, &out_us_port_id).await?;
    start_output_port(client, &out_apac_port_id).await?;
    start_output_port(client, &out_failed_port_id).await?;
    for (id, name) in [
        (&route_id, "RouteOnAttribute-region"),
        (&tag_retries_id, "UpdateAttribute-tag-retries"),
        (&fx_rate_id, "UpdateRecord-fx-rate"),
        (&mark_deleted_id, "UpdateRecord-mark-deleted"),
        (&cancel_id, "UpdateRecord-cancel-old"),
        (&convert_id, "ConvertRecord-csv2json"),
        (&log_front, "LogAttribute-INFO"),
    ] {
        wait_for_valid(client, id, name).await?;
        start_processor(client, id).await?;
    }
    start_input_port(client, &incoming_port_id).await?;

    Ok(TransformIds {
        pg_id,
        incoming_port_id,
        out_eu_port_id,
        out_us_port_id,
        out_apac_port_id,
        out_failed_port_id,
    })
}
