//! Bulletins tab polling worker.
//!
//! Polls `flow().get_bulletin_board(after, limit)` on a 5-second
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

use std::cell::Cell;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::app::worker::spawn_polling_worker;
use crate::client::NifiClient;
use crate::event::{AppEvent, BulletinsPayload, ViewPayload};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const POLL_LIMIT: u32 = 1000;

pub fn spawn(
    client: Arc<RwLock<NifiClient>>,
    tx: mpsc::Sender<AppEvent>,
    initial_last_id: Option<i64>,
) -> JoinHandle<()> {
    // Rc<Cell> is safe here: the worker runs on a single-threaded LocalSet
    // and only one poll is in flight at a time.
    let last_id = Rc::new(Cell::new(initial_last_id));
    spawn_polling_worker(
        POLL_INTERVAL,
        move || {
            let client = client.clone();
            let last_id = Rc::clone(&last_id);
            async move {
                let payload = poll_once(&client, last_id.get()).await?;
                if let Some(max) = payload.bulletins.iter().map(|b| b.id).max() {
                    let updated = match last_id.get() {
                        Some(existing) => existing.max(max),
                        None => max,
                    };
                    last_id.set(Some(updated));
                }
                Ok(ViewPayload::Bulletins(payload))
            }
        },
        tx,
    )
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
