// Consumed by Tasks 5–8
//! Tracer-tab client wrappers and snapshot types.
//!
//! Phase 4 forensic flow: paste a flowfile UUID → submit a lineage
//! query → poll → render the event timeline → optionally fetch per-event
//! content. All helpers map errors via `classify_or_fallback` so the UI
//! layer never sees a raw `NifiError`.

use std::sync::Arc;
use std::time::SystemTime;

use nifi_rust_client::dynamic::types::{LineageDto, LineageEntity, LineageRequestDto};

use crate::client::NifiClient;
use crate::client::classify_or_fallback;
use crate::error::NifiLensError;

/// Preview cap applied to Tracer content fetches. Flowfile bodies
/// above this threshold return `206 Partial Content`; the UI
/// renders the truncated slice and flags the truncation.
pub const PREVIEW_CAP_BYTES: usize = 1 << 20; // 1 MiB

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

#[derive(Debug, Clone)]
pub enum ContentRender {
    Text { pretty: String },
    Hex { first_4k: String },
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
    pub total_bytes: usize,
    pub raw: Arc<[u8]>,
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
    ) -> Result<ContentSnapshot, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            side = side.as_str(),
            "fetching provenance event content",
        );

        let id_str = event_id.to_string();
        let events_api = self.inner.provenanceevents();
        let cluster_node_id = self.inner.cluster_node_id();

        let bytes = match side {
            ContentSide::Input => {
                events_api
                    .get_input_content(&id_str, cluster_node_id, None)
                    .await
            }
            ContentSide::Output => {
                events_api
                    .get_output_content(&id_str, cluster_node_id, None)
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

        let total_bytes = bytes.len();
        let render = classify_content(&bytes);
        let raw: std::sync::Arc<[u8]> = bytes.into();

        Ok(ContentSnapshot {
            event_id,
            side,
            render,
            total_bytes,
            raw,
        })
    }

    /// Fetches the raw content bytes for a provenance event without
    /// classification. Used by the Save path, which writes bytes to disk
    /// and does not need a `ContentRender`.
    ///
    /// Maps `GET /nifi-api/provenance-events/{id}/content/{side}` with
    /// no `Range` header. Errors are mapped to
    /// [`NifiLensError::ProvenanceContentFetchFailed`], same as
    /// `provenance_content`.
    pub async fn provenance_content_raw(
        &self,
        event_id: i64,
        side: ContentSide,
    ) -> Result<Vec<u8>, NifiLensError> {
        tracing::debug!(
            context = %self.context_name(),
            event_id,
            side = side.as_str(),
            "fetching provenance event content (raw, uncapped)",
        );

        let id_str = event_id.to_string();
        let events_api = self.inner.provenanceevents();
        let cluster_node_id = self.inner.cluster_node_id();

        let bytes = match side {
            ContentSide::Input => {
                events_api
                    .get_input_content(&id_str, cluster_node_id, None)
                    .await
            }
            ContentSide::Output => {
                events_api
                    .get_output_content(&id_str, cluster_node_id, None)
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

        Ok(bytes)
    }
}

/// Classifies raw bytes into a [`ContentRender`] variant.
///
/// - Empty slice → [`ContentRender::Empty`]
/// - Valid UTF-8 text → [`ContentRender::Text`] with JSON pretty-printing if parseable
/// - Non-UTF-8 bytes → [`ContentRender::Hex`] with the first 4 KiB hex-dumped
pub(crate) fn classify_content(bytes: &[u8]) -> ContentRender {
    if bytes.is_empty() {
        return ContentRender::Empty;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => {
            let pretty = serde_json::from_slice::<serde_json::Value>(bytes)
                .and_then(|v| serde_json::to_string_pretty(&v))
                .unwrap_or_else(|_| text.to_string());
            ContentRender::Text { pretty }
        }
        Err(_) => ContentRender::Hex {
            first_4k: hex_dump(&bytes[..bytes.len().min(4096)]),
        },
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
        assert!(matches!(classify_content(b""), ContentRender::Empty));
    }

    #[test]
    fn classify_plain_utf8_is_text() {
        match classify_content(b"hello world") {
            ContentRender::Text { pretty } => assert_eq!(pretty, "hello world"),
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_json_is_prettyprinted_text() {
        match classify_content(br#"{"a":1,"b":[2,3]}"#) {
            ContentRender::Text { pretty } => {
                assert!(pretty.contains("\"a\": 1"));
                assert!(pretty.contains('\n'));
            }
            other => panic!("got {other:?}"),
        }
    }

    #[test]
    fn classify_invalid_utf8_is_hex() {
        let bytes = vec![0xff, 0x00, 0x61, 0xfe];
        match classify_content(&bytes) {
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
    async fn provenance_content_raw_fetches_body_without_classification() {
        let server = wiremock::MockServer::start().await;
        wiremock::Mock::given(wiremock::matchers::method("GET"))
            .and(wiremock::matchers::path(
                "/nifi-api/provenance-events/42/content/output",
            ))
            .respond_with(
                wiremock::ResponseTemplate::new(200)
                    .set_body_bytes(b"hello world" as &[u8])
                    .insert_header("content-type", "application/octet-stream"),
            )
            .expect(1)
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let bytes = client
            .provenance_content_raw(42, ContentSide::Output)
            .await
            .expect("fetch should succeed");
        assert_eq!(bytes, b"hello world");
    }
}
