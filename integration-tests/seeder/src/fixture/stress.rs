//! Stress-pipeline fixture: longer branching flow with ConvertRecord,
//! UpdateRecord, RouteOnAttribute, and dual ControlRate bottlenecks.
//!
//! Version-gated: only seeded when the detected NiFi version is >= 2.9.0.
//!
//! ```text
//! stress-pipeline/
//! ├── GenerateFlowFile (500 ms, flat JSON sensor payload)
//! ├── UpdateAttribute-source
//! ├── ConvertRecord (JSON -> CSV)
//! ├── UpdateRecord (add computed + transform fields)
//! ├── RouteOnAttribute
//! │   ├── "hot" (temperature > 40) -> UpdateAttribute-alert -> ControlRate-hot (1/min) -> LogAttribute-alert
//! │   └── "normal" (unmatched) -> ControlRate-normal (2/min) -> LogAttribute-sink
//! ```
//!
//! Controller services (all ENABLED, created at root PG):
//!   - stress-json-reader (JsonTreeReader) — ConvertRecord reader
//!   - stress-csv-writer (CSVRecordSetWriter) — ConvertRecord writer
//!   - stress-csv-reader (CSVReader) — UpdateRecord reader
//!   - stress-csv-writer-out (CSVRecordSetWriter) — UpdateRecord writer

use std::time::Duration;

use nifi_rust_client::dynamic::{
    DynamicClient,
    traits::{
        ControllerServicesApi as _, ControllerServicesRunStatusApi as _, ProcessGroupsApi as _,
        ProcessGroupsConnectionsApi as _, ProcessGroupsControllerServicesApi as _,
    },
    types,
};

use crate::entities::{make_connection, make_controller_service, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::healthy::{
    create_child_pg, create_connection_in_pg, create_processor, start_processor, wait_for_valid,
};
use crate::fixture::stress_payload::STRESS_PAYLOAD;
use crate::state::poll_until;

/// Controller service IDs returned by `create_controller_services`.
struct StressServices {
    json_reader_id: String,
    csv_writer_id: String,
    csv_reader_id: String,
    csv_writer_out_id: String,
}

/// Create and enable the four controller services needed by the stress
/// pipeline. All are created at the root PG level.
async fn create_controller_services(client: &DynamicClient) -> Result<StressServices> {
    let json_reader_id = create_and_enable_cs(
        client,
        "stress-json-reader",
        "org.apache.nifi.json.JsonTreeReader",
        props(&[]),
    )
    .await?;
    let csv_writer_id = create_and_enable_cs(
        client,
        "stress-csv-writer",
        "org.apache.nifi.csv.CSVRecordSetWriter",
        props(&[]),
    )
    .await?;
    let csv_reader_id = create_and_enable_cs(
        client,
        "stress-csv-reader",
        "org.apache.nifi.csv.CSVReader",
        props(&[]),
    )
    .await?;
    let csv_writer_out_id = create_and_enable_cs(
        client,
        "stress-csv-writer-out",
        "org.apache.nifi.csv.CSVRecordSetWriter",
        props(&[]),
    )
    .await?;

    Ok(StressServices {
        json_reader_id,
        csv_writer_id,
        csv_reader_id,
        csv_writer_out_id,
    })
}

/// Create a controller service at the root PG and enable it. Returns the
/// service ID.
async fn create_and_enable_cs(
    client: &DynamicClient,
    name: &str,
    cs_type: &str,
    properties: std::collections::HashMap<String, String>,
) -> Result<String> {
    tracing::info!(%name, "creating controller service");
    let body = make_controller_service(name, cs_type, properties);
    let created = client
        .processgroups_api()
        .controller_services("root")
        .create_controller_service_1(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create {name}"),
            source: Box::new(e),
        })?;
    let id = created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("{name} has no id after create"),
        })?;

    // Poll for DISABLED (validation finished).
    let id_poll = id.clone();
    poll_until(
        name,
        "DISABLED (validated)",
        Duration::from_secs(15),
        || {
            let id = id_poll.clone();
            async move {
                let got = client
                    .controller_services_api()
                    .get_controller_service(&id, None)
                    .await
                    .map_err(|e| SeederError::Api {
                        message: format!("poll {name} validation"),
                        source: Box::new(e),
                    })?;
                let state = got.component.and_then(|c| c.state).unwrap_or_default();
                if state == "DISABLED" {
                    Ok(Some(()))
                } else {
                    Ok(None)
                }
            }
        },
    )
    .await?;

    // Flip to ENABLED — fetch current revision first.
    let current = client
        .controller_services_api()
        .get_controller_service(&id, None)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("fetch revision for {name}"),
            source: Box::new(e),
        })?;
    let revision = current.revision.ok_or_else(|| SeederError::Invariant {
        message: format!("{name} has no revision"),
    })?;

    let mut run_status = types::ControllerServiceRunStatusEntity::default();
    run_status.state = Some("ENABLED".to_string());
    run_status.revision = Some(revision);
    client
        .controller_services_api()
        .run_status(&id)
        .update_run_status_1(&run_status)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("enable {name}"),
            source: Box::new(e),
        })?;

    let id_poll = id.clone();
    poll_until(name, "ENABLED", Duration::from_secs(30), || {
        let id = id_poll.clone();
        async move {
            let got = client
                .controller_services_api()
                .get_controller_service(&id, None)
                .await
                .map_err(|e| SeederError::Api {
                    message: format!("poll {name} enable"),
                    source: Box::new(e),
                })?;
            let state = got.component.and_then(|c| c.state).unwrap_or_default();
            if state == "ENABLED" {
                Ok(Some(()))
            } else {
                Ok(None)
            }
        }
    })
    .await?;

    tracing::info!(%id, %name, "controller service ENABLED");
    Ok(id)
}

/// Top-level seed entry for the stress pipeline. Creates controller
/// services, the PG, all processors and connections, and starts
/// everything downstream-first.
pub async fn seed(client: &DynamicClient, parent_pg_id: &str) -> Result<()> {
    tracing::info!("seeding stress-pipeline");

    // 1. Controller services (at root level).
    let services = create_controller_services(client).await?;

    // 2. Create the PG.
    let pg_id = create_child_pg(client, parent_pg_id, "stress-pipeline").await?;

    // 3. Processors.
    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[
                ("Custom Text", STRESS_PAYLOAD),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("500 ms"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let ua_source_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-source",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("source.node", "${hostname()}"),
                ("source.pipeline", "stress"),
                ("source.batch_id", "${UUID()}"),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-source",
    )
    .await?;

    let convert_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ConvertRecord",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &services.json_reader_id),
                ("Record Writer", &services.csv_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord",
    )
    .await?;

    let update_record_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &services.csv_reader_id),
                ("Record Writer", &services.csv_writer_out_id),
                (
                    "/processed_at",
                    "${now():format('yyyy-MM-dd HH:mm:ss.SSS')}",
                ),
                (
                    "/quality",
                    "${/temperature:toNumber():gt(40):ifElse('high','normal')}",
                ),
                ("/status", "${/status:toUpper()}"),
                ("/temperature", "${/temperature:toNumber():format('0')}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord",
    )
    .await?;

    let route_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "RouteOnAttribute",
            "org.apache.nifi.processors.standard.RouteOnAttribute",
            props(&[("hot", "${quality:equals('high')}")]),
            None,
            vec![],
        ),
        "RouteOnAttribute",
    )
    .await?;

    let ua_alert_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-alert",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[
                ("alert.level", "critical"),
                (
                    "alert.reason",
                    "temperature=${temperature} exceeds threshold",
                ),
                (
                    "alert.timestamp",
                    "${now():format('yyyy-MM-dd HH:mm:ss.SSS')}",
                ),
            ]),
            None,
            vec![],
        ),
        "UpdateAttribute-alert",
    )
    .await?;

    let ctrl_hot_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ControlRate-hot",
            "org.apache.nifi.processors.standard.ControlRate",
            props(&[
                ("Rate Control Criteria", "flowfile count"),
                ("Maximum Rate", "1"),
                ("Time Duration", "1 min"),
            ]),
            None,
            vec!["failure"],
        ),
        "ControlRate-hot",
    )
    .await?;

    let ctrl_normal_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ControlRate-normal",
            "org.apache.nifi.processors.standard.ControlRate",
            props(&[
                ("Rate Control Criteria", "flowfile count"),
                ("Maximum Rate", "2"),
                ("Time Duration", "1 min"),
            ]),
            None,
            vec!["failure"],
        ),
        "ControlRate-normal",
    )
    .await?;

    let log_alert_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-alert",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "warn"), ("Log Payload", "true")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-alert",
    )
    .await?;

    let log_sink_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-sink",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "true")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-sink",
    )
    .await?;

    // 4. Connections.

    // GenerateFlowFile -> UpdateAttribute-source
    create_connection_in_pg(
        client,
        &pg_id,
        &gen_id,
        "PROCESSOR",
        &ua_source_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // UpdateAttribute-source -> ConvertRecord
    create_connection_in_pg(
        client,
        &pg_id,
        &ua_source_id,
        "PROCESSOR",
        &convert_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // ConvertRecord -> UpdateRecord (backpressure: 50 objects, 10 KB)
    create_connection_with_backpressure(
        client,
        &pg_id,
        &convert_id,
        &update_record_id,
        vec!["success"],
        50,
        "10 KB",
    )
    .await?;

    // UpdateRecord -> RouteOnAttribute (backpressure: 100 objects, 50 KB)
    create_connection_with_backpressure(
        client,
        &pg_id,
        &update_record_id,
        &route_id,
        vec!["success"],
        100,
        "50 KB",
    )
    .await?;

    // RouteOnAttribute -> UpdateAttribute-alert (hot path, backpressure: 30 objects, 5 KB)
    create_connection_with_backpressure(
        client,
        &pg_id,
        &route_id,
        &ua_alert_id,
        vec!["hot"],
        30,
        "5 KB",
    )
    .await?;

    // RouteOnAttribute -> ControlRate-normal (unmatched path, backpressure: 50 objects, 10 KB)
    create_connection_with_backpressure(
        client,
        &pg_id,
        &route_id,
        &ctrl_normal_id,
        vec!["unmatched"],
        50,
        "10 KB",
    )
    .await?;

    // UpdateAttribute-alert -> ControlRate-hot (backpressure: 30 objects, 5 KB)
    create_connection_with_backpressure(
        client,
        &pg_id,
        &ua_alert_id,
        &ctrl_hot_id,
        vec!["success"],
        30,
        "5 KB",
    )
    .await?;

    // ControlRate-hot -> LogAttribute-alert (default backpressure)
    create_connection_in_pg(
        client,
        &pg_id,
        &ctrl_hot_id,
        "PROCESSOR",
        &log_alert_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // ControlRate-normal -> LogAttribute-sink (default backpressure)
    create_connection_in_pg(
        client,
        &pg_id,
        &ctrl_normal_id,
        "PROCESSOR",
        &log_sink_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // 5. Start everything downstream-first.
    // Sinks.
    wait_for_valid(client, &log_alert_id, "LogAttribute-alert").await?;
    start_processor(client, &log_alert_id).await?;
    wait_for_valid(client, &log_sink_id, "LogAttribute-sink").await?;
    start_processor(client, &log_sink_id).await?;

    // Bottlenecks.
    wait_for_valid(client, &ctrl_hot_id, "ControlRate-hot").await?;
    start_processor(client, &ctrl_hot_id).await?;
    wait_for_valid(client, &ctrl_normal_id, "ControlRate-normal").await?;
    start_processor(client, &ctrl_normal_id).await?;

    // Hot path enrichment.
    wait_for_valid(client, &ua_alert_id, "UpdateAttribute-alert").await?;
    start_processor(client, &ua_alert_id).await?;

    // Router.
    wait_for_valid(client, &route_id, "RouteOnAttribute").await?;
    start_processor(client, &route_id).await?;

    // Transform stages.
    wait_for_valid(client, &update_record_id, "UpdateRecord").await?;
    start_processor(client, &update_record_id).await?;
    wait_for_valid(client, &convert_id, "ConvertRecord").await?;
    start_processor(client, &convert_id).await?;

    // Source enrichment.
    wait_for_valid(client, &ua_source_id, "UpdateAttribute-source").await?;
    start_processor(client, &ua_source_id).await?;

    // Producer — last.
    wait_for_valid(client, &gen_id, "GenerateFlowFile").await?;
    start_processor(client, &gen_id).await?;

    tracing::info!("stress-pipeline seeded and running");
    Ok(())
}

/// Create a same-PG connection with custom backpressure thresholds.
async fn create_connection_with_backpressure(
    client: &DynamicClient,
    pg_id: &str,
    source_id: &str,
    destination_id: &str,
    relationships: Vec<&str>,
    object_threshold: i64,
    size_threshold: &str,
) -> Result<String> {
    let mut body = make_connection(
        pg_id,
        source_id,
        "PROCESSOR",
        pg_id,
        destination_id,
        "PROCESSOR",
        relationships,
    );
    if let Some(component) = body.component.as_mut() {
        component.back_pressure_object_threshold = Some(object_threshold);
        component.back_pressure_data_size_threshold = Some(size_threshold.to_string());
    }

    let created = client
        .processgroups_api()
        .connections(pg_id)
        .create_connection(&body)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("create connection with backpressure in pg {pg_id}"),
            source: Box::new(e),
        })?;
    created
        .component
        .and_then(|c| c.id)
        .or(created.id)
        .ok_or_else(|| SeederError::Invariant {
            message: format!("connection in pg {pg_id} has no id"),
        })
}
