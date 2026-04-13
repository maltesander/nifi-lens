//! Events tab worker: submits a provenance query and polls until done.
//!
//! Mirrors `src/view/tracer/worker.rs` — a one-shot submit → poll loop →
//! best-effort server cleanup task spawned on the main-thread `LocalSet`
//! because the dynamic NiFi client's futures are `!Send`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::{NifiClient, ProvenancePollResult, ProvenanceQuery, ProvenanceQueryHandle};
use crate::event::{AppEvent, EventsPayload, ViewPayload};

/// How often the worker polls `GET /provenance/{id}` while waiting
/// for the server to mark the query `finished`.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// How long the worker is willing to wait for a query to finish
/// before giving up and emitting `QueryFailed`.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// Spawn a provenance query: submit, then poll until `finished = true`
/// (or timeout), then best-effort-delete the server-side query.
///
/// Emits, in order:
/// 1. `QueryStarted { query_id }` once the server accepts the submission.
/// 2. Zero or more `QueryProgress { query_id, percent }` while polling.
/// 3. One of `QueryDone { .. }` or `QueryFailed { .. }` as the terminal
///    event.
///
/// On error, `QueryFailed` is emitted and the task exits. Returns the
/// `JoinHandle<()>` so the caller can cancel the task if the user
/// requests a new query.
pub fn spawn_query(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    query: ProvenanceQuery,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        // Submit the query under the read lock.
        let handle = {
            let guard = client.read().await;
            match guard.submit_provenance_query(&query).await {
                Ok(h) => h,
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryFailed {
                                query_id: None,
                                error: err.to_string(),
                            },
                        )))
                        .await;
                    return;
                }
            }
        };

        // Announce the query id so the reducer can lock on matching it.
        if tx
            .send(AppEvent::Data(ViewPayload::Events(
                EventsPayload::QueryStarted {
                    query_id: handle.query_id.clone(),
                },
            )))
            .await
            .is_err()
        {
            // Channel closed — receiver is gone. Best-effort cleanup and exit.
            let guard = client.read().await;
            let _ = guard.delete_provenance_query(&handle).await;
            return;
        }

        // Poll loop.
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > POLL_TIMEOUT {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Events(
                        EventsPayload::QueryFailed {
                            query_id: Some(handle.query_id.clone()),
                            error: format!("poll timeout after {}s", POLL_TIMEOUT.as_secs()),
                        },
                    )))
                    .await;
                let guard = client.read().await;
                let _ = guard.delete_provenance_query(&handle).await;
                return;
            }

            tokio::time::sleep(POLL_INTERVAL).await;

            let poll_result = {
                let guard = client.read().await;
                guard.poll_provenance_query(&handle).await
            };
            match poll_result {
                Ok(ProvenancePollResult::Running { percent }) => {
                    if tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryProgress {
                                query_id: handle.query_id.clone(),
                                percent,
                            },
                        )))
                        .await
                        .is_err()
                    {
                        let guard = client.read().await;
                        let _ = guard.delete_provenance_query(&handle).await;
                        return;
                    }
                }
                Ok(ProvenancePollResult::Finished {
                    events,
                    fetched_at,
                    truncated,
                }) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryDone {
                                query_id: handle.query_id.clone(),
                                events,
                                fetched_at,
                                truncated,
                            },
                        )))
                        .await;
                    let guard = client.read().await;
                    let _ = guard.delete_provenance_query(&handle).await;
                    return;
                }
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryFailed {
                                query_id: Some(handle.query_id.clone()),
                                error: err.to_string(),
                            },
                        )))
                        .await;
                    let guard = client.read().await;
                    let _ = guard.delete_provenance_query(&handle).await;
                    return;
                }
            }
        }
    })
}

/// Best-effort server-side cancellation. Spawns a fire-and-forget task
/// that calls `DELETE /provenance/{id}` and drops any error. Used when
/// the UI wants to cancel an in-flight query whose `JoinHandle` has
/// already been aborted.
pub fn spawn_cancel(
    client: Arc<RwLock<NifiClient>>,
    handle: ProvenanceQueryHandle,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let guard = client.read().await;
        if let Err(err) = guard.delete_provenance_query(&handle).await {
            tracing::warn!(
                query_id = %handle.query_id,
                error = %err,
                "events: background provenance cancel failed",
            );
        }
    })
}
