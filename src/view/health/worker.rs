//! Health tab worker: polls PG status at 10s and system diagnostics
//! at 30s via two independent interval timers in a `tokio::select!`.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{RwLock, mpsc};
use tokio::task::JoinHandle;
use tokio::time::MissedTickBehavior;

use crate::client::NifiClient;
use crate::event::{AppEvent, HealthPayload, ViewPayload};

const PG_STATUS_INTERVAL: Duration = Duration::from_secs(10);
const SYSDIAG_INTERVAL: Duration = Duration::from_secs(30);

pub fn spawn(client: Arc<RwLock<NifiClient>>, tx: mpsc::Sender<AppEvent>) -> JoinHandle<()> {
    tokio::task::spawn_local(async move {
        let mut pg_ticker = tokio::time::interval(PG_STATUS_INTERVAL);
        let mut sysdiag_ticker = tokio::time::interval(SYSDIAG_INTERVAL);
        pg_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        sysdiag_ticker.set_missed_tick_behavior(MissedTickBehavior::Delay);
        loop {
            tokio::select! {
                _ = pg_ticker.tick() => { poll_pg_status(&client, &tx).await; }
                _ = sysdiag_ticker.tick() => { poll_system_diagnostics(&client, &tx).await; }
            }
        }
    })
}

async fn poll_pg_status(client: &Arc<RwLock<NifiClient>>, tx: &mpsc::Sender<AppEvent>) {
    let guard = client.read().await;
    match guard.root_pg_status_full().await {
        Ok(snapshot) => {
            let _ = tx
                .send(AppEvent::Data(ViewPayload::Health(
                    HealthPayload::PgStatus(snapshot),
                )))
                .await;
        }
        Err(err) => {
            tracing::warn!(error = %err, "health worker: PG status poll failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
        }
    }
}

async fn poll_system_diagnostics(client: &Arc<RwLock<NifiClient>>, tx: &mpsc::Sender<AppEvent>) {
    let guard = client.read().await;
    match guard.system_diagnostics(true).await {
        Ok(diag) => {
            let _ = tx
                .send(AppEvent::Data(ViewPayload::Health(
                    HealthPayload::SystemDiag(diag),
                )))
                .await;
        }
        Err(err) => {
            tracing::warn!(error = %err, "health worker: sysdiag poll failed");
            let _ = tx.send(AppEvent::IntentOutcome(Err(err))).await;
        }
    }
}
