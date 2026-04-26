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
