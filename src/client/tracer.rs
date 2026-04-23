//! Tracer-tab client wrappers and snapshot types.
//!
//! Phase 4 forensic flow: paste a flowfile UUID → submit a lineage
//! query → poll → render the event timeline → optionally fetch per-event
//! content. All helpers map errors via `classify_or_fallback` so the UI
//! layer never sees a raw `NifiError`.

use std::time::SystemTime;

use nifi_rust_client::dynamic::types::{LineageDto, LineageEntity, LineageRequestDto};

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

/// Byte cap on the inline content-pane preview in the Tracer tab.
/// The full-screen content viewer modal uses `provenance_content_range`
/// with its own streaming ceiling; this constant only bounds the inline
/// mini-preview.
pub const INLINE_PREVIEW_BYTES: usize = 8 * 1024;

/// Direction of a content claim on a provenance event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentSide {
    Input,
    Output,
}

impl ContentSide {
    pub fn as_str(self) -> &'static str {
        match self {
            ContentSide::Input => "input",
            ContentSide::Output => "output",
        }
    }
}

#[derive(Debug, Clone)]
pub struct LineageSnapshot {
    pub events: Vec<ProvenanceEventSummary>,
    pub percent_completed: u8,
    pub finished: bool,
}

#[derive(Debug, Clone)]
pub struct ProvenanceEventSummary {
    pub event_id: i64,
    pub event_time_iso: String,
    pub event_type: String,
    pub component_id: String,
    pub component_name: String,
    pub component_type: String,
    pub group_id: String,
    pub flow_file_uuid: String,
    pub relationship: Option<String>,
    pub details: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProvenanceEventDetail {
    pub summary: ProvenanceEventSummary,
    pub attributes: Vec<AttributeTriple>,
    pub transit_uri: Option<String>,
    pub input_available: bool,
    pub output_available: bool,
    /// Content claim size (bytes) for the input side, when the NiFi
    /// DTO exposes it. None when the field is absent or no input
    /// content claim exists.
    pub input_size: Option<u64>,
    /// Content claim size (bytes) for the output side.
    pub output_size: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttributeTriple {
    pub key: String,
    pub previous: Option<String>,
    pub current: Option<String>,
}

impl AttributeTriple {
    pub fn is_changed(&self) -> bool {
        self.previous != self.current
    }
}

#[derive(Debug, Clone)]
pub enum LineagePoll {
    Running { percent: u8 },
    Finished(LineageSnapshot),
}

/// Identifies the binary container format for a [`ContentRender::Tabular`] payload.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabularFormat {
    /// Apache Parquet columnar file (magic bytes `PAR1`).
    Parquet,
    /// Apache Avro Object Container File (magic bytes `Obj\x01`).
    Avro,
}

impl TabularFormat {
    /// Lowercase format name suitable for footer chips and log messages.
    pub fn label(self) -> &'static str {
        match self {
            TabularFormat::Parquet => "parquet",
            TabularFormat::Avro => "avro",
        }
    }
}

#[derive(Debug, Clone, Default)]
pub enum ContentRender {
    /// Valid UTF-8 body. `pretty_printed` is true iff JSON pretty-
    /// printing succeeded AND produced bytes different from the
    /// original. `text` is always the authoritative render target.
    Text { text: String, pretty_printed: bool },
    /// Decoded Parquet or Avro container. `body` is JSON-Lines, one
    /// record per line. `schema_summary` is one column per line in
    /// the format's native type names.
    Tabular {
        format: TabularFormat,
        schema_summary: String,
        body: String,
        /// Byte length of the decoded JSON-Lines `body`. This is the
        /// quantity the modal compares against the diff cap, not the byte
        /// length of the source Parquet/Avro container. Maintained equal
        /// to `body.len()` by every code path that constructs this
        /// variant; the cached field exists so the modal can size the diff
        /// cap chip in O(1) without re-counting bytes.
        decoded_bytes: usize,
        truncated: bool,
    },
    /// Non-UTF-8 body, hex dump of up to the first 4 KiB.
    Hex { first_4k: String },
    /// Empty body.
    #[default]
    Empty,
}

#[derive(Debug, Clone)]
pub struct LatestEventsSnapshot {
    pub component_id: String,
    pub component_label: String,
    pub events: Vec<ProvenanceEventSummary>,
    pub fetched_at: SystemTime,
}

#[derive(Debug, Clone)]
pub struct ContentSnapshot {
    pub event_id: i64,
    pub side: ContentSide,
    pub render: ContentRender,
    pub bytes_fetched: usize,
    pub truncated: bool,
}

/// Byte-range slice of a provenance event's content. Produced by
/// `NifiClient::provenance_content_range`.
#[derive(Debug, Clone)]
pub struct ContentRangeSnapshot {
    pub event_id: i64,
    pub side: ContentSide,
    /// Absolute byte offset into the content claim for the first byte
    /// of `bytes`.
    pub offset: usize,
    /// Response body bytes. Length may be less than the requested len
    /// when the server reached end-of-claim.
    pub bytes: Vec<u8>,
    /// True iff `bytes.len() < requested_len`, i.e. the server sent
    /// fewer bytes than asked for — treated as end-of-claim by the
    /// modal reducer.
    pub eof: bool,
}

impl NifiClient {
    /// Fetches the latest cached provenance events for a given component.
    ///
    /// Maps `GET /nifi-api/provenance-events/latest/{component_id}?limit={limit}`
    /// into a [`LatestEventsSnapshot`]. Errors are classified via
    /// `classify_or_fallback` so callers only see typed `NifiLensError` variants.
    pub async fn latest_events(
        &self,
        component_id: &str,
        limit: i32,
    ) -> Result<LatestEventsSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            component_id,
            limit,
            "fetching /provenance-events/latest",
        );

        let dto = self
            .inner
            .provenanceevents()
            .get_latest_provenance_events(component_id, Some(limit))
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LatestProvenanceEventsFailed {
                        context: self.context_name().to_string(),
                        component_id: component_id.to_string(),
                        source,
                    }
                })
            })?;

        let events = dto
            .provenance_events
            .unwrap_or_default()
            .into_iter()
            .map(summary_from_dto)
            .collect::<Vec<_>>();

        let component_label = events
            .first()
            .map(|e| format!("{} \u{00b7} {}", e.component_name, e.group_id))
            .unwrap_or_else(|| component_id.to_string());

        Ok(LatestEventsSnapshot {
            component_id: component_id.to_string(),
            component_label,
            events,
            fetched_at: SystemTime::now(),
        })
    }

    /// Submits a lineage query for the given flowfile UUID.
    ///
    /// Maps `POST /nifi-api/provenance/lineage` with a `FLOWFILE` request body
    /// and returns `(query_id, cluster_node_id)`. The `cluster_node_id` is
    /// `Some` when NiFi runs in cluster mode and must be passed to
    /// [`poll_lineage`](Self::poll_lineage) and
    /// [`delete_lineage`](Self::delete_lineage).
    pub async fn submit_lineage(
        &self,
        flow_file_uuid: &str,
    ) -> Result<(String, Option<String>), NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            flow_file_uuid,
            "submitting lineage query",
        );

        let mut request = LineageRequestDto::default();
        request.lineage_request_type = Some("FLOWFILE".to_string());
        request.uuid = Some(flow_file_uuid.to_string());
        // In clustered mode NiFi rejects lineage submissions that don't
        // name a node; DynamicClient pins this at login.
        request.cluster_node_id = self.inner.cluster_node_id().map(String::from);

        let mut lineage_dto = LineageDto::default();
        lineage_dto.request = Some(request);

        let body = LineageEntity {
            lineage: Some(lineage_dto),
        };

        let dto = self
            .inner
            .provenance()
            .submit_lineage_request(&body)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQuerySubmitFailed {
                        context: self.context_name().to_string(),
                        uuid: flow_file_uuid.to_string(),
                        source,
                    }
                })
            })?;

        let cluster_node_id = dto.request.and_then(|r| r.cluster_node_id);

        let query_id = dto
            .id
            .ok_or_else(|| NifiLensError::LineageQuerySubmitFailed {
                context: self.context_name().to_string(),
                uuid: flow_file_uuid.to_string(),
                source: "server returned no query id".into(),
            })?;

        Ok((query_id, cluster_node_id))
    }

    /// Polls a lineage query and returns [`LineagePoll::Running`] or
    /// [`LineagePoll::Finished`].
    ///
    /// Maps `GET /nifi-api/provenance/lineage/{id}`. Errors are classified via
    /// `classify_or_fallback`.
    pub async fn poll_lineage(
        &self,
        query_id: &str,
        cluster_node_id: Option<&str>,
    ) -> Result<LineagePoll, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id,
            "polling lineage query",
        );

        let dto = self
            .inner
            .provenance()
            .get_lineage(query_id, cluster_node_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQueryPollFailed {
                        context: self.context_name().to_string(),
                        query_id: query_id.to_string(),
                        source,
                    }
                })
            })?;

        let percent = dto.percent_completed.unwrap_or(0).clamp(0, 100) as u8;
        let finished = dto.finished.unwrap_or(false);

        if finished {
            let nodes = dto.results.and_then(|r| r.nodes).unwrap_or_default();
            let events = nodes_to_events(nodes);
            Ok(LineagePoll::Finished(LineageSnapshot {
                events,
                percent_completed: percent,
                finished: true,
            }))
        } else {
            Ok(LineagePoll::Running { percent })
        }
    }

    /// Fetches the full detail of a single provenance event by its numeric ID.
    ///
    /// Maps `GET /nifi-api/provenance-events/{id}` into a [`ProvenanceEventDetail`].
    /// Errors are classified via `classify_or_fallback`.
    pub async fn get_provenance_event(
        &self,
        event_id: i64,
    ) -> Result<ProvenanceEventDetail, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            "fetching /provenance-events/{event_id}",
        );

        let dto = self
            .inner
            .provenanceevents()
            .get_provenance_event(&event_id.to_string(), self.inner.cluster_node_id())
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::ProvenanceEventFetchFailed {
                        context: self.context_name().to_string(),
                        event_id,
                        source,
                    }
                })
            })?;

        let input_available = dto.input_content_available.unwrap_or(false);
        let output_available = dto.output_content_available.unwrap_or(false);
        let transit_uri = dto.transit_uri.clone();
        let input_size = dto
            .input_content_claim_file_size_bytes
            .and_then(|n| u64::try_from(n).ok());
        let output_size = dto
            .output_content_claim_file_size_bytes
            .and_then(|n| u64::try_from(n).ok());
        let attributes = dto
            .attributes
            .as_ref()
            .map(|attrs| {
                attrs
                    .iter()
                    .map(|a| AttributeTriple {
                        key: a.name.clone().unwrap_or_default(),
                        previous: a.previous_value.clone(),
                        current: a.value.clone(),
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();

        Ok(ProvenanceEventDetail {
            summary: summary_from_dto(dto),
            attributes,
            transit_uri,
            input_available,
            output_available,
            input_size,
            output_size,
        })
    }

    /// Deletes a lineage query from the NiFi server.
    ///
    /// Maps `DELETE /nifi-api/provenance/lineage/{id}`. Errors are classified
    /// via `classify_or_fallback`. Delete failures are typically logged at warn
    /// level and never surfaced to the user.
    pub async fn delete_lineage(
        &self,
        query_id: &str,
        cluster_node_id: Option<&str>,
    ) -> Result<(), NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            query_id,
            "deleting lineage query",
        );

        self.inner
            .provenance()
            .delete_lineage(query_id, cluster_node_id)
            .await
            .map_err(|err| {
                classify_or_fallback(self.context_name(), Box::new(err), |source| {
                    NifiLensError::LineageQueryDeleteFailed {
                        context: self.context_name().to_string(),
                        query_id: query_id.to_string(),
                        source,
                    }
                })
            })?;

        Ok(())
    }

    /// Fetches the raw content bytes for a provenance event and classifies them.
    ///
    /// Maps `GET /nifi-api/provenance-events/{id}/content/input` or `.../output`
    /// depending on `side`. The raw bytes are classified by `classify_content`
    /// into a [`ContentRender`] variant. Errors are mapped to
    /// [`NifiLensError::ProvenanceContentFetchFailed`].
    pub async fn provenance_content(
        &self,
        event_id: i64,
        side: ContentSide,
        max_bytes: Option<usize>,
    ) -> Result<ContentSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            side = side.as_str(),
            max_bytes = ?max_bytes,
            "fetching provenance event content",
        );

        let id_str = event_id.to_string();
        let events_api = self.inner.provenanceevents();
        let cluster_node_id = self.inner.cluster_node_id();
        // NiFi treats the Range header's end value as exclusive (returns
        // `last - first` bytes), unlike RFC 7233's inclusive semantics. Ask
        // for one byte past the cap so we receive exactly `n` bytes back.
        let range = max_bytes.map(|n| format!("bytes=0-{n}"));
        let range_ref = range.as_deref();

        let bytes = match side {
            ContentSide::Input => {
                events_api
                    .get_input_content(&id_str, cluster_node_id, range_ref)
                    .await
            }
            ContentSide::Output => {
                events_api
                    .get_output_content(&id_str, cluster_node_id, range_ref)
                    .await
            }
        }
        .map_err(|err| {
            classify_or_fallback(self.context_name(), Box::new(err), |source| {
                NifiLensError::ProvenanceContentFetchFailed {
                    context: self.context_name().to_string(),
                    event_id,
                    side: side.as_str(),
                    source,
                }
            })
        })?;

        let bytes_fetched = bytes.len();
        let truncated = matches!(max_bytes, Some(n) if bytes_fetched >= n);
        let render = classify_content(bytes);

        Ok(ContentSnapshot {
            event_id,
            side,
            render,
            bytes_fetched,
            truncated,
        })
    }

    /// Opens a streaming body for the content bytes of a provenance event.
    ///
    /// Maps `GET /nifi-api/provenance-events/{id}/content/{side}` with
    /// no `Range` header and returns the response as a
    /// [`nifi_rust_client::BytesStream`] so callers can sink arbitrarily
    /// large flowfile bodies to disk without buffering them in memory.
    ///
    /// Errors on the initial status-line exchange are mapped to
    /// [`NifiLensError::ProvenanceContentFetchFailed`]. Transport errors
    /// that terminate the stream mid-body are surfaced on the stream
    /// itself as `Result<Bytes, NifiError>`; the caller decides how to
    /// report them.
    pub async fn provenance_content_stream(
        &self,
        event_id: i64,
        side: ContentSide,
    ) -> Result<nifi_rust_client::BytesStream, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            side = side.as_str(),
            "opening provenance event content stream (uncapped)",
        );

        let id_str = event_id.to_string();
        let events_api = self.inner.provenanceevents();
        let cluster_node_id = self.inner.cluster_node_id();

        let stream = match side {
            ContentSide::Input => {
                events_api
                    .get_input_content_stream(&id_str, cluster_node_id, None)
                    .await
            }
            ContentSide::Output => {
                events_api
                    .get_output_content_stream(&id_str, cluster_node_id, None)
                    .await
            }
        }
        .map_err(|err| {
            classify_or_fallback(self.context_name(), Box::new(err), |source| {
                NifiLensError::ProvenanceContentFetchFailed {
                    context: self.context_name().to_string(),
                    event_id,
                    side: side.as_str(),
                    source,
                }
            })
        })?;

        Ok(stream)
    }

    /// Fetches a byte range `[offset, offset + len)` of a provenance
    /// event's content. NiFi treats the `Range` header's end value as
    /// exclusive (unlike RFC 7233), so we ask for `bytes={offset}-{end}`
    /// with `end = offset + len` — the server returns exactly `len` bytes
    /// unless it reaches EOF first.
    ///
    /// Errors map to `NifiLensError::ProvenanceContentFetchFailed`.
    pub async fn provenance_content_range(
        &self,
        event_id: i64,
        side: ContentSide,
        offset: usize,
        len: usize,
    ) -> Result<ContentRangeSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            side = side.as_str(),
            offset,
            len,
            "fetching provenance event content range",
        );

        let id_str = event_id.to_string();
        let events_api = self.inner.provenanceevents();
        let cluster_node_id = self.inner.cluster_node_id();
        // `offset + len` can overflow on pathological inputs; saturate so the
        // server returns a short read (treated as EOF) rather than panic.
        let end = offset.saturating_add(len);
        let range = format!("bytes={offset}-{end}");

        let bytes = match side {
            ContentSide::Input => {
                events_api
                    .get_input_content(&id_str, cluster_node_id, Some(&range))
                    .await
            }
            ContentSide::Output => {
                events_api
                    .get_output_content(&id_str, cluster_node_id, Some(&range))
                    .await
            }
        }
        .map_err(|err| {
            classify_or_fallback(self.context_name(), Box::new(err), |source| {
                NifiLensError::ProvenanceContentFetchFailed {
                    context: self.context_name().to_string(),
                    event_id,
                    side: side.as_str(),
                    source,
                }
            })
        })?;

        let eof = bytes.len() < len;

        Ok(ContentRangeSnapshot {
            event_id,
            side,
            offset,
            bytes,
            eof,
        })
    }
}

/// Classifies raw bytes into a [`ContentRender`] variant.
///
/// - Empty slice → [`ContentRender::Empty`]
/// - Valid UTF-8 text → [`ContentRender::Text`] with JSON pretty-printing if parseable
/// - Non-UTF-8 bytes → [`ContentRender::Hex`] with the first 4 KiB hex-dumped
pub fn classify_content(bytes: Vec<u8>) -> ContentRender {
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
const TABULAR_BODY_LIMIT: usize = 128 * 1024 * 1024;

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

/// Converts lineage graph nodes into a chronological list of event summaries.
///
/// Filters to nodes whose `type` field equals `"EVENT"`, sorts ascending by
/// `millis`, and maps each to a [`ProvenanceEventSummary`].
pub(crate) fn nodes_to_events(
    nodes: Vec<nifi_rust_client::dynamic::types::ProvenanceNodeDto>,
) -> Vec<ProvenanceEventSummary> {
    let mut events: Vec<_> = nodes
        .into_iter()
        .filter(|n| n.r#type.as_deref() == Some("EVENT"))
        .collect();
    events.sort_by_key(|n| n.millis.unwrap_or(0));
    events
        .into_iter()
        .map(|n| ProvenanceEventSummary {
            // For EVENT-type nodes, ProvenanceNodeDto.id is the numeric
            // provenance event id serialized as a string; parse it so
            // the detail fetch can target the right event.
            event_id: n.id.as_deref().and_then(|s| s.parse().ok()).unwrap_or(0),
            event_time_iso: n.timestamp.unwrap_or_default(),
            event_type: n.event_type.unwrap_or_default(),
            component_id: String::new(),
            component_name: String::new(),
            component_type: n.component_type.unwrap_or_default(),
            group_id: String::new(),
            flow_file_uuid: n.flow_file_uuid.unwrap_or_default(),
            relationship: None,
            details: None,
        })
        .collect()
}

pub(crate) fn summary_from_dto(
    dto: nifi_rust_client::dynamic::types::ProvenanceEventDto,
) -> ProvenanceEventSummary {
    ProvenanceEventSummary {
        event_id: dto.event_id.unwrap_or(0),
        event_time_iso: dto.event_time.unwrap_or_default(),
        event_type: dto.event_type.unwrap_or_default(),
        component_id: dto.component_id.unwrap_or_default(),
        component_name: dto.component_name.unwrap_or_default(),
        component_type: dto.component_type.unwrap_or_default(),
        group_id: dto.group_id.unwrap_or_default(),
        flow_file_uuid: dto.flow_file_uuid.unwrap_or_default(),
        relationship: dto.relationship,
        details: dto.details,
    }
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
    fn nodes_to_events_parses_event_id_from_node_id() {
        use nifi_rust_client::dynamic::types::ProvenanceNodeDto;

        let mut earlier = ProvenanceNodeDto::default();
        earlier.r#type = Some("EVENT".to_string());
        earlier.id = Some("42".to_string());
        earlier.event_type = Some("RECEIVE".to_string());
        earlier.millis = Some(1_000);
        earlier.flow_file_uuid = Some("uuid-a".to_string());

        let mut later = ProvenanceNodeDto::default();
        later.r#type = Some("EVENT".to_string());
        later.id = Some("99".to_string());
        later.event_type = Some("DROP".to_string());
        later.millis = Some(2_000);
        later.flow_file_uuid = Some("uuid-b".to_string());

        let mut flowfile = ProvenanceNodeDto::default();
        flowfile.r#type = Some("FLOWFILE".to_string());
        flowfile.id = Some("should-be-filtered".to_string());

        let events = nodes_to_events(vec![later, earlier, flowfile]);

        assert_eq!(events.len(), 2, "FLOWFILE nodes must be filtered out");
        assert_eq!(
            events[0].event_id, 42,
            "sorted ascending by millis; event id parsed from node id"
        );
        assert_eq!(events[0].event_type, "RECEIVE");
        assert_eq!(events[0].flow_file_uuid, "uuid-a");
        assert_eq!(events[1].event_id, 99);
        assert_eq!(events[1].event_type, "DROP");
    }

    #[test]
    fn nodes_to_events_unparseable_id_falls_back_to_zero() {
        use nifi_rust_client::dynamic::types::ProvenanceNodeDto;

        let mut node = ProvenanceNodeDto::default();
        node.r#type = Some("EVENT".to_string());
        node.id = Some("not-a-number".to_string());
        node.event_type = Some("RECEIVE".to_string());
        node.millis = Some(0);

        let events = nodes_to_events(vec![node]);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].event_id, 0);
    }

    /// Build a `NifiClient` backed by a wiremock `MockServer`.
    ///
    /// Mounts a `/nifi-api/flow/about` stub returning version `2.6.0` so that
    /// `detect_version` succeeds. The caller mounts additional stubs before
    /// (or after) calling this helper.
    async fn test_client(server: &wiremock::MockServer) -> crate::client::NifiClient {
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path("/nifi-api/flow/about"))
            .respond_with(
                wiremock::ResponseTemplate::new(200).set_body_json(
                    serde_json::json!({"about": {"version": "2.6.0", "title": "NiFi"}}),
                ),
            )
            .mount(server)
            .await;

        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic client");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        crate::client::NifiClient::from_parts(inner, "test", version)
    }

    #[tokio::test]
    async fn provenance_content_stream_yields_full_body() {
        use futures::StreamExt;

        let server = wiremock::MockServer::start().await;
        let payload: Vec<u8> = (0..=255u8).cycle().take(4096).collect();
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/42/content/output",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(payload.clone())
                    .insert_header("content-type", "application/octet-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let mut stream = client
            .provenance_content_stream(42, ContentSide::Output)
            .await
            .expect("stream open should succeed");

        let mut collected = Vec::new();
        while let Some(chunk) = stream.next().await {
            collected.extend_from_slice(&chunk.expect("chunk"));
        }
        assert_eq!(collected, payload);
    }

    #[tokio::test]
    async fn provenance_content_without_cap_does_not_send_range_header() {
        let server = wiremock::MockServer::start().await;

        // Strict: no request with a Range header should arrive.
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/7/content/input",
            ))
            .and(wiremock::matchers::header_exists("range"))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .expect(0)
            .mount(&server)
            .await;

        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/7/content/input",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(b"abcd" as &[u8])
                    .insert_header("content-type", "text/plain"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let snap = client
            .provenance_content(7, ContentSide::Input, None)
            .await
            .expect("fetch should succeed");
        assert_eq!(snap.bytes_fetched, 4);
        assert!(!snap.truncated);
    }

    #[tokio::test]
    async fn provenance_content_with_cap_sends_range_and_marks_truncated() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/8/content/output",
            ))
            .and(wiremock::matchers::header("range", "bytes=0-1024"))
            .respond_with(
                wiremock::ResponseTemplate::new(206)
                    .set_body_bytes(vec![b'x'; 1024])
                    .insert_header("content-type", "application/octet-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let snap = client
            .provenance_content(8, ContentSide::Output, Some(1024))
            .await
            .expect("fetch should succeed");
        assert_eq!(snap.bytes_fetched, 1024);
        assert!(snap.truncated);
    }

    #[tokio::test]
    async fn provenance_content_with_cap_under_body_size_not_truncated() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/9/content/output",
            ))
            .and(wiremock::matchers::header("range", "bytes=0-1024"))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(vec![b'x'; 800])
                    .insert_header("content-type", "application/octet-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let snap = client
            .provenance_content(9, ContentSide::Output, Some(1024))
            .await
            .expect("fetch should succeed");
        assert_eq!(snap.bytes_fetched, 800);
        assert!(!snap.truncated);
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

    #[tokio::test]
    async fn provenance_content_range_sends_correct_range_header() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/42/content/output",
            ))
            .and(wiremock::matchers::header("Range", "bytes=1024-2048"))
            .respond_with(wiremock::ResponseTemplate::new(206).set_body_bytes(vec![b'x'; 1024]))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let snap = client
            .provenance_content_range(42, ContentSide::Output, 1024, 1024)
            .await
            .unwrap();
        assert_eq!(snap.offset, 1024);
        assert_eq!(snap.bytes.len(), 1024);
        assert!(!snap.eof);
    }

    #[tokio::test]
    async fn provenance_content_range_short_read_sets_eof() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/9/content/input",
            ))
            .respond_with(wiremock::ResponseTemplate::new(206).set_body_bytes(vec![b'a'; 300]))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let snap = client
            .provenance_content_range(9, ContentSide::Input, 0, 1024)
            .await
            .unwrap();
        assert_eq!(snap.bytes.len(), 300);
        assert!(snap.eof);
    }

    #[tokio::test]
    async fn provenance_content_range_failure_maps_to_typed_error() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/3/content/input",
            ))
            .respond_with(wiremock::ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let err = client
            .provenance_content_range(3, ContentSide::Input, 0, 512)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            crate::error::NifiLensError::ProvenanceContentFetchFailed { .. }
        ));
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
}
