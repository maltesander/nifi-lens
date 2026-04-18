//! The central store. Task 1 ships the skeleton: type, empty fetcher
//! spawn list, shutdown. Later tasks (2–8) add one fetch task per
//! endpoint.

use std::sync::Arc;

use tokio::sync::{Notify, RwLock, mpsc};
use tokio::task::JoinHandle;

use crate::app::state::ViewId;
use crate::client::health::SystemDiagSnapshot;
use crate::client::{
    AboutSnapshot, BulletinSnapshot, ConnectionEndpoints, ControllerServiceCounts,
    ControllerStatusSnapshot, NifiClient, RootPgStatusSnapshot,
};
use crate::cluster::ClusterEndpoint;
use crate::cluster::config::ClusterPollingConfig;
use crate::cluster::snapshot::{ClusterSnapshot, FetchMeta};
use crate::cluster::subscriber::SubscriberRegistry;
use crate::error::NifiLensError;
use crate::event::AppEvent;

pub use crate::cluster::subscriber::SubscriberId;

/// Update emitted by a fetch task on every cycle — even failures, so
/// `EndpointState::Failed` can preserve `last_ok`.
#[derive(Debug)]
pub enum ClusterUpdate {
    About(Result<AboutSnapshot, NifiLensError>, FetchMeta),
    ControllerStatus(Result<ControllerStatusSnapshot, NifiLensError>, FetchMeta),
    RootPgStatus(Result<RootPgStatusSnapshot, NifiLensError>, FetchMeta),
    ControllerServices(Result<ControllerServiceCounts, NifiLensError>, FetchMeta),
    SystemDiagnostics(Result<SystemDiagSnapshot, NifiLensError>, FetchMeta),
    Connections {
        pg_id: String,
        result: Result<ConnectionEndpoints, NifiLensError>,
        meta: FetchMeta,
    },
    BulletinsDelta {
        result: Result<Vec<BulletinSnapshot>, NifiLensError>,
        meta: FetchMeta,
    },
}

impl ClusterUpdate {
    pub fn endpoint(&self) -> ClusterEndpoint {
        match self {
            Self::About(..) => ClusterEndpoint::About,
            Self::ControllerStatus(..) => ClusterEndpoint::ControllerStatus,
            Self::RootPgStatus(..) => ClusterEndpoint::RootPgStatus,
            Self::ControllerServices(..) => ClusterEndpoint::ControllerServices,
            Self::SystemDiagnostics(..) => ClusterEndpoint::SystemDiagnostics,
            Self::Connections { .. } => ClusterEndpoint::ConnectionsByPg,
            Self::BulletinsDelta { .. } => ClusterEndpoint::Bulletins,
        }
    }
}

/// One `Arc<Notify>` per endpoint — UI task signals these on force
/// refresh or subscriber-add.
///
/// `tokio::sync::Notify` does not implement `Default`, so `NotifyMap`
/// cannot derive `Default` even though each element is an
/// `Arc<Notify>`. The manual impl builds one fresh `Notify` per slot
/// via `std::array::from_fn`.
#[derive(Debug)]
struct NotifyMap {
    notifies: [Arc<Notify>; ClusterEndpoint::COUNT],
}

impl Default for NotifyMap {
    fn default() -> Self {
        Self {
            notifies: std::array::from_fn(|_| Arc::new(Notify::new())),
        }
    }
}

impl NotifyMap {
    fn get(&self, endpoint: ClusterEndpoint) -> Arc<Notify> {
        self.notifies[endpoint as usize].clone()
    }
}

pub struct ClusterStore {
    pub snapshot: ClusterSnapshot,
    pub subscribers: SubscriberRegistry,
    config: ClusterPollingConfig,
    notifies: NotifyMap,
    handles: Vec<JoinHandle<()>>,
}

impl std::fmt::Debug for ClusterStore {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClusterStore")
            .field("snapshot", &self.snapshot)
            .field("config", &self.config)
            .field("subscribers", &self.subscribers)
            .finish_non_exhaustive()
    }
}

impl ClusterStore {
    /// Constructs an empty store. Call `spawn_fetchers` to start the
    /// per-endpoint tasks on the current `LocalSet`.
    pub fn new(config: ClusterPollingConfig) -> Self {
        Self {
            snapshot: ClusterSnapshot::default(),
            subscribers: SubscriberRegistry::new(),
            config,
            notifies: NotifyMap::default(),
            handles: Vec::new(),
        }
    }

    /// Spawn every per-endpoint fetch task on the current `LocalSet`.
    /// Task 1 is a no-op; Tasks 2–8 each add one arm.
    pub fn spawn_fetchers(
        &mut self,
        _client: Arc<RwLock<NifiClient>>,
        _tx: mpsc::Sender<AppEvent>,
    ) {
        // Tasks 2–8 populate this.
    }

    pub fn subscribe(&mut self, endpoint: ClusterEndpoint, view: ViewId) {
        if self.subscribers.subscribe(endpoint, SubscriberId(view)) {
            self.notifies.get(endpoint).notify_one();
        }
    }

    pub fn unsubscribe(&mut self, endpoint: ClusterEndpoint, view: ViewId) {
        self.subscribers.unsubscribe(endpoint, SubscriberId(view));
    }

    pub fn force(&self, endpoint: ClusterEndpoint) {
        self.notifies.get(endpoint).notify_one();
    }

    /// Apply an update from a fetch task to the snapshot. Returns the
    /// endpoint that changed so the main loop can fan out
    /// `AppEvent::ClusterChanged(endpoint)`.
    pub fn apply_update(&mut self, update: ClusterUpdate) -> ClusterEndpoint {
        let endpoint = update.endpoint();
        match update {
            ClusterUpdate::About(result, meta) => self.snapshot.about.apply(result, meta),
            ClusterUpdate::ControllerStatus(result, meta) => {
                self.snapshot.controller_status.apply(result, meta)
            }
            ClusterUpdate::RootPgStatus(result, meta) => {
                self.snapshot.root_pg_status.apply(result, meta)
            }
            ClusterUpdate::ControllerServices(result, meta) => {
                self.snapshot.controller_services.apply(result, meta)
            }
            ClusterUpdate::SystemDiagnostics(result, meta) => {
                self.snapshot.system_diagnostics.apply(result, meta)
            }
            ClusterUpdate::Connections {
                pg_id,
                result,
                meta,
            } => {
                self.snapshot
                    .connections_by_pg
                    .entry(pg_id)
                    .or_default()
                    .apply(result, meta);
            }
            ClusterUpdate::BulletinsDelta { result, meta } => {
                // Bulletins merge semantics are task-7 territory; for the
                // skeleton we replace. Task 7 will swap this for a
                // cursor-aware merge.
                self.snapshot.bulletins.apply(result, meta);
            }
        }
        endpoint
    }

    /// Test-only accessor for the per-endpoint `Arc<Notify>`. Used by
    /// unit tests to verify that `subscribe` and `force` wake waiters
    /// without having to spin up a full fetch task.
    #[cfg(test)]
    pub(crate) fn notify_for(&self, endpoint: ClusterEndpoint) -> Arc<Notify> {
        self.notifies.get(endpoint)
    }

    /// Abort all fetch tasks. Called on context switch and shutdown.
    pub fn shutdown(&mut self) {
        tracing::debug!(
            "cluster store: shutting down {} fetch tasks",
            self.handles.len()
        );
        for h in self.handles.drain(..) {
            h.abort();
        }
    }
}

impl Drop for ClusterStore {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cluster::snapshot::EndpointState;
    use std::time::{Duration, Instant};

    fn meta() -> FetchMeta {
        FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(10),
            next_interval: Duration::from_secs(10),
        }
    }

    #[test]
    fn apply_update_routes_to_correct_field() {
        let mut store = ClusterStore::new(ClusterPollingConfig::default());
        let err = NifiLensError::WritesNotImplemented;
        let ep = store.apply_update(ClusterUpdate::About(Err(err), meta()));
        assert_eq!(ep, ClusterEndpoint::About);
        assert!(matches!(store.snapshot.about, EndpointState::Failed { .. }));
        assert!(matches!(
            store.snapshot.controller_status,
            EndpointState::Loading
        ));
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn subscribe_wakes_waiter_on_first_subscriber() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut store = ClusterStore::new(ClusterPollingConfig::default());
                let notify = store.notify_for(ClusterEndpoint::RootPgStatus);
                let flag = std::rc::Rc::new(std::cell::Cell::new(false));
                let flag_clone = flag.clone();

                let waiter = tokio::task::spawn_local(async move {
                    notify.notified().await;
                    flag_clone.set(true);
                });

                // Before subscribe, the waiter hasn't woken.
                tokio::task::yield_now().await;
                assert!(!flag.get(), "waiter fired before subscribe");

                // Subscribing (0 → 1 transition) must call notify_one() internally.
                store.subscribe(ClusterEndpoint::RootPgStatus, ViewId::Overview);
                tokio::task::yield_now().await;
                assert!(flag.get(), "waiter did not wake on first subscribe");
                assert_eq!(store.subscribers.count(ClusterEndpoint::RootPgStatus), 1);

                waiter.abort();
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn force_wakes_waiter() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let store = ClusterStore::new(ClusterPollingConfig::default());
                let notify = store.notify_for(ClusterEndpoint::ControllerStatus);
                let flag = std::rc::Rc::new(std::cell::Cell::new(false));
                let flag_clone = flag.clone();

                let waiter = tokio::task::spawn_local(async move {
                    notify.notified().await;
                    flag_clone.set(true);
                });

                tokio::task::yield_now().await;
                assert!(!flag.get(), "waiter fired before force");

                store.force(ClusterEndpoint::ControllerStatus);
                tokio::task::yield_now().await;
                assert!(flag.get(), "waiter did not wake on force()");

                waiter.abort();
            })
            .await;
    }

    #[tokio::test(flavor = "current_thread")]
    async fn shutdown_then_new_is_clean() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut store = ClusterStore::new(ClusterPollingConfig::default());
                // Pretend we spawned something.
                store.handles.push(tokio::task::spawn_local(async {}));
                store.shutdown();
                assert!(store.handles.is_empty());
                // Drop would also call shutdown — must be idempotent.
                store.shutdown();
            })
            .await;
    }
}
