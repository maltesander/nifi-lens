//! Bulletins tab polling worker.
//!
//! Polls `flow_api().get_bulletin_board(after, limit)` on a 5-second
//! cadence and ships each batch back to the UI task as
//! `AppEvent::Data(ViewPayload::Bulletins(...))`. The cursor (`last_id`)
//! is kept locally: the worker receives it at spawn time, updates it on
//! each batch, and never writes to shared state. On tab re-entry, the
//! `WorkerRegistry` reads the cursor from `AppState.bulletins.last_id`
//! and passes it into a new `spawn` call so polling resumes without
//! re-seeding the ring.
//!
//! Runs under the main-thread `LocalSet` (see `lib::run_inner`) because
//! `nifi-rust-client`'s dynamic dispatch traits return `!Send` futures.

use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::client::NifiClient;
use crate::event::{AppEvent, BulletinsPayload, ViewPayload};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const POLL_LIMIT: u32 = 1000;

pub fn spawn(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    initial_last_id: Option<i64>,
) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut ticker = tokio::time::interval(POLL_INTERVAL);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        let mut last_id = initial_last_id;
        loop {
            ticker.tick().await;
            match poll_once(&client, last_id).await {
                Ok(payload) => {
                    if let Some(max) = payload.bulletins.iter().map(|b| b.id).max() {
                        last_id = Some(match last_id {
                            Some(existing) => existing.max(max),
                            None => max,
                        });
                    }
                    if tx
                        .send(AppEvent::Data(ViewPayload::Bulletins(payload)))
                        .await
                        .is_err()
                    {
                        tracing::debug!("bulletins worker: channel closed, exiting");
                        return;
                    }
                }
                Err(err) => {
                    tracing::warn!(error = %err, "bulletins worker: poll failed");
                    if tx.send(AppEvent::IntentOutcome(Err(err))).await.is_err() {
                        return;
                    }
                }
            }
        }
    })
}

async fn poll_once(
    client: &Arc<RwLock<NifiClient>>,
    after_id: Option<i64>,
) -> Result<BulletinsPayload, crate::error::NifiLensError> {
    let guard = client.read().await;
    let board = guard.bulletin_board(after_id, Some(POLL_LIMIT)).await?;
    Ok(BulletinsPayload {
        bulletins: board.bulletins,
        fetched_at: SystemTime::now(),
    })
}
