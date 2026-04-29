//! Browser tab detail worker.
//!
//! Task 6 of the central-cluster-store refactor retired the periodic
//! tree-poll branch: the Browser arena is now rebuilt from
//! `AppState.cluster.snapshot` whenever `RootPgStatus`,
//! `ControllerServices`, or `ConnectionsByPg` updates arrive. This
//! worker's only remaining job is servicing on-demand per-node detail
//! fetches the reducer emits via the `detail_tx` side channel.
//!
//! Runs under the main-thread `LocalSet` (see `lib::run_inner`) because
//! `nifi-rust-client`'s dynamic dispatch traits return `!Send` futures.

use std::sync::{Arc, Mutex};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::app::worker::send_poll_result;
use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::event::{AppEvent, BrowserPayload, ViewPayload};
use crate::view::browser::state::{DetailRequest, NodeDetail, NodeDetailSnapshot};

pub fn spawn(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    mut detail_rx: mpsc::UnboundedReceiver<DetailRequest>,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        loop {
            let Some(req) = detail_rx.recv().await else {
                tracing::debug!("browser worker: detail channel closed, exiting");
                return;
            };
            fetch_detail_once(&client, &tx, req).await;
        }
    })
}

async fn fetch_detail_once(
    client: &Arc<RwLock<NifiClient>>,
    tx: &mpsc::Sender<AppEvent>,
    req: DetailRequest,
) {
    use crate::client::NodeKind;
    let guard = client.read().await;
    let detail: Result<NodeDetail, NifiLensError> = match req.kind {
        NodeKind::ProcessGroup => guard
            .browser_pg_detail(&req.id)
            .await
            .map(NodeDetail::ProcessGroup),
        NodeKind::Processor => guard
            .browser_processor_detail(&req.id)
            .await
            .map(NodeDetail::Processor),
        NodeKind::Connection => guard
            .browser_connection_detail(&req.id)
            .await
            .map(NodeDetail::Connection),
        NodeKind::ControllerService => guard
            .browser_cs_detail(&req.id)
            .await
            .map(NodeDetail::ControllerService),
        NodeKind::InputPort => {
            let kind = crate::client::PortKind::Input;
            guard
                .browser_port_detail(&req.id, kind)
                .await
                .map(NodeDetail::Port)
        }
        NodeKind::OutputPort => {
            let kind = crate::client::PortKind::Output;
            guard
                .browser_port_detail(&req.id, kind)
                .await
                .map(NodeDetail::Port)
        }
        NodeKind::RemoteProcessGroup => guard
            .browser_remote_process_group_detail(&req.id)
            .await
            .map(NodeDetail::RemoteProcessGroup),
        NodeKind::Folder(_) => return,
    };
    let result = detail.map(|detail| {
        ViewPayload::Browser(BrowserPayload::Detail(Box::new(NodeDetailSnapshot {
            arena_idx: req.arena_idx,
            kind: req.kind,
            id: req.id,
            detail,
        })))
    });
    send_poll_result(tx, "browser detail", result).await;
}

/// Fetch the full parameter-context inheritance chain for a single bound
/// context and emit `BrowserPayload::ParameterContextModalLoaded` (or
/// `…Failed`). One-shot; no polling.
pub fn spawn_parameter_context_modal_fetch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    pg_id: String,
    bound_context_id: String,
) -> tokio::task::JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let result = crate::client::parameter_context::fetch_chain(client, &bound_context_id).await;
        let payload = match result {
            crate::client::parameter_context::ChainFetchResult::Loaded(chain) => {
                BrowserPayload::ParameterContextModalLoaded { pg_id, chain }
            }
            crate::client::parameter_context::ChainFetchResult::BoundFailed(message) => {
                BrowserPayload::ParameterContextModalFailed {
                    pg_id,
                    err: message,
                }
            }
        };
        let _ = tx.send(AppEvent::Data(ViewPayload::Browser(payload))).await;
    })
}

/// Action-history modal worker. Eagerly fetches the first page (100
/// actions), emits `ActionHistoryPage`, then sleeps on `signal` until
/// the reducer wakes it for the next page. Exits when the paginator
/// is exhausted, on first error (after emitting `ActionHistoryError`),
/// or when aborted by the modal-close path.
pub fn spawn_action_history_modal_fetch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    source_id: String,
    signal: std::sync::Arc<tokio::sync::Notify>,
) -> JoinHandle<()> {
    const PAGE_SIZE: u32 = 100;
    tokio::task::spawn_local(async move {
        let mut offset: u32 = 0;
        loop {
            let res = {
                let guard = client.read().await;
                let offset_s = offset.to_string();
                let count_s = PAGE_SIZE.to_string();
                guard
                    .flow()
                    .query_history(
                        &offset_s,
                        &count_s,
                        None,
                        None,
                        None,
                        None,
                        None,
                        Some(&source_id),
                    )
                    .await
            };
            match res {
                Ok(dto) => {
                    let total = dto.total.map(|t| t.max(0) as u32);
                    let actions = dto.actions.unwrap_or_default();
                    let returned = actions.len() as u32;
                    let payload = BrowserPayload::ActionHistoryPage {
                        source_id: source_id.clone(),
                        offset,
                        actions,
                        total,
                    };
                    if tx
                        .send(AppEvent::Data(ViewPayload::Browser(payload)))
                        .await
                        .is_err()
                    {
                        return; // channel closed
                    }
                    // Exhaustion: short page or offset reaches total.
                    let exhausted = match total {
                        Some(t) => {
                            returned == 0
                                || returned < PAGE_SIZE
                                || offset.saturating_add(returned) >= t
                        }
                        None => returned == 0 || returned < PAGE_SIZE,
                    };
                    if exhausted {
                        return;
                    }
                    offset = offset.saturating_add(returned);
                }
                Err(err) => {
                    let payload = BrowserPayload::ActionHistoryError {
                        source_id: source_id.clone(),
                        err: err.to_string(),
                    };
                    let _ = tx.send(AppEvent::Data(ViewPayload::Browser(payload))).await;
                    return;
                }
            }
            // Wait for the reducer to signal next-page.
            signal.notified().await;
        }
    })
}

/// Per-selection sparkline fetch loop. Eagerly fetches the first
/// `status_history` series, emits `AppEvent::SparklineUpdate`, then
/// sleeps `cadence` and repeats. On 404 emits
/// `SparklineEndpointMissing` once and continues looping (the next
/// tick may still 404 — emits are idempotent at the reducer level).
/// Other errors log at `warn!` and continue. Exits when aborted by
/// the selection-change path or when the channel closes.
pub fn spawn_sparkline_fetch_loop(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    kind: crate::client::history::ComponentKind,
    id: String,
    cadence: std::time::Duration,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        tracing::debug!(?kind, %id, ?cadence, "sparkline worker started");
        loop {
            let res = {
                let guard = client.read().await;
                crate::client::history::status_history(&guard, kind, &id).await
            };
            let event = match res {
                Ok(series) => {
                    tracing::debug!(
                        ?kind,
                        %id,
                        bucket_count = series.buckets.len(),
                        "status_history fetch ok"
                    );
                    AppEvent::SparklineUpdate {
                        kind,
                        id: id.clone(),
                        series,
                    }
                }
                Err(err) if crate::client::history::is_status_history_endpoint_missing(&err) => {
                    AppEvent::SparklineEndpointMissing {
                        kind,
                        id: id.clone(),
                    }
                }
                Err(err) => {
                    tracing::warn!(?err, %id, "status_history fetch failed; will retry");
                    tokio::time::sleep(cadence).await;
                    continue;
                }
            };
            if tx.send(event).await.is_err() {
                return;
            }
            tokio::time::sleep(cadence).await;
        }
    })
}

/// Fetch identity + diff for a single PG and emit
/// `BrowserPayload::VersionControlModalLoaded` (or `…Failed`). One-shot;
/// no polling. Uses `futures::future::try_join` to issue both calls in
/// parallel — both share the same `NifiLensError` so `try_join` types
/// align without an explicit error map.
pub fn spawn_version_control_modal_fetch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    pg_id: String,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let res = {
            let guard = client.read().await;
            futures::future::try_join(
                guard.version_information_optional(&pg_id),
                guard.local_modifications(&pg_id),
            )
            .await
        };
        let payload = match res {
            Ok((identity, differences)) => BrowserPayload::VersionControlModalLoaded {
                pg_id,
                identity,
                differences,
            },
            Err(err) => BrowserPayload::VersionControlModalFailed {
                pg_id,
                err: err.to_string(),
            },
        };
        let _ = tx.send(AppEvent::Data(ViewPayload::Browser(payload))).await;
    })
}

/// Two-phase queue-listing worker.
///
/// **Phase 1 — POST:** calls [`crate::client::queues::submit_listing_request`]
/// to create a NiFi listing-request. On success, writes the returned
/// `request_id` into the shared `Arc<Mutex<Option<String>>>` slot (so the
/// returned [`QueueListingHandle`]'s `Drop` impl can DELETE it on cleanup),
/// then emits [`BrowserPayload::QueueListingRequestIdAssigned`].
///
/// **Phase 2 — poll:** loops on 500 ms ticks calling
/// [`crate::client::queues::poll_listing_request`]. Each tick emits
/// [`BrowserPayload::QueueListingProgress`]. On `finished == true` emits
/// [`BrowserPayload::QueueListingComplete`] and returns. On
/// `state == "FAILED"` emits [`BrowserPayload::QueueListingError`] and
/// returns. On HTTP error emits [`BrowserPayload::QueueListingError`] and
/// returns. If `timeout` elapses before `finished`, emits
/// [`BrowserPayload::QueueListingTimeout`] and returns.
///
/// Emit ordering guarantee: `RequestIdAssigned` → one or more `Progress` →
/// exactly one terminal (`Complete` | `Error` | `Timeout`).
pub fn spawn_queue_listing_fetch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    queue_id: String,
    timeout: std::time::Duration,
) -> QueueListingHandle {
    let request_id_slot: Arc<Mutex<Option<String>>> = Arc::new(Mutex::new(None));
    let request_id_writer = request_id_slot.clone();
    let q_id = queue_id.clone();
    let client_for_worker = client.clone();

    let join = tokio::task::spawn_local(async move {
        let started = std::time::Instant::now();

        // Phase 1: POST listing request.
        let dto = {
            let guard = client_for_worker.read().await;
            crate::client::queues::submit_listing_request(&guard, &q_id).await
        };
        let dto = match dto {
            Ok(dto) => dto,
            Err(e) => {
                tracing::warn!(error = ?e, queue_id = %q_id, "queue listing POST failed");
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingError {
                            queue_id: q_id,
                            err: e.to_string(),
                        },
                    )))
                    .await;
                return;
            }
        };
        let request_id = match dto.id.clone() {
            Some(id) => id,
            None => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingError {
                            queue_id: q_id,
                            err: "NiFi response missing listing request id".to_string(),
                        },
                    )))
                    .await;
                return;
            }
        };
        // Write id so Drop can DELETE it even if we exit before completing.
        if let Ok(mut g) = request_id_writer.lock() {
            *g = Some(request_id.clone());
        }
        let _ = tx
            .send(AppEvent::Data(ViewPayload::Browser(
                BrowserPayload::QueueListingRequestIdAssigned {
                    queue_id: q_id.clone(),
                    request_id: request_id.clone(),
                },
            )))
            .await;

        // Phase 2: poll until finished, failed, error, or timeout.
        loop {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            if started.elapsed() > timeout {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingTimeout { queue_id: q_id },
                    )))
                    .await;
                return;
            }

            let dto = {
                let guard = client_for_worker.read().await;
                crate::client::queues::poll_listing_request(&guard, &q_id, &request_id).await
            };
            let dto = match dto {
                Ok(dto) => dto,
                Err(e) => {
                    tracing::warn!(error = ?e, queue_id = %q_id, "queue listing poll failed");
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Browser(
                            BrowserPayload::QueueListingError {
                                queue_id: q_id,
                                err: e.to_string(),
                            },
                        )))
                        .await;
                    return;
                }
            };

            let percent = dto.percent_completed.unwrap_or(0).clamp(0, 100) as u8;
            let _ = tx
                .send(AppEvent::Data(ViewPayload::Browser(
                    BrowserPayload::QueueListingProgress {
                        queue_id: q_id.clone(),
                        percent,
                    },
                )))
                .await;

            if dto.finished.unwrap_or(false) {
                let total = dto
                    .queue_size
                    .as_ref()
                    .and_then(|q| q.object_count)
                    .unwrap_or(0)
                    .max(0) as u64;
                let summaries = dto.flow_file_summaries.unwrap_or_default();
                let rows: Vec<crate::view::browser::state::queue_listing::QueueListingRow> =
                    summaries
                        .into_iter()
                        .map(
                            |s| crate::view::browser::state::queue_listing::QueueListingRow {
                                uuid: s.uuid.unwrap_or_default(),
                                filename: s.filename,
                                size: s.size.unwrap_or(0).max(0) as u64,
                                queued_duration: crate::client::queues::ms_to_duration(
                                    s.queued_duration,
                                ),
                                position: s.position.unwrap_or(0).max(0) as u64,
                                penalized: s.penalized.unwrap_or(false),
                                cluster_node_id: s.cluster_node_id,
                                lineage_duration: crate::client::queues::ms_to_duration(
                                    s.lineage_duration,
                                ),
                            },
                        )
                        .collect();
                let truncated = total > rows.len() as u64;
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingComplete {
                            queue_id: q_id,
                            rows,
                            total,
                            truncated,
                        },
                    )))
                    .await;
                return;
            }

            if dto.state.as_deref() == Some("FAILED") {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingError {
                            queue_id: q_id,
                            err: dto
                                .failure_reason
                                .unwrap_or_else(|| "listing failed".to_string()),
                        },
                    )))
                    .await;
                return;
            }
        }
    });

    QueueListingHandle::new(join, queue_id, request_id_slot, client)
}

/// One-shot fetch for the per-flowfile peek modal. Emits exactly one
/// of `BrowserPayload::FlowfilePeek` (success) or
/// `BrowserPayload::FlowfilePeekError` (HTTP failure). Returns the
/// `JoinHandle<()>` so the modal-close path can `.abort()` if the
/// modal closes before the fetch completes.
///
/// `cluster_node_id` threads through to NiFi's `?clusterNodeId=...`
/// query param. `None` for standalone clusters; `Some(node_id)` for
/// clustered NiFi where the flowfile lives on a specific node (each
/// `FlowFileSummaryDto` carries the id we should pass).
pub fn spawn_flowfile_peek_fetch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    queue_id: String,
    uuid: String,
    cluster_node_id: Option<String>,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let result = {
            let guard = client.read().await;
            crate::client::queues::get_flowfile(
                &guard,
                &queue_id,
                &uuid,
                cluster_node_id.as_deref(),
            )
            .await
        };

        match result {
            Err(e) => {
                tracing::warn!(
                    error = ?e,
                    queue_id = %queue_id,
                    uuid = %uuid,
                    "flowfile peek failed"
                );
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::FlowfilePeekError {
                            queue_id,
                            uuid,
                            err: e.to_string(),
                        },
                    )))
                    .await;
            }
            Ok(dto) => {
                let attrs: std::collections::BTreeMap<String, String> = dto
                    .attributes
                    .unwrap_or_default()
                    .into_iter()
                    .filter_map(|(k, v)| v.map(|val| (k, val)))
                    .collect();

                let content_claim = match (
                    dto.content_claim_container,
                    dto.content_claim_section,
                    dto.content_claim_identifier,
                    dto.content_claim_offset,
                    dto.content_claim_file_size_bytes,
                ) {
                    (
                        Some(container),
                        Some(section),
                        Some(identifier),
                        Some(offset),
                        Some(file_size),
                    ) if file_size > 0 => Some(
                        crate::view::browser::state::queue_listing::ContentClaimSummary {
                            container,
                            section,
                            identifier,
                            offset: offset.max(0) as u64,
                            file_size: file_size.max(0) as u64,
                        },
                    ),
                    _ => None,
                };

                let mime_type = dto.mime_type;

                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::FlowfilePeek {
                            queue_id,
                            uuid,
                            attrs,
                            content_claim,
                            mime_type,
                        },
                    )))
                    .await;
            }
        }
    })
}

/// Owns the polling worker's `JoinHandle` plus the listing-request id
/// once the worker has POSTed it. Drop aborts the worker and (if a
/// request id is known) fires `DELETE /flowfile-queues/{q}/listing-requests/{r}`
/// best-effort on the same `LocalSet` we're already on.
///
/// Constructed by `spawn_queue_listing_fetch` (Task 6); the
/// `new_for_test` constructor exists to exercise Drop semantics
/// without spinning up the full polling worker.
pub struct QueueListingHandle {
    join: Option<JoinHandle<()>>,
    queue_id: String,
    request_id: Arc<Mutex<Option<String>>>,
    client: Arc<RwLock<NifiClient>>,
}

impl std::fmt::Debug for QueueListingHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let request_id = self.request_id.lock().ok().and_then(|g| g.clone());
        f.debug_struct("QueueListingHandle")
            .field("queue_id", &self.queue_id)
            .field("request_id", &request_id)
            .field("join_active", &self.join.is_some())
            .finish()
    }
}

impl QueueListingHandle {
    /// Constructor for the polling worker. Called by
    /// `spawn_queue_listing_fetch` to bind the worker handle to its
    /// cleanup state.
    pub(crate) fn new(
        join: JoinHandle<()>,
        queue_id: String,
        request_id: Arc<Mutex<Option<String>>>,
        client: Arc<RwLock<NifiClient>>,
    ) -> Self {
        Self {
            join: Some(join),
            queue_id,
            request_id,
            client,
        }
    }

    /// Test-only constructor that skips the worker spawn. Drop behavior
    /// is identical so the inline tests can verify DELETE semantics in
    /// isolation from the polling logic.
    #[doc(hidden)]
    pub fn new_for_test(
        queue_id: String,
        request_id: Arc<Mutex<Option<String>>>,
        client: Arc<RwLock<NifiClient>>,
    ) -> Self {
        Self {
            join: None,
            queue_id,
            request_id,
            client,
        }
    }
}

impl Drop for QueueListingHandle {
    fn drop(&mut self) {
        if let Some(join) = self.join.take() {
            join.abort();
        }
        let request_id = match self.request_id.lock() {
            Ok(g) => g.clone(),
            Err(p) => p.into_inner().clone(),
        };
        let Some(request_id) = request_id else { return };
        let client = self.client.clone();
        let queue_id = self.queue_id.clone();
        // Drop runs on the main UI task, which lives on the LocalSet —
        // spawn_local is the right primitive here.
        tokio::task::spawn_local(async move {
            let guard = client.read().await;
            if let Err(e) =
                crate::client::queues::cancel_listing_request(&guard, &queue_id, &request_id).await
            {
                tracing::warn!(
                    error = ?e,
                    queue_id,
                    request_id,
                    "failed to delete listing request",
                );
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use tokio::sync::{RwLock, mpsc};
    use tokio::task::LocalSet;

    use crate::client::NifiClient;
    use crate::event::{AppEvent, BrowserPayload, ViewPayload};

    /// Build a `NifiClient` against the wiremock server with a stubbed
    /// `/nifi-api/flow/about` so `detect_version` succeeds.
    async fn test_client(server: &wiremock::MockServer) -> Arc<RwLock<NifiClient>> {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, ResponseTemplate};
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

    /// Verify the worker emits `ParameterContextModalLoaded` with the
    /// fetched chain when the bound context is reachable.
    #[tokio::test(flavor = "current_thread")]
    async fn chain_fetch_worker_emits_loaded_on_success() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Stub: bound context with no inherited contexts.
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-a"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "id": "ctx-a",
                "component": {
                    "id": "ctx-a",
                    "name": "Context A",
                    "parameters": [
                        {"parameter": {"name": "host", "value": "localhost", "sensitive": false, "provided": false}}
                    ],
                    "inheritedParameterContexts": []
                }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(8);

        let local = LocalSet::new();
        local
            .run_until(async {
                let h = super::spawn_parameter_context_modal_fetch(
                    client,
                    tx,
                    "pg-1".into(),
                    "ctx-a".into(),
                );
                h.await.expect("worker completed");
            })
            .await;

        let ev = rx.recv().await.expect("event emitted");
        match ev {
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::ParameterContextModalLoaded {
                pg_id,
                chain,
            })) => {
                assert_eq!(pg_id, "pg-1");
                assert_eq!(chain.len(), 1);
                assert_eq!(chain[0].name, "Context A");
                assert_eq!(chain[0].parameters.len(), 1);
                assert_eq!(chain[0].parameters[0].name, "host");
            }
            AppEvent::Data(ViewPayload::Browser(_)) => {
                panic!("received a Browser payload but not ParameterContextModalLoaded")
            }
            _ => panic!("expected ParameterContextModalLoaded"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn action_history_worker_emits_first_page_eagerly() {
        use std::sync::Arc;
        use tokio::sync::Notify;
        use wiremock::matchers::{method, path, query_param};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .and(query_param("offset", "0"))
            .and(query_param("count", "100"))
            .and(query_param("sourceId", "proc-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "history": {
                    "total": 1,
                    "actions": [{
                        "id": 42, "sourceId": "proc-1", "timestamp": "2026-04-27T10:00:00Z",
                        "action": {"id": 42, "operation": "Configure", "sourceId": "proc-1",
                                   "sourceName": "p", "sourceType": "Processor",
                                   "userIdentity": "alice"}
                    }]
                }
            })))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(8);
        let signal = Arc::new(Notify::new());
        let local = tokio::task::LocalSet::new();

        local
            .run_until(async {
                let h = super::spawn_action_history_modal_fetch(
                    client,
                    tx,
                    "proc-1".into(),
                    signal.clone(),
                );
                let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .expect("event received in time")
                    .expect("event present");
                match ev {
                    AppEvent::Data(ViewPayload::Browser(BrowserPayload::ActionHistoryPage {
                        source_id,
                        offset,
                        actions,
                        total,
                    })) => {
                        assert_eq!(source_id, "proc-1");
                        assert_eq!(offset, 0);
                        assert_eq!(actions.len(), 1);
                        assert_eq!(actions[0].id, Some(42));
                        assert_eq!(total, Some(1));
                    }
                    _ => panic!("expected ActionHistoryPage"),
                }
                // Worker exits because the paginator is exhausted; aborting is harmless.
                h.abort();
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn action_history_worker_emits_error_on_500() {
        use std::sync::Arc;
        use tokio::sync::Notify;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/history"))
            .respond_with(ResponseTemplate::new(500))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(8);
        let signal = Arc::new(Notify::new());
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let _h = super::spawn_action_history_modal_fetch(
                    client,
                    tx,
                    "proc-1".into(),
                    signal.clone(),
                );
                let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .expect("event received")
                    .expect("event present");
                match ev {
                    AppEvent::Data(ViewPayload::Browser(BrowserPayload::ActionHistoryError {
                        source_id,
                        err,
                    })) => {
                        assert_eq!(source_id, "proc-1");
                        assert!(!err.is_empty());
                    }
                    _ => panic!("expected ActionHistoryError"),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sparkline_worker_emits_first_series_then_loops() {
        use crate::client::history::ComponentKind;
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        let counter = Arc::new(AtomicUsize::new(0));
        let counter_for_resp = counter.clone();
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/processors/p-1/status/history"))
            .respond_with(move |_: &wiremock::Request| {
                let n = counter_for_resp.fetch_add(1, Ordering::SeqCst);
                ResponseTemplate::new(200).set_body_json(serde_json::json!({
                    "statusHistory": {
                        "aggregateSnapshots": [{
                            "timestamp": "04/27/2026 10:00:00 UTC",
                            "statusMetrics": {"flowFilesIn": (n as i64) * 10}
                        }]
                    }
                }))
            })
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(16);
        let local = LocalSet::new();
        local
            .run_until(async {
                let _h = super::spawn_sparkline_fetch_loop(
                    client,
                    tx,
                    ComponentKind::Processor,
                    "p-1".into(),
                    std::time::Duration::from_millis(50),
                );
                let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .expect("first event")
                    .expect("event present");
                match ev {
                    AppEvent::SparklineUpdate { kind, id, series } => {
                        assert!(matches!(kind, ComponentKind::Processor));
                        assert_eq!(id, "p-1");
                        assert_eq!(series.buckets[0].in_count, 0);
                    }
                    _ => panic!("expected SparklineUpdate"),
                }
                let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .expect("second event")
                    .expect("event present");
                match ev {
                    AppEvent::SparklineUpdate { series, .. } => {
                        assert_eq!(
                            series.buckets[0].in_count, 10,
                            "second tick must reflect updated mock counter"
                        );
                    }
                    _ => panic!("expected SparklineUpdate"),
                }
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn sparkline_worker_emits_endpoint_missing_on_404() {
        use crate::client::history::ComponentKind;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/processors/missing/status/history"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(8);
        let local = LocalSet::new();
        local
            .run_until(async {
                let _h = super::spawn_sparkline_fetch_loop(
                    client,
                    tx,
                    ComponentKind::Processor,
                    "missing".into(),
                    std::time::Duration::from_millis(50),
                );
                let ev = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv())
                    .await
                    .expect("event")
                    .expect("event present");
                match ev {
                    AppEvent::SparklineEndpointMissing { kind, id } => {
                        assert!(matches!(kind, ComponentKind::Processor));
                        assert_eq!(id, "missing");
                    }
                    _ => panic!("expected SparklineEndpointMissing"),
                }
            })
            .await;
    }

    /// Verify the worker emits `ParameterContextModalFailed` when the
    /// bound context returns a 404.
    #[tokio::test(flavor = "current_thread")]
    async fn chain_fetch_worker_emits_failed_on_404() {
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        // Bound context returns 404 → BoundFailed.
        Mock::given(method("GET"))
            .and(path("/nifi-api/parameter-contexts/ctx-missing"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(8);

        let local = LocalSet::new();
        local
            .run_until(async {
                let h = super::spawn_parameter_context_modal_fetch(
                    client,
                    tx,
                    "pg-2".into(),
                    "ctx-missing".into(),
                );
                h.await.expect("worker completed");
            })
            .await;

        let ev = rx.recv().await.expect("event emitted");
        match ev {
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::ParameterContextModalFailed {
                pg_id,
                err,
            })) => {
                assert_eq!(pg_id, "pg-2");
                assert!(!err.is_empty(), "error message must be non-empty");
            }
            _ => panic!("expected ParameterContextModalFailed"),
        }
    }

    /// `QueueListingHandle::drop` aborts the worker and fires
    /// `DELETE /flowfile-queues/{id}/listing-requests/{request_id}` on
    /// the same `LocalSet`, so cleanup happens uniformly across
    /// selection-move, tab-switch, and app-shutdown.
    #[tokio::test(flavor = "current_thread")]
    async fn drop_fires_delete_when_request_id_known() {
        use std::sync::Mutex as StdMutex;
        use std::time::Duration as StdDuration;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-42"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!(
                {"listingRequest": {"id": "req-42", "state": "CANCELLED", "finished": true}}
            )))
            .expect(1)
            .mount(&server)
            .await;

        let local = LocalSet::new();
        local
            .run_until(async {
                let client = test_client(&server).await;
                let request_id = Arc::new(StdMutex::new(Some("req-42".to_string())));

                let handle =
                    super::QueueListingHandle::new_for_test("q1".to_string(), request_id, client);
                drop(handle);

                // Give the spawn_local cleanup task time to run + complete
                // the HTTP request. wiremock's `expect(1)` checked at
                // server.verify() catches under-fire, but we want to give
                // the spawned future a fair chance first.
                for _ in 0..30 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(StdDuration::from_millis(20)).await;
                }
            })
            .await;

        server.verify().await;
    }

    /// `spawn_queue_listing_fetch` progresses through WAITING → GENERATING
    /// → GENERATING → COMPLETED and emits the expected sequence of payloads.
    /// Drop fires DELETE once exactly.
    #[tokio::test(flavor = "current_thread")]
    async fn polling_progresses_then_completes() {
        use std::time::Duration as StdDuration;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // POST → WAITING
        Mock::given(method("POST"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "WAITING",
                    "percentCompleted": 0,
                    "finished": false,
                    "flowFileSummaries": [],
                    "queueSize": {"objectCount": 0, "byteCount": 0}
                }
            })))
            .mount(&server)
            .await;

        // First GET → 30% (mounted first, served first in FIFO order; exhausted after 1 use)
        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "GENERATING",
                    "percentCompleted": 30,
                    "finished": false,
                    "flowFileSummaries": [],
                    "queueSize": {"objectCount": 0, "byteCount": 0}
                }
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Second GET → 80% (mounted second, served second; exhausted after 1 use)
        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "GENERATING",
                    "percentCompleted": 80,
                    "finished": false,
                    "flowFileSummaries": [],
                    "queueSize": {"objectCount": 0, "byteCount": 0}
                }
            })))
            .up_to_n_times(1)
            .mount(&server)
            .await;

        // Third GET → COMPLETED with one flowfile (mounted last, served after both progress stubs exhaust)
        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "COMPLETED",
                    "percentCompleted": 100,
                    "finished": true,
                    "flowFileSummaries": [{
                        "uuid": "ff-aaaa",
                        "filename": "a.parquet",
                        "size": 2048,
                        "position": 1,
                        "penalized": false,
                        "queuedDuration": 5000,
                        "lineageDuration": 60000,
                        "clusterNodeId": null
                    }],
                    "queueSize": {"objectCount": 1, "byteCount": 2048}
                }
            })))
            .mount(&server)
            .await;

        // DELETE → 200 (Drop will fire this exactly once)
        Mock::given(method("DELETE"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests/req-1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-1",
                    "state": "CANCELLED",
                    "finished": true
                }
            })))
            .expect(1)
            .mount(&server)
            .await;

        let local = LocalSet::new();
        local
            .run_until(async {
                let client = test_client(&server).await;
                let (tx, mut rx) = mpsc::channel::<AppEvent>(64);

                let handle = super::spawn_queue_listing_fetch(
                    client,
                    tx,
                    "q1".to_string(),
                    StdDuration::from_secs(10),
                );

                // 1. RequestIdAssigned must arrive first.
                let ev = tokio::time::timeout(StdDuration::from_secs(5), rx.recv())
                    .await
                    .expect("timeout waiting for RequestIdAssigned")
                    .expect("channel closed");
                match ev {
                    AppEvent::Data(ViewPayload::Browser(
                        BrowserPayload::QueueListingRequestIdAssigned {
                            queue_id,
                            request_id,
                        },
                    )) => {
                        assert_eq!(queue_id, "q1");
                        assert_eq!(request_id, "req-1");
                    }
                    _ => panic!("expected QueueListingRequestIdAssigned"),
                }

                // 2. Collect payloads until QueueListingComplete.
                let mut progress_count = 0u32;
                let mut complete_payload = None;
                for _ in 0..10 {
                    let ev = tokio::time::timeout(StdDuration::from_secs(5), rx.recv())
                        .await
                        .expect("timeout waiting for progress/complete")
                        .expect("channel closed");
                    match ev {
                        AppEvent::Data(ViewPayload::Browser(
                            BrowserPayload::QueueListingProgress { .. },
                        )) => {
                            progress_count += 1;
                        }
                        AppEvent::Data(ViewPayload::Browser(
                            BrowserPayload::QueueListingComplete {
                                queue_id,
                                rows,
                                total,
                                truncated,
                            },
                        )) => {
                            assert_eq!(queue_id, "q1");
                            assert_eq!(rows.len(), 1, "expected 1 row");
                            assert_eq!(rows[0].uuid, "ff-aaaa");
                            assert_eq!(total, 1);
                            assert!(!truncated);
                            complete_payload = Some(());
                            break;
                        }
                        _ => panic!("unexpected payload"),
                    }
                }
                assert!(
                    progress_count >= 2,
                    "expected ≥2 Progress emits (30%, 80%); got {progress_count}"
                );
                assert!(complete_payload.is_some(), "expected QueueListingComplete");

                // 3. Drop the handle → spawns DELETE on the LocalSet.
                drop(handle);
                for _ in 0..30 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(StdDuration::from_millis(20)).await;
                }
            })
            .await;

        server.verify().await;
    }

    /// `spawn_queue_listing_fetch` emits `QueueListingTimeout` when NiFi
    /// never returns `finished == true` within the configured timeout.
    #[tokio::test(flavor = "current_thread")]
    async fn timeout_emits_when_finished_never_arrives() {
        use std::time::Duration as StdDuration;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;

        // POST → WAITING
        Mock::given(method("POST"))
            .and(path("/nifi-api/flowfile-queues/q1/listing-requests"))
            .respond_with(ResponseTemplate::new(202).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-timeout",
                    "state": "WAITING",
                    "percentCompleted": 0,
                    "finished": false,
                    "flowFileSummaries": [],
                    "queueSize": {"objectCount": 0, "byteCount": 0}
                }
            })))
            .mount(&server)
            .await;

        // GET always returns WAITING — never finishes.
        Mock::given(method("GET"))
            .and(path(
                "/nifi-api/flowfile-queues/q1/listing-requests/req-timeout",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-timeout",
                    "state": "WAITING",
                    "percentCompleted": 0,
                    "finished": false,
                    "flowFileSummaries": [],
                    "queueSize": {"objectCount": 0, "byteCount": 0}
                }
            })))
            .mount(&server)
            .await;

        // DELETE — Drop fires it; no expect() cap.
        Mock::given(method("DELETE"))
            .and(path(
                "/nifi-api/flowfile-queues/q1/listing-requests/req-timeout",
            ))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "listingRequest": {
                    "id": "req-timeout",
                    "state": "CANCELLED",
                    "finished": true
                }
            })))
            .mount(&server)
            .await;

        let local = LocalSet::new();
        local
            .run_until(async {
                let client = test_client(&server).await;
                let (tx, mut rx) = mpsc::channel::<AppEvent>(64);

                let handle = super::spawn_queue_listing_fetch(
                    client,
                    tx,
                    "q1".to_string(),
                    StdDuration::from_secs(1), // short timeout
                );

                // Skip RequestIdAssigned and any Progress emits; hunt for Timeout.
                let mut saw_timeout = false;
                for _ in 0..20 {
                    let ev = tokio::time::timeout(StdDuration::from_secs(5), rx.recv())
                        .await
                        .expect("timeout waiting for payload")
                        .expect("channel closed");
                    match ev {
                        AppEvent::Data(ViewPayload::Browser(
                            BrowserPayload::QueueListingTimeout { queue_id },
                        )) => {
                            assert_eq!(queue_id, "q1");
                            saw_timeout = true;
                            break;
                        }
                        AppEvent::Data(ViewPayload::Browser(
                            BrowserPayload::QueueListingRequestIdAssigned { .. }
                            | BrowserPayload::QueueListingProgress { .. },
                        )) => {
                            // These are expected before the timeout fires.
                        }
                        _ => panic!("unexpected payload"),
                    }
                }
                assert!(saw_timeout, "expected QueueListingTimeout to be emitted");

                drop(handle);
                for _ in 0..10 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(StdDuration::from_millis(20)).await;
                }
            })
            .await;
    }

    /// When the worker hasn't recorded a request id yet (POST hasn't
    /// returned), Drop must NOT fire DELETE — there's nothing to delete.
    #[tokio::test(flavor = "current_thread")]
    async fn drop_skips_delete_before_post_returns() {
        use std::sync::Mutex as StdMutex;
        use std::time::Duration as StdDuration;

        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("DELETE"))
            .and(path(
                "/nifi-api/flowfile-queues/q1/listing-requests/anything",
            ))
            .respond_with(ResponseTemplate::new(200))
            .expect(0)
            .mount(&server)
            .await;

        let local = LocalSet::new();
        local
            .run_until(async {
                let client = test_client(&server).await;
                let request_id = Arc::new(StdMutex::new(None));

                let handle =
                    super::QueueListingHandle::new_for_test("q1".to_string(), request_id, client);
                drop(handle);

                for _ in 0..10 {
                    tokio::task::yield_now().await;
                    tokio::time::sleep(StdDuration::from_millis(20)).await;
                }
            })
            .await;

        server.verify().await;
    }

    /// `spawn_flowfile_peek_fetch` issues `GET /flowfile-queues/{queue}/flowfiles/{uuid}`
    /// and emits `FlowfilePeek` populated with the full `FlowFileDto`'s
    /// attribute map and content-claim summary.
    #[tokio::test(flavor = "current_thread")]
    async fn peek_fetch_emits_attrs_and_content_claim() {
        use std::time::Duration as StdDuration;
        use wiremock::matchers::{method, path};
        use wiremock::{Mock, MockServer, ResponseTemplate};

        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/flowfile-queues/q1/flowfiles/ff-aaaa"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "flowFile": {
                    "uuid": "ff-aaaa",
                    "filename": "a.parquet",
                    "size": 1024,
                    "mimeType": "application/x-parquet",
                    "contentClaimContainer": "default",
                    "contentClaimSection": "1234",
                    "contentClaimIdentifier": "abc",
                    "contentClaimOffset": 0,
                    "contentClaimFileSizeBytes": 1024,
                    "attributes": {
                        "filename": "a.parquet",
                        "record.count": "1000",
                        "uuid": "ff-aaaa"
                    }
                }
            })))
            .mount(&server)
            .await;

        let local = LocalSet::new();
        local
            .run_until(async {
                let client = test_client(&server).await;
                let (tx, mut rx) = mpsc::channel::<AppEvent>(8);
                let _handle = super::spawn_flowfile_peek_fetch(
                    client,
                    tx,
                    "q1".to_string(),
                    "ff-aaaa".to_string(),
                    None,
                );

                let payload = tokio::time::timeout(StdDuration::from_secs(2), rx.recv())
                    .await
                    .expect("payload arrived")
                    .expect("non-empty");
                match payload {
                    AppEvent::Data(ViewPayload::Browser(BrowserPayload::FlowfilePeek {
                        uuid,
                        attrs,
                        content_claim,
                        mime_type,
                        ..
                    })) => {
                        assert_eq!(uuid, "ff-aaaa");
                        assert_eq!(mime_type.as_deref(), Some("application/x-parquet"));
                        assert_eq!(attrs.get("record.count").map(String::as_str), Some("1000"),);
                        let cc = content_claim.expect("content claim populated");
                        assert_eq!(cc.container, "default");
                        assert_eq!(cc.section, "1234");
                        assert_eq!(cc.identifier, "abc");
                        assert_eq!(cc.file_size, 1024);
                    }
                    _ => panic!("expected FlowfilePeek"),
                }
            })
            .await;
    }
}
