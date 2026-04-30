//! Events that flow through the single AppEvent channel.

use crate::error::NifiLensError;

/// Events flowing through the single channel consumed by the UI task.
pub enum AppEvent {
    /// Terminal input forwarded by the crossterm task.
    Input(crossterm::event::Event),
    /// Periodic tick for time-based UI updates (status bar).
    Tick,
    /// Per-view data from a worker task.
    Data(ViewPayload),
    /// Result of an intent dispatch, folded back into AppState by the UI task.
    IntentOutcome(Result<IntentOutcome, NifiLensError>),
    /// Raw fetcher output from `ClusterStore`. Main loop applies it to
    /// `AppState.cluster` and follows up with `ClusterChanged`.
    ClusterUpdate(crate::cluster::ClusterUpdate),
    /// Emitted by the main loop after `ClusterUpdate` is applied, so
    /// per-view reducers can re-derive their projections.
    ClusterChanged(crate::cluster::ClusterEndpoint),
    /// Periodic sparkline series replace from the per-selection
    /// worker. The reducer drops it silently if `(kind, id)` no longer
    /// matches the active selection (defends against the brief window
    /// between worker abort and exit).
    SparklineUpdate {
        kind: crate::client::history::ComponentKind,
        id: String,
        series: crate::client::history::StatusHistorySeries,
    },
    /// 404 from NiFi for the status_history endpoint. Sticky per
    /// `(kind, id)` until selection change — the reducer sets
    /// `SparklineState::endpoint_missing = true`.
    SparklineEndpointMissing {
        kind: crate::client::history::ComponentKind,
        id: String,
    },
    /// Graceful quit request.
    Quit,
}

/// Data delivered from a view's worker task back into the UI loop.
///
/// Overview has no variant here — it is a store-only consumer that
/// reacts to `ClusterChanged` events and reads projections straight
/// from `state.cluster.snapshot`.
#[derive(Debug, Clone)]
pub enum ViewPayload {
    Browser(BrowserPayload),
    Tracer(TracerPayload),
    Events(EventsPayload),
}

/// Payload sent from the Browser detail worker — a single variant
/// carrying one completed per-node detail fetch.
///
/// The central-cluster-store refactor removed the `Tree` variant:
/// the Browser arena is now rebuilt from `AppState.cluster.snapshot`
/// whenever any of `RootPgStatus`, `ControllerServices`, or
/// `ConnectionsByPg` updates arrive.
#[derive(Debug, Clone)]
pub enum BrowserPayload {
    Detail(Box<crate::view::browser::state::NodeDetailSnapshot>),
    /// Result of a successful version-control modal load: identity
    /// re-fetched from `/versions/process-groups/{id}` plus the
    /// flattened diff from `/process-groups/{id}/local-modifications`.
    VersionControlModalLoaded {
        pg_id: String,
        identity: Option<crate::cluster::snapshot::VersionControlSummary>,
        differences: crate::client::FlowComparisonGrouped,
    },
    /// Failure of either fetch in the modal load. The reducer renders
    /// `err` inside the modal and clears the worker handle.
    VersionControlModalFailed {
        pg_id: String,
        err: String,
    },
    /// Successful chain fetch for the parameter-context modal. The
    /// reducer installs the chain and clears the worker handle.
    ParameterContextModalLoaded {
        pg_id: String,
        chain: Vec<crate::client::parameter_context::ParameterContextNode>,
    },
    /// Failed chain fetch for the parameter-context modal. The reducer
    /// renders the error inside the modal and clears the worker handle.
    ParameterContextModalFailed {
        pg_id: String,
        err: String,
    },
    /// One page of action-history results for an open action-history
    /// modal. The reducer appends `actions` to the modal's buffer (de-
    /// duplicating by `ActionEntity::id`), updates `total`, and clears
    /// the loading flag.
    ActionHistoryPage {
        source_id: String,
        offset: u32,
        actions: Vec<nifi_rust_client::dynamic::types::ActionEntity>,
        total: Option<u32>,
    },
    /// Action-history fetch failed. Reducer renders `err` inside the
    /// modal and stops auto-loading.
    ActionHistoryError {
        source_id: String,
        err: String,
    },
    /// Worker has POSTed a listing-request and received a request id from
    /// NiFi. The reducer records the id on `QueueListingState` so a later
    /// refresh chord can DELETE the right id without reaching into worker
    /// state.
    QueueListingRequestIdAssigned {
        queue_id: String,
        request_id: String,
    },
    /// One poll tick of an in-flight listing request — `percent` comes
    /// from NiFi's `percent_completed` field. The reducer updates the
    /// loading chip.
    QueueListingProgress {
        queue_id: String,
        percent: u8,
    },
    /// Terminal success: NiFi returned `state == FINISHED` with summaries.
    /// `total` is NiFi's `queue_size.object_count`; `truncated == true`
    /// when `total > rows.len()` (server caps listing at 100).
    QueueListingComplete {
        queue_id: String,
        rows: Vec<crate::view::browser::state::queue_listing::QueueListingRow>,
        total: u64,
        truncated: bool,
    },
    /// Terminal error: any HTTP failure during POST or polling, or NiFi
    /// reported `state == FAILED`. The reducer renders `err` in the panel
    /// header.
    QueueListingError {
        queue_id: String,
        err: String,
    },
    /// Terminal timeout: 30 s elapsed without `state == FINISHED`. The
    /// reducer renders the timeout chip; user retries via `r`.
    QueueListingTimeout {
        queue_id: String,
    },
    /// Successful per-flowfile peek fetch. Populates the modal's
    /// attribute table and content-claim metadata.
    FlowfilePeek {
        queue_id: String,
        uuid: String,
        attrs: std::collections::BTreeMap<String, String>,
        content_claim: Option<crate::view::browser::state::queue_listing::ContentClaimSummary>,
        mime_type: Option<String>,
    },
    /// Per-flowfile peek fetch failed. Reducer renders `err` inside the
    /// modal.
    FlowfilePeekError {
        queue_id: String,
        uuid: String,
        err: String,
    },
}

/// Result of a successful intent dispatch.
#[derive(Debug)]
pub enum IntentOutcome {
    ContextSwitched {
        new_context_name: String,
        new_version: semver::Version,
        new_base_url: String,
    },
    ViewRefreshed {
        view: crate::app::state::ViewId,
    },
    Quitting,
    /// The intent is valid but its handler is not implemented yet.
    /// The banner shows `"{intent_name}: not yet implemented"`.
    NotImplemented {
        intent_name: &'static str,
    },
    /// The user asked to goto a component in Browser. The reducer
    /// switches tabs, expands ancestors, and sets selection.
    OpenInBrowserTarget {
        component_id: String,
        group_id: String,
    },
    /// Cross-link from Bulletins tab navigates Tracer to latest-events view.
    TracerLandingOn {
        component_id: String,
    },
    /// A lineage query has been submitted; holds the abort handle.
    TracerLineageStarted {
        uuid: String,
        abort: tokio::task::AbortHandle,
    },
    /// User submitted input that is not a valid UUID.
    TracerInputInvalid {
        raw: String,
    },
    /// Cross-link from Bulletins/Browser `t` lands on Events
    /// pre-filled with the component and a 15-minute time window. The
    /// reducer switches tabs, seeds `filters.source`, and kicks off a
    /// query submission.
    EventsLandingOn {
        component_id: String,
    },
    /// Parameter-contexts feature: open the parameter-context modal on
    /// Browser scoped to the given PG, optionally pre-selecting a
    /// parameter name. The reducer calls
    /// `BrowserState::open_parameter_context_modal`.
    OpenParameterContextModalTarget {
        pg_id: String,
        preselect: Option<String>,
    },
}

/// Payload variants pushed from Tracer workers back into the UI loop.
#[derive(Debug, Clone)]
pub enum TracerPayload {
    LatestEvents(crate::client::LatestEventsSnapshot),
    LatestEventsFailed {
        component_id: String,
        error: String,
    },
    LineageSubmitted {
        uuid: String,
        query_id: String,
        cluster_node_id: Option<String>,
    },
    LineagePartial {
        query_id: String,
        percent: u8,
    },
    LineageDone {
        uuid: String,
        query_id: String,
        snapshot: crate::client::LineageSnapshot,
        fetched_at: std::time::SystemTime,
    },
    LineageFailed {
        uuid: String,
        query_id: String,
        error: String,
    },
    EventDetail {
        event_id: i64,
        detail: Box<crate::client::ProvenanceEventDetail>,
    },
    EventDetailFailed {
        event_id: i64,
        error: String,
    },
    Content(crate::client::ContentSnapshot),
    ContentFailed {
        event_id: i64,
        side: crate::client::ContentSide,
        error: String,
    },
    ContentSaved {
        path: std::path::PathBuf,
    },
    ContentSaveFailed {
        path: std::path::PathBuf,
        error: String,
    },
    ModalChunk {
        event_id: i64,
        side: crate::client::ContentSide,
        offset: usize,
        bytes: Vec<u8>,
        eof: bool,
        /// The length the worker asked for; the reducer uses this to
        /// decide whether a short read triggers EOF independently of
        /// client-side classification.
        requested_len: usize,
    },
    ModalChunkFailed {
        event_id: i64,
        side: crate::client::ContentSide,
        offset: usize,
        error: String,
    },
    /// Result of an off-thread tabular decode (Parquet/Avro) spawned
    /// via `tokio::task::spawn_blocking`. The UI-task handler calls
    /// `apply_tabular_decode_result` to install the render.
    ContentDecoded {
        event_id: i64,
        side: crate::client::ContentSide,
        render: crate::client::tracer::ContentRender,
    },
    /// Result of an off-thread JSON pretty-print (`serde_transcode` +
    /// `Serializer::pretty`) spawned via `tokio::task::spawn_blocking`.
    /// `pretty` is `None` when the bytes did not parse as JSON. The
    /// UI-task handler calls `apply_json_pretty_result`.
    JsonPrettyPrinted {
        event_id: i64,
        side: crate::client::ContentSide,
        pretty: Option<String>,
    },
}

/// Payload variants pushed from the Events tab worker back into the UI loop.
///
/// The full lifecycle:
/// 1. User submits a query → worker emits `QueryStarted { query_id }`.
/// 2. Worker polls and emits `QueryProgress { percent }` until the server
///    reports `finished = true`.
/// 3. Worker emits `QueryDone { events, fetched_at, truncated }` on success.
/// 4. Worker emits `QueryFailed { error }` on any error during the above.
#[derive(Debug, Clone)]
pub enum EventsPayload {
    QueryStarted {
        query_id: String,
    },
    QueryProgress {
        query_id: String,
        percent: u8,
    },
    QueryDone {
        query_id: String,
        events: Vec<crate::client::ProvenanceEventSummary>,
        fetched_at: std::time::SystemTime,
        truncated: bool,
    },
    QueryFailed {
        query_id: Option<String>,
        error: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_history_page_payload_destructures() {
        let ev = BrowserPayload::ActionHistoryPage {
            source_id: "proc-1".into(),
            offset: 0,
            actions: vec![],
            total: Some(3),
        };
        match ev {
            BrowserPayload::ActionHistoryPage {
                source_id,
                offset,
                actions,
                total,
            } => {
                assert_eq!(source_id, "proc-1");
                assert_eq!(offset, 0);
                assert!(actions.is_empty());
                assert_eq!(total, Some(3));
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn action_history_error_payload_destructures() {
        let ev = BrowserPayload::ActionHistoryError {
            source_id: "proc-1".into(),
            err: "boom".into(),
        };
        match ev {
            BrowserPayload::ActionHistoryError { source_id, err } => {
                assert_eq!(source_id, "proc-1");
                assert_eq!(err, "boom");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sparkline_update_event_destructures() {
        use crate::client::history::{Bucket, ComponentKind, StatusHistorySeries};
        let series = StatusHistorySeries {
            buckets: vec![Bucket {
                timestamp: std::time::SystemTime::now(),
                in_count: 10,
                out_count: 8,
                queued_count: None,
                task_time_ns: Some(1000),
                bytes_per_sec: None,
            }],
            generated_at: std::time::SystemTime::now(),
        };
        let ev = AppEvent::SparklineUpdate {
            kind: ComponentKind::Processor,
            id: "proc-1".into(),
            series,
        };
        match ev {
            AppEvent::SparklineUpdate { kind, id, series } => {
                assert!(matches!(kind, ComponentKind::Processor));
                assert_eq!(id, "proc-1");
                assert_eq!(series.buckets.len(), 1);
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn sparkline_endpoint_missing_event_destructures() {
        use crate::client::history::ComponentKind;
        let ev = AppEvent::SparklineEndpointMissing {
            kind: ComponentKind::Connection,
            id: "conn-1".into(),
        };
        match ev {
            AppEvent::SparklineEndpointMissing { kind, id } => {
                assert!(matches!(kind, ComponentKind::Connection));
                assert_eq!(id, "conn-1");
            }
            _ => panic!("wrong variant"),
        }
    }

    #[test]
    fn queue_listing_payloads_destructure() {
        let progress = BrowserPayload::QueueListingProgress {
            queue_id: "q1".into(),
            percent: 42,
        };
        let complete = BrowserPayload::QueueListingComplete {
            queue_id: "q1".into(),
            rows: vec![],
            total: 0,
            truncated: false,
        };
        let error = BrowserPayload::QueueListingError {
            queue_id: "q1".into(),
            err: "boom".into(),
        };
        let timeout = BrowserPayload::QueueListingTimeout {
            queue_id: "q1".into(),
        };
        let request_id = BrowserPayload::QueueListingRequestIdAssigned {
            queue_id: "q1".into(),
            request_id: "req-1".into(),
        };
        let _ = (progress, complete, error, timeout, request_id);
    }

    #[test]
    fn flowfile_peek_payloads_destructure() {
        let peek = BrowserPayload::FlowfilePeek {
            queue_id: "q1".into(),
            uuid: "ff-1".into(),
            attrs: std::collections::BTreeMap::new(),
            content_claim: None,
            mime_type: None,
        };
        let err = BrowserPayload::FlowfilePeekError {
            queue_id: "q1".into(),
            uuid: "ff-1".into(),
            err: "404".into(),
        };
        let _ = (peek, err);
    }
}
