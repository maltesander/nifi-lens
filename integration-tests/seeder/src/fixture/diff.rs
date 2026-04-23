//! diff-pipeline fixture: JSON → UpdateRecord → three parallel sink chains.
//! Produces provenance events where the input and output content claims
//! differ in small, diffable ways (modified status field, JSON↔CSV/Avro/Parquet
//! conversion), plus a mime.type attribute on each flowfile so the Tracer
//! content viewer modal's mime-match gate is exercised.
//!
//! ```text
//! diff-pipeline/
//! ├── GenerateFlowFile (Custom Text = ~180 KiB JSON array, schedule 30s)
//! ├── UpdateAttribute-mime-json  (sets mime.type = application/json)
//! ├── UpdateRecord-json          (diff-json-reader + diff-json-writer,
//! │                               uppercases /status — content diff,
//! │                               same mime both sides)
//! │   ├── ConvertRecord              (diff-json-reader → diff-csv-writer;
//! │   │                               flips mime.type to text/csv on output)
//! │   │   ├── UpdateAttribute-mime-csv
//! │   │   ├── UpdateRecord-csv       (diff-csv-reader + diff-csv-writer-out,
//! │   │   │                           lowercases /status)
//! │   │   └── LogAttribute-INFO      (auto-terminate success)
//! │   ├── ConvertRecord-parquet      (diff-json-reader → diff-parquet-writer)
//! │   │   └── LogAttribute-parquet   (auto-terminate success)
//! │   └── ConvertRecord-avro         (diff-json-reader → diff-avro-writer)
//! │       └── LogAttribute-avro      (auto-terminate success)
//! ```
//!
//! Controller services created inside this PG (scoped locally, not at root):
//!   - diff-json-reader       (JsonTreeReader)           ENABLED
//!   - diff-json-writer       (JsonRecordSetWriter)      ENABLED
//!   - diff-csv-reader        (CSVReader)                ENABLED
//!   - diff-csv-writer        (CSVRecordSetWriter)       ENABLED (for ConvertRecord)
//!   - diff-csv-writer-out    (CSVRecordSetWriter)       ENABLED (for UpdateRecord-csv)
//!   - diff-avro-writer       (AvroRecordSetWriter)      ENABLED (for ConvertRecord-avro)
//!   - diff-parquet-writer    (ParquetRecordSetWriter)   ENABLED (for ConvertRecord-parquet)
//!
//! Integration tests use the record-processor events as diff-testable
//! content-modification pairs:
//!
//! 1. UpdateRecord-json:      input JSON ↔ output JSON (same mime, byte diff)
//! 2. ConvertRecord:          input JSON ↔ output CSV (mime mismatch —
//!    exercises the diff-disabled path)
//! 3. UpdateRecord-csv:       input CSV  ↔ output CSV  (same mime, byte diff)
//! 4. ConvertRecord-parquet:  input JSON ↔ output Parquet (tabular decode path)
//! 5. ConvertRecord-avro:     input JSON ↔ output Avro   (tabular decode path)

use std::time::Duration;

use nifi_rust_client::dynamic::{DynamicClient, types};
use nifi_rust_client::{NifiError, wait};

use crate::entities::{make_controller_service, make_processor, props};
use crate::error::{Result, SeederError};
use crate::fixture::healthy::{
    create_child_pg, create_connection_in_pg, create_processor, start_processor, wait_for_valid,
};
use crate::fixture::{custom_text_property_key, query_record_io_property_keys};

/// Embedded JSON payload used as GenerateFlowFile's Custom Text. ~180 KiB
/// of structured sensor data (1000 records) — small enough to stay under
/// the Tracer diff mode's 512 KiB per-side cap, large enough that each
/// provenance event comfortably exceeds the 8 KiB inline preview.
const DIFF_PAYLOAD: &str = include_str!("../../assets/diff_payload.json");

/// Controller service IDs needed by the diff-pipeline processors.
struct DiffServices {
    json_reader_id: String,
    json_writer_id: String,
    csv_reader_id: String,
    csv_writer_id: String,
    csv_writer_out_id: String,
    avro_reader_id: String,
    avro_writer_id: String,
    parquet_reader_id: String,
    parquet_writer_id: String,
}

pub async fn seed(
    client: &DynamicClient,
    parent_pg_id: &str,
    version: &semver::Version,
) -> Result<()> {
    tracing::info!("seeding diff-pipeline");

    let pg_id = create_child_pg(client, parent_pg_id, "diff-pipeline").await?;
    let services = create_controller_services(client, &pg_id).await?;

    // Processors (upstream → downstream).
    let gen_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "GenerateFlowFile",
            "org.apache.nifi.processors.standard.GenerateFlowFile",
            props(&[
                (custom_text_property_key(version), DIFF_PAYLOAD),
                ("Data Format", "Text"),
                ("Unique FlowFiles", "false"),
                ("Batch Size", "1"),
            ]),
            Some("30 sec"),
            vec![],
        ),
        "GenerateFlowFile",
    )
    .await?;

    let mime_json_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-mime-json",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[("mime.type", "application/json")]),
            None,
            vec![],
        ),
        "UpdateAttribute-mime-json",
    )
    .await?;

    let upd_json_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-json",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &services.json_reader_id),
                ("Record Writer", &services.json_writer_id),
                ("/status", "${field.value:toUpper()}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-json",
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

    let mime_csv_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateAttribute-mime-csv",
            "org.apache.nifi.processors.attributes.UpdateAttribute",
            props(&[("mime.type", "text/csv")]),
            None,
            vec![],
        ),
        "UpdateAttribute-mime-csv",
    )
    .await?;

    let upd_csv_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-csv",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &services.csv_reader_id),
                ("Record Writer", &services.csv_writer_out_id),
                ("/status", "${field.value:toLower()}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-csv",
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

    // Parquet sink: ConvertRecord → UpdateRecord → QueryRecord → LogAttribute.
    // UpdateRecord rewrites only the WARN status rows (≈⅓ of records) so
    // the diff shows a partial mutation.
    // QueryRecord drops records with id ≥ SENSOR-0500, halving the row count.
    let convert_parquet_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ConvertRecord-parquet",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &services.json_reader_id),
                ("Record Writer", &services.parquet_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord-parquet",
    )
    .await?;

    let upd_parquet_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-parquet",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &services.parquet_reader_id),
                ("Record Writer", &services.parquet_writer_id),
                ("/status", "${field.value:replaceFirst('WARN', 'WARNING')}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-parquet",
    )
    .await?;

    let (qr_reader_key, qr_writer_key) = query_record_io_property_keys(version);
    let query_parquet_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "QueryRecord-parquet",
            "org.apache.nifi.processors.standard.QueryRecord",
            props(&[
                (qr_reader_key, &services.parquet_reader_id),
                (qr_writer_key, &services.parquet_writer_id),
                ("kept", "SELECT * FROM FLOWFILE WHERE id < 'SENSOR-0500'"),
            ]),
            None,
            vec!["original", "failure"],
        ),
        "QueryRecord-parquet",
    )
    .await?;

    let log_parquet_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-parquet",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-parquet",
    )
    .await?;

    // Avro sink: same shape as parquet.
    let convert_avro_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "ConvertRecord-avro",
            "org.apache.nifi.processors.standard.ConvertRecord",
            props(&[
                ("Record Reader", &services.json_reader_id),
                ("Record Writer", &services.avro_writer_id),
            ]),
            None,
            vec!["failure"],
        ),
        "ConvertRecord-avro",
    )
    .await?;

    let upd_avro_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "UpdateRecord-avro",
            "org.apache.nifi.processors.standard.UpdateRecord",
            props(&[
                ("Record Reader", &services.avro_reader_id),
                ("Record Writer", &services.avro_writer_id),
                ("/status", "${field.value:replaceFirst('WARN', 'WARNING')}"),
            ]),
            None,
            vec!["failure"],
        ),
        "UpdateRecord-avro",
    )
    .await?;

    let query_avro_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "QueryRecord-avro",
            "org.apache.nifi.processors.standard.QueryRecord",
            props(&[
                (qr_reader_key, &services.avro_reader_id),
                (qr_writer_key, &services.avro_writer_id),
                ("kept", "SELECT * FROM FLOWFILE WHERE id < 'SENSOR-0500'"),
            ]),
            None,
            vec!["original", "failure"],
        ),
        "QueryRecord-avro",
    )
    .await?;

    let log_avro_id = create_processor(
        client,
        &pg_id,
        make_processor(
            "LogAttribute-avro",
            "org.apache.nifi.processors.standard.LogAttribute",
            props(&[("Log Level", "info"), ("Log Payload", "false")]),
            None,
            vec!["success"],
        ),
        "LogAttribute-avro",
    )
    .await?;

    // Connections. Each hop carries success from the upstream processor.
    // The CSV chain is the original linear path.
    let hops_success = [
        (&gen_id, &mime_json_id),
        (&mime_json_id, &upd_json_id),
        (&upd_json_id, &convert_id),
        (&convert_id, &mime_csv_id),
        (&mime_csv_id, &upd_csv_id),
        (&upd_csv_id, &log_id),
        // Parquet chain.
        (&convert_parquet_id, &upd_parquet_id),
        (&upd_parquet_id, &query_parquet_id),
        // Avro chain.
        (&convert_avro_id, &upd_avro_id),
        (&upd_avro_id, &query_avro_id),
    ];
    for (src, dst) in hops_success {
        create_connection_in_pg(
            client,
            &pg_id,
            src,
            "PROCESSOR",
            dst,
            "PROCESSOR",
            vec!["success"],
        )
        .await?;
    }
    // QueryRecord routes its dynamic "kept" relationship into the terminator.
    for (src, dst) in [
        (&query_parquet_id, &log_parquet_id),
        (&query_avro_id, &log_avro_id),
    ] {
        create_connection_in_pg(
            client,
            &pg_id,
            src,
            "PROCESSOR",
            dst,
            "PROCESSOR",
            vec!["kept"],
        )
        .await?;
    }
    // Branch from UpdateRecord-json into the two new sink chains.
    // NiFi duplicates the flowfile to each downstream on the same relationship.
    create_connection_in_pg(
        client,
        &pg_id,
        &upd_json_id,
        "PROCESSOR",
        &convert_parquet_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;
    create_connection_in_pg(
        client,
        &pg_id,
        &upd_json_id,
        "PROCESSOR",
        &convert_avro_id,
        "PROCESSOR",
        vec!["success"],
    )
    .await?;

    // Start downstream-first so nothing backs up on startup.
    let startup_order = [
        (&log_id, "LogAttribute-INFO"),
        (&upd_csv_id, "UpdateRecord-csv"),
        (&mime_csv_id, "UpdateAttribute-mime-csv"),
        (&convert_id, "ConvertRecord"),
        (&log_parquet_id, "LogAttribute-parquet"),
        (&query_parquet_id, "QueryRecord-parquet"),
        (&upd_parquet_id, "UpdateRecord-parquet"),
        (&convert_parquet_id, "ConvertRecord-parquet"),
        (&log_avro_id, "LogAttribute-avro"),
        (&query_avro_id, "QueryRecord-avro"),
        (&upd_avro_id, "UpdateRecord-avro"),
        (&convert_avro_id, "ConvertRecord-avro"),
        (&upd_json_id, "UpdateRecord-json"),
        (&mime_json_id, "UpdateAttribute-mime-json"),
        (&gen_id, "GenerateFlowFile"),
    ];
    for (id, name) in startup_order {
        wait_for_valid(client, id, name).await?;
        start_processor(client, id).await?;
    }

    tracing::info!("diff-pipeline seeded and running");
    Ok(())
}

/// Create and enable the five controller services scoped inside the
/// diff-pipeline PG. Order: create → wait DISABLED (validated) → enable
/// → wait ENABLED.
async fn create_controller_services(client: &DynamicClient, pg_id: &str) -> Result<DiffServices> {
    let json_reader_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-json-reader",
        "org.apache.nifi.json.JsonTreeReader",
    )
    .await?;
    let json_writer_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-json-writer",
        "org.apache.nifi.json.JsonRecordSetWriter",
    )
    .await?;
    let csv_reader_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-csv-reader",
        "org.apache.nifi.csv.CSVReader",
    )
    .await?;
    let csv_writer_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-csv-writer",
        "org.apache.nifi.csv.CSVRecordSetWriter",
    )
    .await?;
    let csv_writer_out_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-csv-writer-out",
        "org.apache.nifi.csv.CSVRecordSetWriter",
    )
    .await?;
    let avro_reader_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-avro-reader",
        "org.apache.nifi.avro.AvroReader",
    )
    .await?;
    let avro_writer_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-avro-writer",
        "org.apache.nifi.avro.AvroRecordSetWriter",
    )
    .await?;
    let parquet_reader_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-parquet-reader",
        "org.apache.nifi.parquet.ParquetReader",
    )
    .await?;
    let parquet_writer_id = create_and_enable_cs(
        client,
        pg_id,
        "diff-parquet-writer",
        "org.apache.nifi.parquet.ParquetRecordSetWriter",
    )
    .await?;

    Ok(DiffServices {
        json_reader_id,
        json_writer_id,
        csv_reader_id,
        csv_writer_id,
        csv_writer_out_id,
        avro_reader_id,
        avro_writer_id,
        parquet_reader_id,
        parquet_writer_id,
    })
}

async fn create_and_enable_cs(
    client: &DynamicClient,
    parent_pg_id: &str,
    name: &str,
    cs_type: &str,
) -> Result<String> {
    tracing::info!(%name, "creating diff-pipeline controller service");
    let body = make_controller_service(name, cs_type, props(&[]));
    let created = client
        .processgroups()
        .create_controller_service(parent_pg_id, &body)
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

    let validated_timeout = Duration::from_secs(15);
    wait::controller_service_state_dynamic(
        client,
        &id,
        wait::ControllerServiceTargetState::Disabled,
        wait::WaitConfig {
            timeout: validated_timeout,
            poll_interval: Duration::from_millis(250),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| map_wait_err(e, name, "DISABLED (validated)", validated_timeout))?;

    let current = client
        .controller_services()
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
        .controller_services()
        .update_run_status(&id, &run_status)
        .await
        .map_err(|e| SeederError::Api {
            message: format!("enable {name}"),
            source: Box::new(e),
        })?;

    let enabled_timeout = Duration::from_secs(30);
    wait::controller_service_state_dynamic(
        client,
        &id,
        wait::ControllerServiceTargetState::Enabled,
        wait::WaitConfig {
            timeout: enabled_timeout,
            poll_interval: Duration::from_millis(250),
            ..Default::default()
        },
    )
    .await
    .map_err(|e| map_wait_err(e, name, "ENABLED", enabled_timeout))?;

    tracing::info!(%id, %name, "diff-pipeline controller service ENABLED");
    Ok(id)
}

fn map_wait_err(err: NifiError, what: &str, target: &str, timeout: Duration) -> SeederError {
    match err {
        NifiError::Timeout { .. } => SeederError::StateTimeout {
            what: what.to_string(),
            target_state: target.to_string(),
            elapsed_secs: timeout.as_secs(),
        },
        other => SeederError::Api {
            message: format!("wait for {what} {target}"),
            source: Box::new(other),
        },
    }
}
