//! Browser tab polling worker.
//!
//! Refreshes the full recursive PG tree every 15 seconds and services
//! on-demand per-node detail fetches from the reducer's side-channel.
//! The force-tick oneshot lets the `r` keybind skip the interval wait.
//!
//! Runs under the main-thread `LocalSet` (see `lib::run_inner`) because
//! `nifi-rust-client`'s dynamic dispatch traits return `!Send` futures.

use std::sync::Arc;

use tokio::sync::{RwLock, mpsc, oneshot};
use tokio::task::JoinHandle;
use tokio::time::{Duration, MissedTickBehavior, interval};

use crate::app::worker::send_poll_result;
use crate::client::NifiClient;
use crate::error::NifiLensError;
use crate::event::{AppEvent, BrowserPayload, ViewPayload};
use crate::view::browser::state::{DetailRequest, NodeDetail, NodeDetailSnapshot};

pub fn spawn(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    mut detail_rx: mpsc::UnboundedReceiver<DetailRequest>,
    force_rx: oneshot::Receiver<()>,
    poll_interval: Duration,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut ticker = interval(poll_interval);
        ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);

        // Wrap the oneshot in an Option so we can "disarm" it after it
        // fires — otherwise the select! arm would hot-loop on a closed
        // receiver.
        let mut force_rx: Option<oneshot::Receiver<()>> = Some(force_rx);

        loop {
            tokio::select! {
                _ = ticker.tick() => {
                    fetch_tree_once(&client, &tx).await;
                }
                req = detail_rx.recv() => {
                    let Some(req) = req else {
                        tracing::debug!("browser worker: detail channel closed, exiting");
                        return;
                    };
                    fetch_detail_once(&client, &tx, req).await;
                }
                res = async {
                    match force_rx.as_mut() {
                        Some(rx) => rx.await,
                        None => std::future::pending().await,
                    }
                } => {
                    force_rx = None;
                    if res.is_ok() {
                        fetch_tree_once(&client, &tx).await;
                    }
                }
            }
        }
    })
}

async fn fetch_tree_once(client: &Arc<RwLock<NifiClient>>, tx: &mpsc::Sender<AppEvent>) {
    let guard = client.read().await;
    let result = guard
        .browser_tree()
        .await
        .map(|snap| ViewPayload::Browser(BrowserPayload::Tree(snap)));
    send_poll_result(tx, "browser tree", result).await;
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
