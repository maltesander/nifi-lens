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

/// RAII guard for a server-side lineage query: when dropped, fires a
/// best-effort `DELETE /provenance-events/lineage/{id}` via `spawn_local`.
/// Owned by `spawn_lineage`'s async closure so cleanup happens whether
/// the closure returns normally, encounters an error, or panics.
struct LineageQueryGuard {
    client: Arc<RwLock<NifiClient>>,
    query_id: String,
    cluster_node_id: Option<String>,
}

impl LineageQueryGuard {
    fn new(
        client: Arc<RwLock<NifiClient>>,
        query_id: String,
        cluster_node_id: Option<String>,
    ) -> Self {
        Self {
            client,
            query_id,
            cluster_node_id,
        }
    }

    fn query_id(&self) -> &str {
        &self.query_id
    }

    fn cluster_node_id(&self) -> Option<&str> {
        self.cluster_node_id.as_deref()
    }
}

impl Drop for LineageQueryGuard {
    fn drop(&mut self) {
        let client = self.client.clone();
        let query_id = self.query_id.clone();
        let cluster_node_id = self.cluster_node_id.clone();
        tokio::task::spawn_local(async move {
            let guard = client.read().await;
            if let Err(err) = guard
                .delete_lineage(&query_id, cluster_node_id.as_deref())
                .await
            {
                tracing::warn!(
                    query_id = %query_id,
                    error = %err,
                    "tracer: lineage query Drop-cleanup failed",
                );
            }
        });
    }
}

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
///    - `Finished(snapshot)` → emits `LineageDone`, returns
/// 3. Any error → emits `LineageFailed`
///
/// Once the query is submitted the handle is wrapped in a
/// `LineageQueryGuard` whose `Drop` impl fires a best-effort
/// `delete_lineage`, guaranteeing server-side cleanup regardless of how
/// the closure exits (normal return, channel-closed early-return, or
/// panic during poll).
pub fn spawn_lineage(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    uuid: String,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        // Step 1: submit. On error, no cleanup needed (no server-side query exists).
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

        // Wrap in RAII guard — Drop fires DELETE on every exit path below.
        let lineage_guard =
            LineageQueryGuard::new(client.clone(), query_id.clone(), cluster_node_id.clone());

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
            return; // Guard drops → DELETE fires.
        }

        // Step 3: poll loop
        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;

            let poll_result = {
                let guard = client.read().await;
                guard
                    .poll_lineage(lineage_guard.query_id(), lineage_guard.cluster_node_id())
                    .await
            };
            match poll_result {
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
                    return; // Guard drops → DELETE fires.
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
                    return; // Guard drops → DELETE fires (bug-fix: error branch previously skipped DELETE).
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
                Some(crate::client::tracer::INLINE_PREVIEW_BYTES),
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

/// Streams the content bytes for `(event_id, side)` straight to `path`
/// without buffering the full body in memory. Emits
/// `TracerPayload::ContentSaved` on success or
/// `TracerPayload::ContentSaveFailed` if the fetch, an intermediate
/// chunk, or the write fails.
pub fn spawn_save(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    path: PathBuf,
    event_id: i64,
    side: ContentSide,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        use futures::StreamExt;
        use tokio::io::AsyncWriteExt;

        let mut stream = {
            let guard = client.read().await;
            match guard.provenance_content_stream(event_id, side).await {
                Ok(s) => s,
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

        let mut file = match tokio::fs::File::create(&path).await {
            Ok(f) => f,
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
        };

        let write_result: Result<(), String> = async {
            while let Some(chunk) = stream.next().await {
                let chunk = chunk.map_err(|e| e.to_string())?;
                file.write_all(&chunk).await.map_err(|e| e.to_string())?;
            }
            file.flush().await.map_err(|e| e.to_string())?;
            Ok(())
        }
        .await;

        let payload = match write_result {
            Ok(()) => TracerPayload::ContentSaved { path },
            Err(error) => TracerPayload::ContentSaveFailed { path, error },
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

/// Fetches `len` bytes starting at `offset` for `(event_id, side)` and
/// emits [`TracerPayload::ModalChunk`] on success or
/// [`TracerPayload::ModalChunkFailed`] on error.
///
/// One-shot worker. Handle is not retained — stale deliveries (e.g.,
/// the user closed the modal before the chunk arrived) are filtered
/// by `event_id` in the reducer.
pub fn spawn_modal_chunk(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    event_id: i64,
    side: ContentSide,
    offset: usize,
    len: usize,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        match guard
            .provenance_content_range(event_id, side, offset, len)
            .await
        {
            Ok(snap) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::ModalChunk {
                            event_id: snap.event_id,
                            side: snap.side,
                            offset: snap.offset,
                            bytes: snap.bytes,
                            eof: snap.eof,
                            requested_len: len,
                        },
                    )))
                    .await;
            }
            Err(err) => {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Tracer(
                        TracerPayload::ModalChunkFailed {
                            event_id,
                            side,
                            offset,
                            error: err.to_string(),
                        },
                    )))
                    .await;
            }
        }
    })
}
