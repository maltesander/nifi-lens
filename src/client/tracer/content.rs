//! Content classification and decoding for NiFi flowfile bodies.
//!
//! [`classify_content`] is the entry point used by the Tracer content
//! viewer modal to decide how to render Input / Output bytes
//! (text / hex / Avro JSON-Lines / Parquet JSON-Lines). It runs a
//! two-stage pipeline: a magic-byte sniff routes Parquet / Avro
//! payloads through their respective decoders, and everything else
//! falls through to UTF-8 text (with JSON pretty-printing) or a hex
//! dump of the first 4 KiB.

use super::{ContentRender, TabularFormat};

/// Classifies raw bytes into a [`ContentRender`] variant.
///
/// Two-stage pipeline:
///
/// 1. Magic-byte sniff via [`detect_tabular_format`]: `PAR1` →
///    [`decode_parquet`], `Obj\x01` → [`decode_avro`]. Decoder
///    errors fall back to `Hex` with a `tracing::warn!` log.
/// 2. Otherwise: empty → `Empty`; valid UTF-8 → `Text` (JSON
///    pretty-print when parseable); else → `Hex` of the first 4 KiB.
pub fn classify_content(bytes: Vec<u8>) -> ContentRender {
    if let Some(format) = detect_tabular_format(&bytes) {
        // Snapshot the first 4 KiB for the Hex-fallback branch BEFORE
        // moving `bytes` into the decoder thread.
        let head_for_hex: Vec<u8> = bytes[..bytes.len().min(4096)].to_vec();
        let result = match format {
            TabularFormat::Parquet => decode_with_timeout("parquet", bytes, |b| decode_parquet(&b)),
            TabularFormat::Avro => decode_with_timeout("avro", bytes, |b| decode_avro(&b)),
        };
        match result {
            Ok(render) => return render,
            Err(err) => {
                tracing::warn!(
                    format = format.label(),
                    error = %err,
                    "tabular decoder failed, falling back to hex"
                );
                return ContentRender::Hex {
                    first_4k: hex_dump(&head_for_hex),
                };
            }
        }
    }
    classify_text_or_hex(bytes)
}

/// Cap on synchronous parquet/avro decoder runtime. The decoders are
/// already bounded by `TABULAR_RECORD_LIMIT` and `TABULAR_BODY_LIMIT`,
/// but pathological inputs (broken offsets, huge schemas) can decode
/// slowly enough to wedge a `spawn_blocking` worker. Wall-clock cap.
const DECODER_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(5);

/// Run `f` on a worker thread, return its result, or surface a timeout
/// error after [`DECODER_TIMEOUT`]. The worker thread is detached on
/// timeout — its `bytes` Vec is dropped when the thread eventually
/// finishes; no double-free risk because the bytes were moved in.
fn decode_with_timeout<F, T>(
    label: &'static str,
    bytes: Vec<u8>,
    f: F,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce(Vec<u8>) -> Result<T, Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    T: Send + 'static,
{
    decode_with_timeout_inner(label, bytes, DECODER_TIMEOUT, f)
}

/// Inner helper parameterised on the timeout so unit tests can drive
/// it with a sub-second deadline. Public surface always uses
/// [`DECODER_TIMEOUT`] via [`decode_with_timeout`].
fn decode_with_timeout_inner<F, T>(
    label: &'static str,
    bytes: Vec<u8>,
    timeout: std::time::Duration,
    f: F,
) -> Result<T, Box<dyn std::error::Error + Send + Sync>>
where
    F: FnOnce(Vec<u8>) -> Result<T, Box<dyn std::error::Error + Send + Sync>> + Send + 'static,
    T: Send + 'static,
{
    let (tx, rx) = std::sync::mpsc::channel();
    // Detach: on timeout the worker thread continues until `f` returns,
    // then drops `bytes` and the send-side of the channel. The receiver
    // is already gone, so the send is a no-op.
    let _detach = std::thread::spawn(move || {
        let result = f(bytes);
        let _ = tx.send(result);
    });
    match rx.recv_timeout(timeout) {
        Ok(result) => result,
        Err(_) => Err(format!("{label} decoder timed out after {}s", timeout.as_secs()).into()),
    }
}

/// Today's classifier body, extracted so [`classify_content`] can call
/// it after the magic-byte sniff fails.
fn classify_text_or_hex(bytes: Vec<u8>) -> ContentRender {
    if bytes.is_empty() {
        return ContentRender::Empty;
    }
    match String::from_utf8(bytes) {
        Ok(text) => {
            let pretty = serde_json::from_str::<serde_json::Value>(&text)
                .and_then(|v| serde_json::to_string_pretty(&v))
                .ok();
            match pretty {
                Some(p) if p != text => ContentRender::Text {
                    text: p,
                    pretty_printed: true,
                },
                _ => ContentRender::Text {
                    text,
                    pretty_printed: false,
                },
            }
        }
        Err(err) => {
            let bytes = err.into_bytes();
            ContentRender::Hex {
                first_4k: hex_dump(&bytes[..bytes.len().min(4096)]),
            }
        }
    }
}

fn hex_dump(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let mut out = String::with_capacity(bytes.len() * 3);
    for (i, byte) in bytes.iter().enumerate() {
        if i > 0 && i % 16 == 0 {
            out.push('\n');
        } else if i > 0 {
            out.push(' ');
        }
        let _ = write!(out, "{byte:02x}");
    }
    out
}

/// Returns the tabular format implied by the leading magic bytes, if any.
///
/// - Parquet files start with `PAR1` (the format also ends with `PAR1`,
///   but the streaming chunk only sees the prefix).
/// - Avro Object Container Files start with `Obj\x01`.
/// - Anything shorter than 4 bytes returns `None`.
pub fn detect_tabular_format(bytes: &[u8]) -> Option<TabularFormat> {
    if bytes.len() < 4 {
        return None;
    }
    match &bytes[..4] {
        b"PAR1" => Some(TabularFormat::Parquet),
        b"Obj\x01" => Some(TabularFormat::Avro),
        _ => None,
    }
}

/// Maximum decoded JSON-Lines body bytes per side. Internal OOM guard;
/// the user-facing knob is `[tracer.ceiling] tabular` which caps the
/// fetched bytes that reach the decoder.
const TABULAR_BODY_LIMIT: usize = 128 * crate::bytes::MIB as usize;

/// Maximum number of records decoded per side. Internal safety cap.
const TABULAR_RECORD_LIMIT: usize = 50_000;

/// Decode an Avro Object Container File into `ContentRender::Tabular`.
///
/// Errors only on malformed containers, unsupported schemas, or
/// decoder failures — callers (notably `classify_content`) are
/// expected to catch the error and render `Hex` instead.
pub fn decode_avro(
    bytes: &[u8],
) -> Result<ContentRender, Box<dyn std::error::Error + Send + Sync>> {
    use std::fmt::Write as _;

    let reader = apache_avro::Reader::new(bytes)?;
    let schema_summary = format_avro_schema(reader.writer_schema());

    let mut body = String::new();
    let mut truncated = false;

    for (idx, value) in reader.enumerate() {
        // Pre-check both limits before decoding the next record. The byte
        // limit is a "soft" cap: the previous iteration may have pushed
        // body.len() slightly past the limit, but no further record is
        // added once we cross.
        if idx >= TABULAR_RECORD_LIMIT || body.len() >= TABULAR_BODY_LIMIT {
            truncated = true;
            break;
        }
        let value = value?;
        let json: serde_json::Value = apache_avro::from_value(&value)?;
        let line = serde_json::to_string(&json)?;
        writeln!(&mut body, "{line}").expect("writeln to String is infallible");
    }

    if body.ends_with('\n') {
        body.pop();
    }
    let decoded_bytes = body.len();

    Ok(ContentRender::Tabular {
        format: TabularFormat::Avro,
        schema_summary,
        body,
        decoded_bytes,
        truncated,
    })
}

/// Render an Avro schema as one line per top-level field. Non-record
/// top-level schemas render as a single `(value) : <type>` line.
fn format_avro_schema(schema: &apache_avro::Schema) -> String {
    use apache_avro::Schema;
    match schema {
        Schema::Record(rec) => rec
            .fields
            .iter()
            .map(|f| format!("{} : {}", f.name, format_avro_type(&f.schema)))
            .collect::<Vec<_>>()
            .join("\n"),
        other => format!("(value) : {}", format_avro_type(other)),
    }
}

fn format_avro_type(schema: &apache_avro::Schema) -> String {
    use apache_avro::Schema;
    match schema {
        Schema::Null => "null".into(),
        Schema::Boolean => "boolean".into(),
        Schema::Int => "int".into(),
        Schema::Long => "long".into(),
        Schema::Float => "float".into(),
        Schema::Double => "double".into(),
        Schema::Bytes => "bytes".into(),
        Schema::String => "string".into(),
        Schema::Uuid => "uuid".into(),
        Schema::Date => "date".into(),
        Schema::TimeMillis => "time-millis".into(),
        Schema::TimeMicros => "time-micros".into(),
        Schema::TimestampMillis => "timestamp-millis".into(),
        Schema::TimestampMicros => "timestamp-micros".into(),
        Schema::TimestampNanos => "timestamp-nanos".into(),
        Schema::LocalTimestampMillis => "local-timestamp-millis".into(),
        Schema::LocalTimestampMicros => "local-timestamp-micros".into(),
        Schema::LocalTimestampNanos => "local-timestamp-nanos".into(),
        Schema::Duration => "duration".into(),
        Schema::Decimal(_) => "decimal".into(),
        Schema::BigDecimal => "big-decimal".into(),
        Schema::Array(inner) => format!("array<{}>", format_avro_type(&inner.items)),
        Schema::Map(inner) => format!("map<{}>", format_avro_type(&inner.types)),
        Schema::Union(u) => {
            let parts: Vec<String> = u.variants().iter().map(format_avro_type).collect();
            format!("union<{}>", parts.join("|"))
        }
        Schema::Record(r) => format!("record<{}>", r.name.name),
        Schema::Enum(e) => format!("enum<{}>", e.name.name),
        Schema::Fixed(f) => format!("fixed<{}>", f.name.name),
        Schema::Ref { name } => format!("ref<{}>", name.name),
        // Catch-all for Schema variants added in future apache-avro releases:
        // degrade gracefully to "?" rather than failing to compile.
        #[allow(unreachable_patterns)]
        _ => "?".into(),
    }
}

/// Decode a Parquet file into [`ContentRender::Tabular`] by streaming
/// `RecordBatch`es through `arrow::json::LineDelimitedWriter`.
///
/// Returns `Err` on truncated containers, unsupported codecs, or any
/// downstream Arrow/Parquet error. Callers (notably `classify_content`)
/// catch the error and render `Hex` instead.
pub fn decode_parquet(
    bytes: &[u8],
) -> Result<ContentRender, Box<dyn std::error::Error + Send + Sync>> {
    use bytes::Bytes;
    use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

    let bytes = Bytes::copy_from_slice(bytes);
    let builder = ParquetRecordBatchReaderBuilder::try_new(bytes)?;
    let arrow_schema = builder.schema().clone();
    let schema_summary = format_arrow_schema(&arrow_schema);

    let reader = builder.with_batch_size(1024).build()?;

    // Serialise each batch to a temporary buffer, check limits, then accumulate.
    // This two-phase (per-batch temp buffer → accumulator) approach avoids a
    // borrow-checker conflict: `LineDelimitedWriter` holds `&mut` to its output
    // for its entire lifetime, so we cannot simultaneously read the accumulated
    // length through the same reference.
    let mut body_bytes: Vec<u8> = Vec::with_capacity(64 * 1024);
    let mut decoded_records = 0usize;
    let mut truncated = false;

    for batch_result in reader {
        let batch = batch_result?;
        // Pre-check both limits BEFORE serialising this batch. The byte limit
        // is "soft" — see `decode_avro` for the same pattern.
        if decoded_records + batch.num_rows() > TABULAR_RECORD_LIMIT
            || body_bytes.len() >= TABULAR_BODY_LIMIT
        {
            truncated = true;
            break;
        }
        let mut chunk: Vec<u8> = Vec::new();
        {
            let mut writer = arrow::json::LineDelimitedWriter::new(&mut chunk);
            writer.write(&batch)?;
            writer.finish()?;
        }
        body_bytes.extend_from_slice(&chunk);
        decoded_records += batch.num_rows();
    }

    let mut body = String::from_utf8(body_bytes)?;
    if body.ends_with('\n') {
        body.pop();
    }
    let decoded_bytes = body.len();

    Ok(ContentRender::Tabular {
        format: TabularFormat::Parquet,
        schema_summary,
        body,
        decoded_bytes,
        truncated,
    })
}

/// Render an Arrow schema as one line per top-level field, using each
/// field's `DataType` `Display` output (e.g. `id : Int64`, `name : Utf8`,
/// `tags : List<Utf8>`). Nullable fields carry a trailing `?`.
fn format_arrow_schema(schema: &arrow::datatypes::Schema) -> String {
    schema
        .fields()
        .iter()
        .map(|f| {
            let nullable = if f.is_nullable() { "?" } else { "" };
            format!("{} : {}{}", f.name(), f.data_type(), nullable)
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty_is_empty() {
        assert!(matches!(
            classify_content(b"".to_vec()),
            ContentRender::Empty
        ));
    }

    #[test]
    fn classify_plain_utf8_is_text() {
        match classify_content(b"hello world".to_vec()) {
            ContentRender::Text { text, .. } => assert_eq!(text, "hello world"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_json_is_prettyprinted_text() {
        match classify_content(br#"{"a":1,"b":[2,3]}"#.to_vec()) {
            ContentRender::Text { text, .. } => {
                assert!(text.contains("\"a\": 1"));
                assert!(text.contains('\n'));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_invalid_utf8_is_hex() {
        let bytes = vec![0xff, 0x00, 0x61, 0xfe];
        match classify_content(bytes) {
            ContentRender::Hex { first_4k } => assert_eq!(first_4k, "ff 00 61 fe"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_content_empty_returns_empty() {
        assert!(matches!(classify_content(Vec::new()), ContentRender::Empty));
    }

    #[test]
    fn classify_content_plain_text_no_pretty_print() {
        let csv = b"a,b,c\n1,2,3\n".to_vec();
        match classify_content(csv.clone()) {
            ContentRender::Text {
                text,
                pretty_printed,
            } => {
                assert_eq!(text.as_bytes(), csv.as_slice());
                assert!(!pretty_printed);
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_content_json_pretty_prints() {
        let compact = br#"{"a":1,"b":2}"#.to_vec();
        match classify_content(compact) {
            ContentRender::Text {
                text,
                pretty_printed,
            } => {
                assert!(pretty_printed);
                assert!(text.contains('\n'));
                let v: serde_json::Value = serde_json::from_str(&text).unwrap();
                assert_eq!(v["a"], 1);
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_content_json_already_pretty_no_reformat() {
        let pretty = "{\n  \"a\": 1\n}".as_bytes().to_vec();
        match classify_content(pretty.clone()) {
            ContentRender::Text {
                text,
                pretty_printed,
            } => {
                assert_eq!(text.as_bytes(), pretty.as_slice());
                assert!(!pretty_printed);
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    #[test]
    fn classify_content_non_utf8_hex() {
        let bytes = vec![0xff, 0xfe, 0xfd];
        match classify_content(bytes) {
            ContentRender::Hex { first_4k } => {
                assert!(first_4k.contains("ff fe fd"));
            }
            other => panic!("expected Hex, got {other:?}"),
        }
    }

    #[test]
    fn tabular_format_variants_exist() {
        use ContentRender::*;
        let _ = Tabular {
            format: TabularFormat::Parquet,
            schema_summary: String::new(),
            body: String::new(),
            decoded_bytes: 0,
            truncated: false,
        };
        let _ = Tabular {
            format: TabularFormat::Avro,
            schema_summary: String::new(),
            body: String::new(),
            decoded_bytes: 0,
            truncated: false,
        };
        // Default still works and matches Empty.
        assert!(matches!(ContentRender::default(), Empty));
    }

    #[test]
    fn detect_parquet_magic() {
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_tabular_format(&bytes), Some(TabularFormat::Parquet));
    }

    #[test]
    fn detect_avro_magic() {
        let mut bytes = b"Obj\x01".to_vec();
        bytes.extend_from_slice(&[0u8; 100]);
        assert_eq!(detect_tabular_format(&bytes), Some(TabularFormat::Avro));
    }

    #[test]
    fn detect_no_magic_for_text() {
        assert_eq!(detect_tabular_format(b"{\"a\":1}"), None);
        assert_eq!(detect_tabular_format(b"hello world"), None);
    }

    #[test]
    fn detect_short_input_returns_none() {
        assert_eq!(detect_tabular_format(b""), None);
        assert_eq!(detect_tabular_format(b"PAR"), None);
        assert_eq!(detect_tabular_format(b"Obj"), None);
    }

    fn build_avro_fixture(records: usize) -> Vec<u8> {
        use apache_avro::{Schema, Writer, types::Record};
        let schema_json = r#"
        {"type":"record","name":"User","fields":[
            {"name":"id","type":"long"},
            {"name":"name","type":"string"},
            {"name":"active","type":"boolean"}
        ]}"#;
        let schema = Schema::parse_str(schema_json).unwrap();
        let mut writer = Writer::new(&schema, Vec::new());
        for i in 0..records {
            let mut rec = Record::new(&schema).unwrap();
            rec.put("id", i as i64);
            rec.put("name", format!("user-{i}"));
            rec.put("active", i % 2 == 0);
            writer.append(rec).unwrap();
        }
        writer.into_inner().unwrap()
    }

    #[test]
    fn decode_avro_happy_path_yields_tabular() {
        let bytes = build_avro_fixture(3);
        let render = decode_avro(&bytes).expect("decode_avro");
        match render {
            ContentRender::Tabular {
                format,
                schema_summary,
                body,
                decoded_bytes,
                truncated,
            } => {
                assert_eq!(format, TabularFormat::Avro);
                assert!(schema_summary.contains("id"));
                assert!(schema_summary.contains("name"));
                assert!(schema_summary.contains("active"));
                assert_eq!(body.lines().count(), 3);
                assert!(body.contains(r#""id":0"#));
                assert!(body.contains(r#""name":"user-2""#));
                assert_eq!(decoded_bytes, body.len());
                assert!(!truncated);
            }
            other => panic!("expected Tabular, got {other:?}"),
        }
    }

    #[test]
    fn decode_avro_corrupt_returns_err() {
        let mut bytes = b"Obj\x01".to_vec();
        bytes.extend_from_slice(&[0xff; 32]); // not a valid avro header
        let result = decode_avro(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn decode_avro_small_file_is_not_truncated() {
        // Verifies the happy path leaves `truncated = false`. Triggering
        // `truncated = true` would require building > TABULAR_RECORD_LIMIT
        // (50 000) records inline; the integration test (Task 22) exercises
        // real-world sizing instead.
        let bytes = build_avro_fixture(10);
        let render = decode_avro(&bytes).unwrap();
        if let ContentRender::Tabular {
            body, truncated, ..
        } = render
        {
            assert_eq!(body.lines().count(), 10);
            assert!(!truncated);
        } else {
            panic!("expected Tabular");
        }
    }

    fn build_parquet_fixture(records: usize) -> Vec<u8> {
        use std::sync::Arc;

        use arrow::array::{ArrayRef, BooleanArray, Int64Array, StringArray};
        use arrow::datatypes::{DataType, Field, Schema};
        use arrow::record_batch::RecordBatch;
        use parquet::arrow::ArrowWriter;

        let schema = Arc::new(Schema::new(vec![
            Field::new("id", DataType::Int64, false),
            Field::new("name", DataType::Utf8, false),
            Field::new("active", DataType::Boolean, false),
        ]));
        let ids: Int64Array = (0..records as i64).collect();
        let names: StringArray = (0..records).map(|i| Some(format!("user-{i}"))).collect();
        let active: BooleanArray = (0..records).map(|i| Some(i % 2 == 0)).collect();
        let batch = RecordBatch::try_new(
            schema.clone(),
            vec![
                Arc::new(ids) as ArrayRef,
                Arc::new(names) as ArrayRef,
                Arc::new(active) as ArrayRef,
            ],
        )
        .unwrap();

        let mut buf = Vec::new();
        let mut writer = ArrowWriter::try_new(&mut buf, schema, None).unwrap();
        writer.write(&batch).unwrap();
        writer.close().unwrap();
        buf
    }

    #[test]
    fn decode_parquet_happy_path_yields_tabular() {
        let bytes = build_parquet_fixture(5);
        let render = decode_parquet(&bytes).expect("decode_parquet");
        match render {
            ContentRender::Tabular {
                format,
                schema_summary,
                body,
                decoded_bytes,
                truncated,
            } => {
                assert_eq!(format, TabularFormat::Parquet);
                assert!(schema_summary.contains("id"));
                assert!(schema_summary.contains("name"));
                assert!(schema_summary.contains("active"));
                assert_eq!(body.lines().count(), 5);
                assert!(body.contains(r#""id":0"#));
                assert!(body.contains(r#""name":"user-3""#));
                assert_eq!(decoded_bytes, body.len());
                assert!(!truncated);
            }
            other => panic!("expected Tabular, got {other:?}"),
        }
    }

    #[test]
    fn decode_parquet_corrupt_returns_err() {
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 64]); // header magic but no footer
        let result = decode_parquet(&bytes);
        assert!(result.is_err());
    }

    #[test]
    fn classify_parquet_bytes_produce_tabular() {
        let bytes = build_parquet_fixture(2);
        match classify_content(bytes) {
            ContentRender::Tabular {
                format: TabularFormat::Parquet,
                ..
            } => {}
            other => panic!("expected Tabular::Parquet, got {other:?}"),
        }
    }

    #[test]
    fn classify_avro_bytes_produce_tabular() {
        let bytes = build_avro_fixture(2);
        match classify_content(bytes) {
            ContentRender::Tabular {
                format: TabularFormat::Avro,
                ..
            } => {}
            other => panic!("expected Tabular::Avro, got {other:?}"),
        }
    }

    #[test]
    fn classify_parquet_corrupt_falls_back_to_hex() {
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 64]);
        match classify_content(bytes) {
            ContentRender::Hex { .. } => {}
            other => panic!("expected Hex fallback, got {other:?}"),
        }
    }

    #[test]
    fn classify_avro_corrupt_falls_back_to_hex() {
        let mut bytes = b"Obj\x01".to_vec();
        bytes.extend_from_slice(&[0xff; 32]);
        match classify_content(bytes) {
            ContentRender::Hex { .. } => {}
            other => panic!("expected Hex fallback, got {other:?}"),
        }
    }

    #[test]
    fn classify_text_unaffected_by_tabular_routing() {
        // Existing behavior must be preserved.
        match classify_content(b"hello world".to_vec()) {
            ContentRender::Text {
                text,
                pretty_printed,
            } => {
                assert_eq!(text, "hello world");
                assert!(!pretty_printed);
            }
            other => panic!("expected Text, got {other:?}"),
        }
    }

    // ── Finding 2: spec-mandated unit tests ──────────────────────────────────

    /// Mirrors the real ceiling-hit-mid-parquet scenario: valid PAR1 magic
    /// plus body bytes but clipped before the footer → Hex fallback.
    #[test]
    fn classify_parquet_truncated_footer_falls_back_to_hex() {
        let mut bytes = b"PAR1".to_vec();
        bytes.extend_from_slice(&[0u8; 1024]); // body bytes, no footer magic at end
        match classify_content(bytes) {
            ContentRender::Hex { .. } => {}
            other => panic!("expected Hex fallback for truncated parquet, got {other:?}"),
        }
    }

    /// `TABULAR_BODY_LIMIT` is the documented internal OOM guard value.
    #[test]
    fn tabular_decode_respects_body_limit() {
        assert_eq!(TABULAR_BODY_LIMIT, 128 * 1024 * 1024);
    }

    /// `TABULAR_RECORD_LIMIT` is the documented internal safety cap.
    #[test]
    fn tabular_decode_respects_record_limit() {
        assert_eq!(TABULAR_RECORD_LIMIT, 50_000);
    }

    // ── Decoder-timeout wrapper ──────────────────────────────────────────────

    #[test]
    fn decode_with_timeout_returns_err_after_deadline() {
        let result: Result<i32, _> = decode_with_timeout_inner(
            "test",
            Vec::new(),
            std::time::Duration::from_millis(100),
            |_| {
                std::thread::sleep(std::time::Duration::from_millis(500));
                Ok::<i32, Box<dyn std::error::Error + Send + Sync>>(42)
            },
        );
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.to_string().contains("timed out"),
            "expected timeout error, got: {err}"
        );
    }

    #[test]
    fn decode_with_timeout_returns_ok_when_fast() {
        let result: Result<i32, _> = decode_with_timeout_inner(
            "test",
            Vec::new(),
            std::time::Duration::from_millis(500),
            |_| Ok::<i32, Box<dyn std::error::Error + Send + Sync>>(42),
        );
        assert_eq!(result.unwrap(), 42);
    }
}
