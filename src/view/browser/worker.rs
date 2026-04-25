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
