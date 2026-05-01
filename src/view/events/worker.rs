//! Events tab worker: submits a provenance query and polls until done.
//!
//! Mirrors `src/view/tracer/worker.rs` — a one-shot submit → poll loop →
//! best-effort server cleanup task spawned on the main-thread `LocalSet`
//! because the dynamic NiFi client's futures are `!Send`. Also hosts
//! [`spawn_watch`], the long-running tail worker driving the Watch
//! sub-mode (Task 11 / 12).

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use futures::StreamExt;
use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::{
    NifiClient, Predicate, ProvenancePollResult, ProvenanceQuery, ProvenanceQueryHandle,
};
use crate::event::{AppEvent, EventsPayload, ViewPayload};
use crate::view::events::state::TailCursor;

/// How often the worker polls `GET /provenance/{id}` while waiting
/// for the server to mark the query `finished`.
const POLL_INTERVAL: Duration = Duration::from_millis(750);

/// How long the worker is willing to wait for a query to finish
/// before giving up and emitting `QueryFailed`.
const POLL_TIMEOUT: Duration = Duration::from_secs(60);

/// RAII guard for a server-side provenance query: when dropped, fires a
/// best-effort `DELETE /provenance/{id}` via `spawn_local`. Owned by
/// `spawn_query`'s async closure so cleanup happens whether the closure
/// returns normally, encounters an error, or panics during poll.
struct ProvenanceQueryGuard {
    client: Arc<RwLock<NifiClient>>,
    handle: ProvenanceQueryHandle,
}

impl ProvenanceQueryGuard {
    fn new(client: Arc<RwLock<NifiClient>>, handle: ProvenanceQueryHandle) -> Self {
        Self { client, handle }
    }

    /// Borrow the inner handle for use during normal operation.
    fn handle(&self) -> &ProvenanceQueryHandle {
        &self.handle
    }
}

impl Drop for ProvenanceQueryGuard {
    fn drop(&mut self) {
        let client = self.client.clone();
        let handle = self.handle.clone();
        // Drop runs on the worker's task which lives on the main-thread
        // LocalSet — spawn_local is the correct primitive.
        tokio::task::spawn_local(async move {
            let guard = client.read().await;
            if let Err(err) = guard.delete_provenance_query(&handle).await {
                tracing::warn!(
                    query_id = %handle.query_id,
                    error = %err,
                    "events: provenance query Drop-cleanup failed",
                );
            }
        });
    }
}

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
        // Submit. On error, no cleanup needed (no server-side query exists).
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

        // Wrap in RAII guard — Drop fires DELETE on every exit path below.
        let query_guard = ProvenanceQueryGuard::new(client.clone(), handle);

        // Announce the query id so the reducer can lock on matching it.
        if tx
            .send(AppEvent::Data(ViewPayload::Events(
                EventsPayload::QueryStarted {
                    query_id: query_guard.handle().query_id.clone(),
                },
            )))
            .await
            .is_err()
        {
            return; // Guard drops → DELETE fires.
        }

        // Poll loop.
        let start = std::time::Instant::now();
        loop {
            if start.elapsed() > POLL_TIMEOUT {
                let _ = tx
                    .send(AppEvent::Data(ViewPayload::Events(
                        EventsPayload::QueryFailed {
                            query_id: Some(query_guard.handle().query_id.clone()),
                            error: format!("poll timeout after {}s", POLL_TIMEOUT.as_secs()),
                        },
                    )))
                    .await;
                return; // Guard drops → DELETE fires.
            }

            tokio::time::sleep(POLL_INTERVAL).await;

            let poll_result = {
                let guard = client.read().await;
                guard.poll_provenance_query(query_guard.handle()).await
            };
            match poll_result {
                Ok(ProvenancePollResult::Running { percent }) => {
                    if tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryProgress {
                                query_id: query_guard.handle().query_id.clone(),
                                percent,
                            },
                        )))
                        .await
                        .is_err()
                    {
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
                                query_id: query_guard.handle().query_id.clone(),
                                events,
                                fetched_at,
                                truncated,
                            },
                        )))
                        .await;
                    return;
                }
                Err(err) => {
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::QueryFailed {
                                query_id: Some(query_guard.handle().query_id.clone()),
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

/// Exponential backoff for the watch worker's submit/poll retries:
/// 5s → 10s → 30s → 60s, capped. `attempt` is 1-based; `0` is treated
/// as `1` defensively.
fn retry_backoff(attempt: u32) -> Duration {
    match attempt {
        0 | 1 => Duration::from_secs(5),
        2 => Duration::from_secs(10),
        3 => Duration::from_secs(30),
        _ => Duration::from_secs(60),
    }
}

/// Format a `SystemTime` as NiFi's `start_date` wire format —
/// `"MM/DD/YYYY HH:MM:SS UTC"`. NiFi 2.x rejects start/end dates
/// without a named-zone suffix; the rest of the events code already
/// emits this same shape (see `EventsState::build_query`).
fn format_nifi_start_date(t: SystemTime) -> Option<String> {
    use time::OffsetDateTime;
    use time::macros::format_description;
    let nifi_fmt = format_description!("[month]/[day]/[year] [hour]:[minute]:[second] UTC");
    OffsetDateTime::from(t).format(&nifi_fmt).ok()
}

/// Parse the event-time string NiFi puts on summary rows
/// (`"MM/DD/YYYY HH:MM:SS.SSS UTC"`, also accepts ISO-8601 for safety)
/// into a `SystemTime`. Returns `None` on any parse failure — callers
/// fall back to `SystemTime::now()` so a parse miss does not stall
/// cursor advancement.
fn parse_nifi_event_time(s: &str) -> Option<SystemTime> {
    crate::timestamp::parse_nifi_timestamp(s).map(SystemTime::from)
}

/// Spawn a tail-mode provenance watcher.
///
/// Loops:
/// 1. Submit a provenance query with `narrow` plus a `start_date`
///    derived from the current cursor (or the worker's start time on
///    the first iteration).
/// 2. Poll until the server reports `finished` (or `POLL_TIMEOUT`).
/// 3. Fan out per-event detail fetches via
///    `futures::stream::buffer_unordered` bounded by
///    `detail_concurrency`.
/// 4. Apply `predicate` to each detail's attribute map; emit
///    `WatchMatch` for matches, drop non-matches.
/// 5. Advance the cursor and emit `WatchTick` with rolling stats.
/// 6. Sleep `cadence`; loop.
///
/// On submit/poll error: emit `WatchFailed` carrying the next
/// backoff (5s → 10s → 30s → 60s, capped), sleep, and retry. RAII
/// guard fires `DELETE /provenance/{id}` on every exit path
/// including `JoinHandle::abort()` — we reuse `ProvenanceQueryGuard`
/// from `spawn_query`. Worker uses `tokio::task::spawn_local` because
/// the dynamic NiFi futures are `!Send`.
pub fn spawn_watch(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    narrow: ProvenanceQuery,
    predicate: Predicate,
    initial_cursor: Option<TailCursor>,
    cadence: Duration,
    detail_concurrency: usize,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut cursor = initial_cursor;
        let mut ewma_per_sec: f32 = 0.0;
        let mut detail_fetch_errors: u64 = 0;
        let mut consecutive_failures: u32 = 0;

        tracing::info!(
            target: "nifi_lens::view::events::watch",
            predicate = %predicate.redacted(),
            cadence_ms = cadence.as_millis() as u64,
            detail_concurrency,
            "events: watch started",
        );

        loop {
            // Build per-iteration narrow with a cursor-derived start
            // date so we only see strictly-newer events. We bump by
            // 1ms past `last_event_time` to skip the inclusive boundary.
            let mut iter_narrow = narrow.clone();
            if let Some(c) = cursor
                && let Some(s) =
                    format_nifi_start_date(c.last_event_time + Duration::from_millis(1))
            {
                iter_narrow.start_time_iso = Some(s);
            }
            if iter_narrow.max_results == 0 {
                iter_narrow.max_results = 1000;
            }

            let started = std::time::Instant::now();

            // ---------------- Submit ----------------
            let handle = {
                let guard = client.read().await;
                match guard.submit_provenance_query(&iter_narrow).await {
                    Ok(h) => h,
                    Err(err) => {
                        consecutive_failures = consecutive_failures.saturating_add(1);
                        let backoff = retry_backoff(consecutive_failures);
                        let _ = tx
                            .send(AppEvent::Data(ViewPayload::Events(
                                EventsPayload::WatchFailed {
                                    error: err.to_string(),
                                    retry_in_ms: backoff.as_millis() as u64,
                                },
                            )))
                            .await;
                        tracing::info!(
                            target: "nifi_lens::view::events::watch",
                            attempt = consecutive_failures,
                            delay_ms = backoff.as_millis() as u64,
                            error = %err,
                            "events: watch submit failed; backing off",
                        );
                        tokio::time::sleep(backoff).await;
                        continue;
                    }
                }
            };
            consecutive_failures = 0;
            let query_guard = ProvenanceQueryGuard::new(client.clone(), handle.clone());

            // ---------------- Poll ----------------
            let poll_started = std::time::Instant::now();
            let poll_outcome: Result<Vec<crate::client::ProvenanceEventSummary>, String> = loop {
                if poll_started.elapsed() > POLL_TIMEOUT {
                    break Err(format!("poll timeout after {}s", POLL_TIMEOUT.as_secs()));
                }
                tokio::time::sleep(POLL_INTERVAL).await;
                let poll_result = {
                    let guard = client.read().await;
                    guard.poll_provenance_query(query_guard.handle()).await
                };
                match poll_result {
                    Ok(ProvenancePollResult::Running { .. }) => continue,
                    Ok(ProvenancePollResult::Finished { events, .. }) => break Ok(events),
                    Err(err) => break Err(err.to_string()),
                }
            };

            let events = match poll_outcome {
                Ok(e) => e,
                Err(error) => {
                    consecutive_failures = consecutive_failures.saturating_add(1);
                    let backoff = retry_backoff(consecutive_failures);
                    let _ = tx
                        .send(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::WatchFailed {
                                error,
                                retry_in_ms: backoff.as_millis() as u64,
                            },
                        )))
                        .await;
                    // Guard drops here at end of scope → DELETE fires.
                    drop(query_guard);
                    tokio::time::sleep(backoff).await;
                    continue;
                }
            };

            // ---------------- Detail fan-out ----------------
            let scanned = events.len();
            let mut matched = 0usize;
            let cluster_node_id = query_guard.handle().cluster_node_id.clone();

            let detail_stream = futures::stream::iter(events)
                .map(|summary| {
                    let client = client.clone();
                    let cluster_node_id = cluster_node_id.clone();
                    async move {
                        let detail = {
                            let guard = client.read().await;
                            guard
                                .fetch_provenance_event_detail(
                                    summary.event_id,
                                    cluster_node_id.as_deref(),
                                )
                                .await
                        };
                        (summary, detail)
                    }
                })
                .buffer_unordered(detail_concurrency.max(1));
            tokio::pin!(detail_stream);

            while let Some((summary, detail_res)) = detail_stream.next().await {
                match detail_res {
                    Ok(detail) => {
                        if predicate.matches(&detail.attributes) {
                            matched += 1;
                            let _ = tx
                                .send(AppEvent::Data(ViewPayload::Events(
                                    EventsPayload::WatchMatch {
                                        summary: detail.summary.clone(),
                                        attrs: detail.attributes.clone(),
                                    },
                                )))
                                .await;
                        }
                        let advance = cursor
                            .map(|c| detail.summary.event_id > c.last_event_id)
                            .unwrap_or(true);
                        if advance {
                            let last_event_time =
                                parse_nifi_event_time(&detail.summary.event_time_iso)
                                    .unwrap_or_else(SystemTime::now);
                            cursor = Some(TailCursor {
                                last_event_id: detail.summary.event_id,
                                last_event_time,
                            });
                        }
                    }
                    Err(err) => {
                        detail_fetch_errors = detail_fetch_errors.saturating_add(1);
                        tracing::warn!(
                            target: "nifi_lens::view::events::watch",
                            event_id = summary.event_id,
                            error = %err,
                            "events: watch detail fetch failed",
                        );
                    }
                }
            }

            // ---------------- Tick stats ----------------
            let elapsed_secs = started.elapsed().as_secs_f32().max(0.001);
            // EWMA tracks MATCHED events per second (i.e., events that
            // passed the predicate and made it into the rolling buffer).
            // Scanned-but-not-matched events don't count toward the
            // headline rate — when the predicate is blocking everything,
            // the user should see "0.0 ev/s" even if NiFi is producing
            // events as fast as ever.
            let inst = matched as f32 / elapsed_secs;
            // EWMA with alpha=0.3 — same shape as Browser sparkline workers.
            ewma_per_sec = 0.7 * ewma_per_sec + 0.3 * inst;

            let _ = tx
                .send(AppEvent::Data(ViewPayload::Events(
                    EventsPayload::WatchTick {
                        events_per_sec_ewma: ewma_per_sec,
                        last_poll_latency_ms: started.elapsed().as_millis() as u64,
                        scanned,
                        matched,
                        detail_fetch_errors,
                    },
                )))
                .await;

            // Guard drops here → DELETE fires before we sleep on the
            // next iteration's submit. (We don't keep the server-side
            // query around between ticks — each iteration is a fresh
            // narrow window from the cursor forward.)
            drop(query_guard);
            tokio::time::sleep(cadence).await;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::Predicate;
    use crate::event::{AppEvent, EventsPayload, ViewPayload};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
    use std::time::Duration;
    use tokio::sync::{RwLock, mpsc};
    use wiremock::matchers::{method, path, path_regex};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    /// Build a `NifiClient` against the wiremock server with a stubbed
    /// `/nifi-api/flow/about` so `detect_version` succeeds.
    async fn test_client(server: &MockServer) -> Arc<RwLock<NifiClient>> {
        Mock::given(method("GET"))
            .and(path("/nifi-api/flow/about"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "about": {"version": "2.6.0", "title": "NiFi"}
            })))
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

    /// Test A — happy path: one event in the tail batch, detail fetch
    /// succeeds, predicate matches → emit `WatchMatch` and `WatchTick`.
    #[tokio::test(flavor = "current_thread")]
    async fn spawn_watch_emits_match_and_tick_for_one_matching_event() {
        let server = MockServer::start().await;

        Mock::given(method("POST"))
            .and(path("/nifi-api/provenance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenance": { "id": "q1", "request": {}, "results": null }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path("/nifi-api/provenance/q1"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenance": {
                    "id": "q1",
                    "finished": true,
                    "percentCompleted": 100,
                    "results": {
                        "provenanceEvents": [{
                            "id": "1",
                            "eventId": 1,
                            "eventTime": "04/30/2026 10:00:00.000 UTC",
                            "eventType": "SEND",
                            "componentId": "c",
                            "componentName": "n",
                            "componentType": "T",
                            "groupId": "g",
                            "flowFileUuid": "ff"
                        }]
                    }
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("GET"))
            .and(path_regex(r"^/nifi-api/provenance-events/1$"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenanceEvent": {
                    "id": "1",
                    "eventId": 1,
                    "eventTime": "04/30/2026 10:00:00.000 UTC",
                    "eventType": "SEND",
                    "componentId": "c",
                    "componentName": "n",
                    "componentType": "T",
                    "groupId": "g",
                    "flowFileUuid": "ff",
                    "attributes": [
                        {"name": "filename", "value": "invoice-1.json", "previousValue": null}
                    ]
                }
            })))
            .mount(&server)
            .await;

        Mock::given(method("DELETE"))
            .and(path("/nifi-api/provenance/q1"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(64);

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let predicate = Predicate::parse("filename =~ /^invoice-/").expect("parse");
                let narrow = ProvenanceQuery {
                    component_id: Some("c".into()),
                    event_types: vec![],
                    max_results: 1000,
                    ..Default::default()
                };
                let handle = spawn_watch(
                    client.clone(),
                    tx,
                    narrow,
                    predicate,
                    None,
                    Duration::from_millis(50),
                    4,
                );

                let mut got_match = false;
                let mut got_tick = false;
                for _ in 0..20 {
                    match tokio::time::timeout(Duration::from_secs(2), rx.recv()).await {
                        Ok(Some(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::WatchMatch { summary, attrs },
                        )))) => {
                            assert_eq!(summary.event_id, 1);
                            assert!(attrs.iter().any(|a| a.key == "filename"));
                            got_match = true;
                        }
                        Ok(Some(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::WatchTick { .. },
                        )))) => {
                            got_tick = true;
                        }
                        _ => break,
                    }
                    if got_match && got_tick {
                        break;
                    }
                }
                handle.abort();
                assert!(got_match, "expected at least one WatchMatch");
                assert!(got_tick, "expected at least one WatchTick");
            })
            .await;
    }

    /// Test B — submit failure → `WatchFailed`, then on retry the
    /// second submit succeeds and the worker reaches a `WatchTick`.
    /// Runs in real time, which makes the test wait the full 5s
    /// backoff once. That is acceptable for one test (~6s wall-clock)
    /// and sidesteps the wiremock-vs-paused-time interaction (real
    /// socket I/O looks "idle" to the runtime under `start_paused`
    /// and auto-advances `timeout` past the recv).
    #[tokio::test(flavor = "current_thread")]
    async fn spawn_watch_emits_failed_on_submit_error_then_recovers() {
        let server = MockServer::start().await;
        let calls = Arc::new(AtomicU32::new(0));
        let calls_in = calls.clone();
        Mock::given(method("POST"))
            .and(path("/nifi-api/provenance"))
            .respond_with(move |_: &wiremock::Request| {
                let n = calls_in.fetch_add(1, Ordering::SeqCst);
                if n == 0 {
                    ResponseTemplate::new(500)
                } else {
                    ResponseTemplate::new(200).set_body_json(serde_json::json!({
                        "provenance": { "id": "q2", "request": {}, "results": null }
                    }))
                }
            })
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/provenance/q2"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenance": {
                    "id": "q2",
                    "finished": true,
                    "percentCompleted": 100,
                    "results": { "provenanceEvents": [] }
                }
            })))
            .mount(&server)
            .await;
        Mock::given(method("DELETE"))
            .and(path("/nifi-api/provenance/q2"))
            .respond_with(ResponseTemplate::new(200))
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, mut rx) = mpsc::channel::<AppEvent>(64);

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let handle = spawn_watch(
                    client.clone(),
                    tx,
                    ProvenanceQuery {
                        component_id: Some("c".into()),
                        event_types: vec![],
                        max_results: 1000,
                        ..Default::default()
                    },
                    Predicate::default(),
                    None,
                    Duration::from_millis(50),
                    4,
                );

                let mut got_failed = false;
                let mut got_tick = false;
                // Generous overall deadline. The first iteration emits
                // WatchFailed quickly (no sleep before), then sleeps 5s
                // backoff, then the recovery iteration emits WatchTick.
                let deadline = std::time::Instant::now() + Duration::from_secs(10);
                while std::time::Instant::now() < deadline && !(got_failed && got_tick) {
                    let remaining = deadline.saturating_duration_since(std::time::Instant::now());
                    match tokio::time::timeout(remaining, rx.recv()).await {
                        Ok(Some(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::WatchFailed { .. },
                        )))) => {
                            got_failed = true;
                        }
                        Ok(Some(AppEvent::Data(ViewPayload::Events(
                            EventsPayload::WatchTick { .. },
                        )))) => {
                            got_tick = true;
                        }
                        Ok(_) => {}
                        Err(_) => break,
                    }
                }
                handle.abort();
                assert!(got_failed, "expected WatchFailed after submit 500");
                assert!(got_tick, "expected WatchTick after recovery");
            })
            .await;
    }

    /// Test C — abort fires `DELETE` through the RAII guard. We let
    /// the worker submit and start polling (the poll mock returns
    /// `finished=false` so the worker stays in the poll loop), then
    /// abort the `JoinHandle`. Drop on `ProvenanceQueryGuard` spawns
    /// the DELETE on the LocalSet; we keep the LocalSet alive after
    /// abort to drive that follow-up task.
    #[tokio::test(flavor = "current_thread")]
    async fn spawn_watch_aborts_cleanly_and_fires_delete() {
        let server = MockServer::start().await;
        Mock::given(method("POST"))
            .and(path("/nifi-api/provenance"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenance": { "id": "q3", "request": {}, "results": null }
            })))
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(path("/nifi-api/provenance/q3"))
            .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
                "provenance": {
                    "id": "q3",
                    "finished": false,
                    "percentCompleted": 50,
                    "results": null
                }
            })))
            .mount(&server)
            .await;
        let delete_called = Arc::new(AtomicBool::new(false));
        let delete_called_in = delete_called.clone();
        Mock::given(method("DELETE"))
            .and(path("/nifi-api/provenance/q3"))
            .respond_with(move |_: &wiremock::Request| {
                delete_called_in.store(true, Ordering::SeqCst);
                ResponseTemplate::new(200)
            })
            .mount(&server)
            .await;

        let client = test_client(&server).await;
        let (tx, _rx) = mpsc::channel::<AppEvent>(64);

        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let handle = spawn_watch(
                    client.clone(),
                    tx,
                    ProvenanceQuery {
                        component_id: Some("c".into()),
                        max_results: 1000,
                        event_types: vec![],
                        ..Default::default()
                    },
                    Predicate::default(),
                    None,
                    Duration::from_millis(50),
                    4,
                );
                // Let the worker submit + start polling.
                tokio::time::sleep(Duration::from_millis(900)).await;
                handle.abort();
                // Drive the LocalSet so the DELETE task spawned by
                // ProvenanceQueryGuard::drop gets to run.
                tokio::time::sleep(Duration::from_millis(500)).await;
                assert!(
                    delete_called.load(Ordering::SeqCst),
                    "expected DELETE to fire on abort",
                );
            })
            .await;
    }

    #[test]
    fn retry_backoff_progression() {
        assert_eq!(retry_backoff(0), Duration::from_secs(5));
        assert_eq!(retry_backoff(1), Duration::from_secs(5));
        assert_eq!(retry_backoff(2), Duration::from_secs(10));
        assert_eq!(retry_backoff(3), Duration::from_secs(30));
        assert_eq!(retry_backoff(4), Duration::from_secs(60));
        assert_eq!(retry_backoff(99), Duration::from_secs(60));
    }
}
