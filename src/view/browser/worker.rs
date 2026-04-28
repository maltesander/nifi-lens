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

use std::sync::Arc;

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
}

/// Placeholder — replaced in Task 5 with the full Drop-DELETE wrapper.
#[derive(Debug)]
pub struct QueueListingHandle {
    _opaque: (),
}
