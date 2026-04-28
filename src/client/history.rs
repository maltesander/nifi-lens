//! Helpers for `/flow/history` (action audit) and `/flow/{type}/{id}/status/history`
//! (sparkline series). PR 1 of the browser-forensics work covers action history;
//! status-history support is added in PR 2.

use std::pin::Pin;
use std::time::SystemTime;

use nifi_rust_client::NifiError;
use nifi_rust_client::pagination::{
    HistoryFilter, HistoryPage, HistoryPaginator, flow_history_dynamic,
};

use crate::client::NifiClient;
use crate::error::NifiLensError;

/// Local alias for the future type used by upstream's
/// `flow_history_dynamic` closure. Upstream's `BoxedFetchFuture` alias is
/// crate-private, so the wrapper signature spells the equivalent shape
/// out by hand.
type BoxedHistoryFuture<'a> =
    Pin<Box<dyn core::future::Future<Output = Result<HistoryPage, NifiError>> + Send + 'a>>;

/// Component kinds that participate in the Browser detail panes. Used by
/// the action-history modal to label the source on rows and (in PR 2) to
/// dispatch status-history fetches per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ComponentKind {
    Processor,
    ProcessGroup,
    Connection,
    ControllerService,
    Port,
}

impl ComponentKind {
    /// Short display label used in the action-history modal's `type` column.
    pub fn label(self) -> &'static str {
        match self {
            Self::Processor => "Processor",
            Self::ProcessGroup => "ProcessGroup",
            Self::Connection => "Connection",
            Self::ControllerService => "ControllerService",
            Self::Port => "Port",
        }
    }
}

/// Build a `HistoryPaginator` filtered by `source_id`. Returns the
/// paginator unchanged so the caller drives paging themselves.
///
/// Callers must hold the `DynamicClient` reference for the paginator's
/// lifetime — `next_page` borrows the inner client.
pub fn flow_actions_paginator<'a>(
    client: &'a nifi_rust_client::dynamic::DynamicClient,
    source_id: &str,
    page_size: u32,
) -> HistoryPaginator<impl FnMut(u32, u32) -> BoxedHistoryFuture<'a> + 'a> {
    let filter = HistoryFilter {
        source_id: Some(source_id.to_string()),
        ..HistoryFilter::default()
    };
    flow_history_dynamic(client, filter, page_size)
}

/// One time-bucket of NiFi status history. NiFi default cadence is one
/// bucket per 5 minutes; the renderer just consumes whatever the server
/// emits without enforcing a cadence.
#[derive(Debug, Clone)]
pub struct Bucket {
    /// Bucket end-time as reported by NiFi. Parsed from the
    /// `MM/DD/YYYY HH:MM:SS UTC` string the server returns. Falls back
    /// to `SystemTime::now()` when the server omits the field.
    pub timestamp: SystemTime,
    /// `flowFilesIn` metric value. Negative server values clamp to 0.
    pub in_count: u64,
    /// `flowFilesOut` metric value. Same clamping.
    pub out_count: u64,
    /// `flowFilesQueued` metric — populated for ProcessGroup /
    /// Connection kinds; `None` for Processor.
    pub queued_count: Option<u64>,
    /// `taskNanos` metric — populated for Processor kind; `None` for
    /// ProcessGroup / Connection.
    pub task_time_ns: Option<u64>,
}

/// Reduced status-history payload — only the metrics the sparkline
/// strip renders. Buckets are in NiFi-emit order (oldest first, newest
/// last); the renderer right-aligns and truncates from the left.
#[derive(Debug, Clone)]
pub struct StatusHistorySeries {
    pub buckets: Vec<Bucket>,
    pub generated_at: SystemTime,
}

/// Fetch the status-history series for `(kind, id)`. Dispatches to the
/// per-kind generated function and reduces the raw `StatusHistoryEntity`
/// down to a metric-keyed `StatusHistorySeries`.
///
/// 404 responses map to `NifiLensError::SparklineEndpointMissing` so
/// the worker can branch on that variant and emit
/// `AppEvent::SparklineEndpointMissing` instead of warn-logging. Other
/// failures wrap the underlying `NifiError` in
/// `NifiLensError::StatusHistoryFetchFailed`.
///
/// `ControllerService` and `Port` kinds return
/// `NifiLensError::SparklineUnsupportedKind` — there is no
/// `/status/history` endpoint for those component types.
pub async fn status_history(
    client: &NifiClient,
    kind: ComponentKind,
    id: &str,
) -> Result<StatusHistorySeries, NifiLensError> {
    let entity = match kind {
        ComponentKind::Processor => client.flow().get_processor_status_history(id).await,
        ComponentKind::ProcessGroup => client.flow().get_process_group_status_history(id).await,
        ComponentKind::Connection => client.flow().get_connection_status_history(id).await,
        ComponentKind::ControllerService | ComponentKind::Port => {
            return Err(NifiLensError::SparklineUnsupportedKind {
                kind: kind.label().to_string(),
            });
        }
    };
    match entity {
        Ok(e) => Ok(reduce_status_history(e, kind)),
        Err(err) => Err(map_status_history_error(id, err)),
    }
}

/// True iff `err` represents a 404 from `/status/history`. The worker
/// branches on this to choose between an `EndpointMissing` emit (which
/// the reducer renders as a sticky muted "no history yet" banner) and
/// a generic warn-log.
pub fn is_status_history_endpoint_missing(err: &NifiLensError) -> bool {
    matches!(err, NifiLensError::SparklineEndpointMissing { .. })
}

fn map_status_history_error(id: &str, err: NifiError) -> NifiLensError {
    let dbg = format!("{err:?}");
    if matches!(err, NifiError::NotFound { .. }) || dbg.contains("404") {
        return NifiLensError::SparklineEndpointMissing {
            id: id.to_string(),
            source: Box::new(err),
        };
    }
    NifiLensError::StatusHistoryFetchFailed {
        id: id.to_string(),
        source: Box::new(err),
    }
}

fn reduce_status_history(
    entity: nifi_rust_client::dynamic::types::StatusHistoryEntity,
    kind: ComponentKind,
) -> StatusHistorySeries {
    let dto = entity.status_history.unwrap_or_default();
    let snapshots = dto.aggregate_snapshots.unwrap_or_default();
    let buckets = snapshots
        .into_iter()
        .map(|snap| {
            let metrics = snap.status_metrics.unwrap_or_default();
            let in_count = pick_metric(&metrics, "flowFilesIn").unwrap_or(0);
            let out_count = pick_metric(&metrics, "flowFilesOut").unwrap_or(0);
            let (queued_count, task_time_ns) = match kind {
                ComponentKind::Processor => (None, pick_metric(&metrics, "taskNanos")),
                ComponentKind::ProcessGroup | ComponentKind::Connection => {
                    (pick_metric(&metrics, "flowFilesQueued"), None)
                }
                ComponentKind::ControllerService | ComponentKind::Port => (None, None),
            };
            let timestamp = snap
                .timestamp
                .as_deref()
                .and_then(parse_nifi_timestamp_to_systemtime)
                .unwrap_or_else(SystemTime::now);
            Bucket {
                timestamp,
                in_count,
                out_count,
                queued_count,
                task_time_ns,
            }
        })
        .collect();
    let generated_at = dto
        .generated
        .as_deref()
        .and_then(parse_nifi_timestamp_to_systemtime)
        .unwrap_or_else(SystemTime::now);
    StatusHistorySeries {
        buckets,
        generated_at,
    }
}

fn pick_metric(metrics: &std::collections::HashMap<String, Option<i64>>, key: &str) -> Option<u64> {
    metrics
        .get(key)
        .and_then(|v| v.as_ref())
        .map(|v| (*v).max(0) as u64)
}

fn parse_nifi_timestamp_to_systemtime(raw: &str) -> Option<SystemTime> {
    let dt = crate::timestamp::parse_nifi_timestamp(raw)?;
    let unix = dt.unix_timestamp();
    if unix < 0 {
        return None;
    }
    Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(unix as u64))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tokio::sync::RwLock;
    use wiremock::matchers::{method, path, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    async fn test_client(server: &MockServer) -> Arc<RwLock<NifiClient>> {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
                {"about": {"version": "2.6.0", "title": "NiFi"}}
            )))
            .mount(server)
            .await;
        let inner = nifi_rust_client::NifiClientBuilder::new(&server.uri())
            .expect("builder")
            .build_dynamic()
            .expect("dynamic");
        inner.detect_version().await.expect("detect_version");
        let version = semver::Version::parse("2.6.0").expect("parse");
        Arc::new(RwLock::new(NifiClient::from_parts(inner, "test", version)))
    }

    fn action(id: i32, source_id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "sourceId": source_id,
            "timestamp": "2026-04-27T10:00:00Z",
            "action": {
                "id": id,
                "operation": "Configure",
                "sourceId": source_id,
                "sourceName": "test",
                "sourceType": "Processor",
                "userIdentity": "alice",
                "timestamp": "2026-04-27T10:00:00Z"
            }
        })
    }

    #[tokio::test(flavor = "current_thread")]
    async fn paginator_filters_by_source_id_and_stitches_pages() {
        let server = MockServer::start().await;
        // Page 1: 2 actions out of 3 total.
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .and(query_param("offset", "0"))
            .and(query_param("count", "2"))
            .and(query_param("sourceId", "proc-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "history": {
                    "total": 3,
                    "actions": [action(10, "proc-1"), action(11, "proc-1")]
                }
            })))
            .mount(&server)
            .await;
        // Page 2: 1 action remaining.
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .and(query_param("offset", "2"))
            .and(query_param("count", "2"))
            .and(query_param("sourceId", "proc-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "history": {
                    "total": 3,
                    "actions": [action(12, "proc-1")]
                }
            })))
            .mount(&server)
            .await;

        let client_arc = test_client(&server).await;
        let guard = client_arc.read().await;
        // NifiClient: Deref<Target = DynamicClient>, so &guard auto-derefs.
        let mut p = flow_actions_paginator(&guard, "proc-1", 2);
        let page1 = p.next_page().await.expect("page1").expect("some");
        assert_eq!(page1.len(), 2);
        assert_eq!(page1[0].id, Some(10));
        let page2 = p.next_page().await.expect("page2").expect("some");
        assert_eq!(page2.len(), 1);
        assert_eq!(page2[0].id, Some(12));
        let exhausted = p.next_page().await.expect("exhausted");
        assert!(exhausted.is_none(), "third page must be None");
    }

    #[test]
    fn component_kind_labels() {
        assert_eq!(ComponentKind::Processor.label(), "Processor");
        assert_eq!(ComponentKind::Port.label(), "Port");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_history_processor_returns_reduced_series() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/processors/proc-1/status/history"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "statusHistory": {
                    "generated": "04/27/2026 10:00:00 UTC",
                    "fieldDescriptors": [
                        {"field": "flowFilesIn", "label": "FlowFiles In", "formatter": "COUNT"},
                        {"field": "flowFilesOut", "label": "FlowFiles Out", "formatter": "COUNT"},
                        {"field": "taskNanos", "label": "Total Task Time (nanos)", "formatter": "DURATION"}
                    ],
                    "aggregateSnapshots": [
                        {"timestamp": "04/27/2026 09:55:00 UTC",
                         "statusMetrics": {"flowFilesIn": 10, "flowFilesOut": 9, "taskNanos": 1500000}},
                        {"timestamp": "04/27/2026 10:00:00 UTC",
                         "statusMetrics": {"flowFilesIn": 12, "flowFilesOut": 11, "taskNanos": 1700000}}
                    ]
                }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let guard = client.read().await;
        let series = status_history(&guard, ComponentKind::Processor, "proc-1")
            .await
            .expect("series");
        assert_eq!(series.buckets.len(), 2);
        assert_eq!(series.buckets[0].in_count, 10);
        assert_eq!(series.buckets[0].out_count, 9);
        assert_eq!(series.buckets[0].task_time_ns, Some(1_500_000));
        assert_eq!(series.buckets[0].queued_count, None);
        assert_eq!(series.buckets[1].in_count, 12);
        assert_eq!(series.buckets[1].task_time_ns, Some(1_700_000));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_history_process_group_carries_queued() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/process-groups/pg-1/status/history"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "statusHistory": {
                    "aggregateSnapshots": [
                        {"timestamp": "04/27/2026 10:00:00 UTC",
                         "statusMetrics": {"flowFilesIn": 50, "flowFilesOut": 48, "flowFilesQueued": 2}}
                    ]
                }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let guard = client.read().await;
        let series = status_history(&guard, ComponentKind::ProcessGroup, "pg-1")
            .await
            .expect("series");
        assert_eq!(series.buckets.len(), 1);
        assert_eq!(series.buckets[0].in_count, 50);
        assert_eq!(series.buckets[0].queued_count, Some(2));
        assert_eq!(series.buckets[0].task_time_ns, None);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_history_connection_returns_queued_series() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/connections/conn-1/status/history"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "statusHistory": {
                    "aggregateSnapshots": [
                        {"timestamp": "04/27/2026 10:00:00 UTC",
                         "statusMetrics": {"flowFilesIn": 5, "flowFilesOut": 4, "flowFilesQueued": 1}}
                    ]
                }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let guard = client.read().await;
        let series = status_history(&guard, ComponentKind::Connection, "conn-1")
            .await
            .expect("series");
        assert_eq!(series.buckets.len(), 1);
        assert_eq!(series.buckets[0].queued_count, Some(1));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_history_404_surfaces_as_endpoint_missing() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/processors/missing/status/history"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let guard = client.read().await;
        let err = status_history(&guard, ComponentKind::Processor, "missing")
            .await
            .expect_err("404");
        assert!(
            is_status_history_endpoint_missing(&err),
            "404 must classify as endpoint-missing, got {err:?}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn status_history_unsupported_kind_returns_unsupported_error() {
        let server = MockServer::start().await;
        let client = test_client(&server).await;
        let guard = client.read().await;
        for kind in [ComponentKind::ControllerService, ComponentKind::Port] {
            let err = status_history(&guard, kind, "any-id")
                .await
                .expect_err("unsupported");
            assert!(
                matches!(err, NifiLensError::SparklineUnsupportedKind { .. }),
                "expected SparklineUnsupportedKind for {kind:?}, got {err:?}"
            );
        }
    }
}
