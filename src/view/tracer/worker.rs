//! Tracer tab one-shot worker functions.
//!
//! All workers use `tokio::task::spawn_local` because `nifi-rust-client`
//! dynamic dispatch traits return `!Send` futures, and all workers run under
//! the main-thread `LocalSet` (see `lib::run_inner`).
//!
//! Unlike the Bulletins / Browser workers (which are owned by `WorkerRegistry`
//! and loop forever), Tracer workers are one-shot: they perform a single
//! async operation and push the result back via `AppEvent::Data(ViewPayload::Tracer(...))`.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::{ContentSide, NifiClient};
use crate::event::{AppEvent, TracerPayload, ViewPayload};

/// Fetches the latest cached provenance events for a component and emits
/// [`TracerPayload::LatestEvents`] or [`TracerPayload::LatestEventsFailed`].
pub fn spawn_latest_events(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    component_id: String,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        match guard.latest_events(&component_id, 20).await {
            Ok(snap) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::LatestEvents(snap),
                    )))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::LatestEventsFailed {
                            component_id,
                            error: err.to_string(),
                        },
                    )))
                    .await;
            }
        }
    })
}

/// Submits a lineage query, polls until complete, and emits intermediate
/// progress events followed by a final [`TracerPayload::LineageDone`].
///
/// State machine:
/// 1. `submit_lineage` → emits `LineageSubmitted`
/// 2. Loop every 500 ms: `poll_lineage`
///    - `Running { percent }` → emits `LineagePartial`
///    - `Finished(snapshot)` → emits `LineageDone`, best-effort `delete_lineage`, returns
/// 3. Any error → emits `LineageFailed`
pub fn spawn_lineage(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    uuid: String,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        // Step 1: submit
        let (query_id, cluster_node_id) = {
            let guard = client.read().await;
            match guard.submit_lineage(&uuid).await {
                Ok(pair) => pair,
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Tracer(
                            TracerPayload::LineageFailed {
                                uuid,
                                query_id: String::new(),
                                error: err.to_string(),
                            },
                        )))
                        .await;
                    return;
                }
            }
        };

        // Step 2: notify caller of the query_id
        if tx
            .send(AppEvent::Data(ViewPayload::Tracer(
                TracerPayload::LineageSubmitted {
                    uuid: uuid.clone(),
                    query_id: query_id.clone(),
                    cluster_node_id: cluster_node_id.clone(),
                },
            )))
            .await
            .is_err()
        {
            return;
        }

        // Step 3: poll loop
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let guard = client.read().await;
            match guard
                .poll_lineage(&query_id, cluster_node_id.as_deref())
                .await
            {
                Ok(crate::client::LineagePoll::Running { percent }) => {
                    if tx
                        .send(AppEvent::Data(ViewPayload::Tracer(
                            TracerPayload::LineagePartial {
                                query_id: query_id.clone(),
                                percent,
                            },
                        )))
                        .await
                        .is_err()
                    {
                        return;
                    }
                }
                Ok(crate::client::LineagePoll::Finished(snapshot)) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Tracer(
                            TracerPayload::LineageDone {
                                uuid: uuid.clone(),
                                query_id: query_id.clone(),
                                snapshot,
                                fetched_at: SystemTime::now(),
                            },
                        )))
                        .await;
                    // Best-effort cleanup — ignore errors
                    let _ = guard
                        .delete_lineage(&query_id, cluster_node_id.as_deref())
                        .await;
                    return;
                }
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Tracer(
                            TracerPayload::LineageFailed {
                                uuid,
                                query_id,
                                error: err.to_string(),
                            },
                        )))
                        .await;
                    return;
                }
            }
        }
    })
}

/// Fetches the full detail of a single provenance event and emits
/// [`TracerPayload::EventDetail`] or [`TracerPayload::EventDetailFailed`].
pub fn spawn_event_detail(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    event_id: i64,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        match guard.get_provenance_event(event_id).await {
            Ok(detail) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::EventDetail {
                            event_id,
                            detail: Box::new(detail),
                        },
                    )))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::EventDetailFailed {
                            event_id,
                            error: err.to_string(),
                        },
                    )))
                    .await;
            }
        }
    })
}

/// Fetches the raw content for a provenance event (input or output side) and
/// emits [`TracerPayload::Content`] or [`TracerPayload::ContentFailed`].
pub fn spawn_content(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    event_id: i64,
    side: ContentSide,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        match guard
            .provenance_content(
                event_id,
                side,
                Some(crate::client::tracer::PREVIEW_CAP_BYTES),
            )
            .await
        {
            Ok(snap) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(TracerPayload::Content(
                        snap,
                    ))))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::ContentFailed {
                            event_id,
                            side,
                            error: err.to_string(),
                        },
                    )))
                    .await;
            }
        }
    })
}

/// Re-fetches the full content bytes for `(event_id, side)` and
/// writes them to `path`. Emits `TracerPayload::ContentSaved` on
/// success or `TracerPayload::ContentSaveFailed` on fetch or write
/// error.
///
/// The write runs on the blocking thread pool via
/// `tokio::task::spawn_blocking` so it does not block the `LocalSet`.
//
// TODO(nifi-rust-client): switch to a streaming body API when
// upstream adds one, so the full response doesn't need to be
// buffered in memory before writing to disk.
pub fn spawn_save(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    path: PathBuf,
    event_id: i64,
    side: ContentSide,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let bytes = {
            let guard = client.read().await;
            match guard.provenance_content_raw(event_id, side).await {
                Ok(bytes) => bytes,
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Tracer(
                            TracerPayload::ContentSaveFailed {
                                path,
                                error: err.to_string(),
                            },
                        )))
                        .await;
                    return;
                }
            }
        };

        let path_clone = path.clone();
        let result = tokio::task::spawn_blocking(move || std::fs::write(&path_clone, &bytes)).await;

        let payload = match result {
            Ok(Ok(())) => TracerPayload::ContentSaved { path },
            Ok(Err(io_err)) => TracerPayload::ContentSaveFailed {
                path,
                error: io_err.to_string(),
            },
            Err(join_err) => TracerPayload::ContentSaveFailed {
                path,
                error: join_err.to_string(),
            },
        };

        let _ = tx.send(AppEvent::Data(ViewPayload::Tracer(payload))).await;
    })
}

/// Fire-and-forget lineage query deletion. Failures are logged at `warn` level
/// and never surfaced to the user.
pub fn spawn_delete_lineage(
    client: Arc<RwLock<NifiClient>>,
    query_id: String,
    cluster_node_id: Option<String>,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        if let Err(err) = guard
            .delete_lineage(&query_id, cluster_node_id.as_deref())
            .await
        {
            tracing::warn!(
                query_id,
                error = %err,
                "tracer: background lineage delete failed",
            );
        }
    })
}
