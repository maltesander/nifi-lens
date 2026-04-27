//! The central store. Task 1 ships the skeleton: type, empty fetcher
//! spawn list, shutdown. Later tasks (2–8) add one fetch task per
//! endpoint.

use std::sync::Arc;

use tokio::sync::{Notify, RwLock, mpsc, watch};
use tokio::task::JoinHandle;

use crate::app::state::ViewId;
use crate::client::overview::{ClusterNodesSnapshot, SystemDiagSnapshot};
use crate::client::{
    AboutSnapshot, BulletinSnapshot, ConnectionEndpoints, ControllerServicesSnapshot,
    ControllerStatusSnapshot, NifiClient, RootPgStatusSnapshot,
};
use crate::cluster::ClusterEndpoint;
use crate::cluster::config::ClusterPollingConfig;
use crate::cluster::fetcher_tasks::{
    FetchTaskConfig, spawn_about, spawn_bulletins, spawn_cluster_nodes, spawn_connections_by_pg,
    spawn_controller_services, spawn_controller_status, spawn_parameter_context_bindings,
    spawn_root_pg_status, spawn_system_diagnostics, spawn_tls_certs, spawn_version_control,
};
use crate::cluster::snapshot::{
    ClusterSnapshot, FetchMeta, ParameterContextBindingsMap, VersionControlMap,
};
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
    ControllerServices(Result<ControllerServicesSnapshot, NifiLensError>, FetchMeta),
    SystemDiagnostics(Result<SystemDiagSnapshot, NifiLensError>, FetchMeta),
    ClusterNodes(Result<ClusterNodesSnapshot, NifiLensError>, FetchMeta),
    TlsCerts(
        Result<crate::client::tls_cert::TlsCertsSnapshot, NifiLensError>,
        FetchMeta,
    ),
    Connections {
        pg_id: String,
        result: Result<ConnectionEndpoints, NifiLensError>,
        meta: FetchMeta,
    },
    BulletinsDelta {
        result: Result<Vec<BulletinSnapshot>, NifiLensError>,
        meta: FetchMeta,
    },
    VersionControl(Result<VersionControlMap, NifiLensError>, FetchMeta),
    ParameterContextBindings(
        Result<ParameterContextBindingsMap, NifiLensError>,
        FetchMeta,
    ),
}

impl ClusterUpdate {
    pub fn endpoint(&self) -> ClusterEndpoint {
        match self {
            Self::About(..) => ClusterEndpoint::About,
            Self::ControllerStatus(..) => ClusterEndpoint::ControllerStatus,
            Self::RootPgStatus(..) => ClusterEndpoint::RootPgStatus,
            Self::ControllerServices(..) => ClusterEndpoint::ControllerServices,
            Self::SystemDiagnostics(..) => ClusterEndpoint::SystemDiagnostics,
            Self::ClusterNodes(..) => ClusterEndpoint::ClusterNodes,
            Self::TlsCerts(..) => ClusterEndpoint::TlsCerts,
            Self::Connections { .. } => ClusterEndpoint::ConnectionsByPg,
            Self::BulletinsDelta { .. } => ClusterEndpoint::Bulletins,
            Self::VersionControl(..) => ClusterEndpoint::VersionControl,
            Self::ParameterContextBindings(..) => ClusterEndpoint::ParameterContextBindings,
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
    /// Latest PG-id fan-out list published from the UI task whenever
    /// `RootPgStatus` delivers a successful update. Consumed by the
    /// connections-by-PG fetcher's three-way `select!`.
    pg_ids_tx: watch::Sender<Vec<String>>,
    /// Receiver half retained so `spawn_fetchers` can clone one into the
    /// connections-by-PG fetcher on each (re)spawn — including context
    /// switches where the channel would otherwise be unreachable.
    pg_ids_rx: watch::Receiver<Vec<String>>,
    /// Latest node-address fan-out list published from the UI task after
    /// every `ClusterUpdate::ClusterNodes` is applied. Consumed by the
    /// tls-certs fetcher to know which nodes to probe.
    node_addresses_tx: watch::Sender<Vec<String>>,
    /// Receiver half retained so `spawn_fetchers` can clone one into the
    /// tls-certs fetcher on each (re)spawn — including context switches.
    node_addresses_rx: watch::Receiver<Vec<String>>,
    /// Base URL of the active NiFi context (e.g. `"https://nifi.host:8443"`).
    /// Stored here so `spawn_fetchers` can hand it to the tls-certs fetcher
    /// without needing the full `NifiClient`.
    base_url: String,
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
    ///
    /// `bulletins_capacity` sizes the cluster-owned `BulletinRing` and
    /// is sourced from `config.bulletins.ring_size` by callers. Passing
    /// `0` is technically valid (zero-capacity ring) but makes the ring
    /// useless — the config loader enforces a 100..=100_000 bound.
    ///
    /// `base_url` is the base URL of the active NiFi context (e.g.
    /// `"https://nifi.host:8443"`), stored for later use by
    /// `spawn_fetchers` when wiring the tls-certs fetcher.
    pub fn new(config: ClusterPollingConfig, bulletins_capacity: usize, base_url: String) -> Self {
        let (pg_ids_tx, pg_ids_rx) = watch::channel(Vec::<String>::new());
        let (node_addresses_tx, node_addresses_rx) = watch::channel(Vec::<String>::new());
        Self {
            snapshot: ClusterSnapshot::with_bulletins_capacity(bulletins_capacity),
            subscribers: SubscriberRegistry::new(),
            config,
            notifies: NotifyMap::default(),
            handles: Vec::new(),
            pg_ids_tx,
            pg_ids_rx,
            node_addresses_tx,
            node_addresses_rx,
            base_url,
        }
    }

    /// Returns the base URL of the active NiFi context stored on this store.
    pub fn base_url(&self) -> &str {
        &self.base_url
    }

    /// Subscribe to the watch channel that publishes node addresses
    /// (host:port) extracted from `ClusterNodesSnapshot.rows`. The
    /// tls-certs fetcher uses this to know which nodes to probe.
    pub fn node_addresses_receiver(&self) -> watch::Receiver<Vec<String>> {
        self.node_addresses_rx.clone()
    }

    /// Publish the node-address list used by the `tls_certs` fetcher
    /// on the watch channel. Called from the main loop after every
    /// `ClusterUpdate::ClusterNodes` and `ClusterUpdate::SystemDiagnostics`
    /// apply.
    ///
    /// Source precedence: `cluster_nodes` (definitive cluster roster)
    /// when non-empty, else `system_diagnostics.nodes` (so standalone
    /// NiFi — where `/controller/cluster` returns 409 — still probes
    /// the address sysdiag reports, matching the join key used by
    /// `update_nodes`).
    ///
    /// Force-wakes the `tls_certs` fetcher when the resulting list
    /// actually changed — otherwise a fetcher that ran its first cycle
    /// against an empty watch would wait a full cadence tick before
    /// re-probing.
    pub fn publish_node_addresses(&self) {
        let addrs: Vec<String> = match self.snapshot.cluster_nodes.latest() {
            Some(snap) if !snap.rows.is_empty() => {
                snap.rows.iter().map(|r| r.address.clone()).collect()
            }
            _ => self
                .snapshot
                .system_diagnostics
                .latest()
                .map(|s| s.nodes.iter().map(|n| n.address.clone()).collect())
                .unwrap_or_default(),
        };
        let changed = self.node_addresses_tx.send_if_modified(|current| {
            if *current != addrs {
                *current = addrs;
                true
            } else {
                false
            }
        });
        if changed {
            self.force(ClusterEndpoint::TlsCerts);
        }
    }

    /// Publish the latest PG-id list (from `snapshot.root_pg_status`) on
    /// the watch channel so the connections-by-PG fetcher fans out over
    /// the refreshed set. Called from the main loop right after
    /// `apply_update` routes a `RootPgStatus` variant, regardless of
    /// whether any view cares about the overview-side redraw.
    ///
    /// No-op when the endpoint has never succeeded. Send errors — which
    /// only happen when every receiver has been dropped, e.g. before
    /// `spawn_fetchers` has been called — are ignored.
    pub fn publish_pg_ids(&self) {
        let Some(snap) = self.snapshot.root_pg_status.latest() else {
            return;
        };
        let ids = snap.pg_ids();
        if self.pg_ids_tx.send(ids).is_err() {
            tracing::trace!("publish_pg_ids: no receivers (fetchers not yet spawned or torn down)");
        }
    }

    /// Spawn every per-endpoint fetch task on the current `LocalSet`.
    /// Tasks 2–8 each add one arm.
    pub fn spawn_fetchers(&mut self, client: Arc<RwLock<NifiClient>>, tx: mpsc::Sender<AppEvent>) {
        let status_cfg = FetchTaskConfig {
            base_interval: self.config.controller_status,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::ControllerStatus),
            gated: false,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::ControllerStatus),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_controller_status(
            client.clone(),
            tx.clone(),
            status_cfg,
        ));

        let pg_cfg = FetchTaskConfig {
            base_interval: self.config.root_pg_status,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::RootPgStatus),
            gated: true,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::RootPgStatus),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles
            .push(spawn_root_pg_status(client.clone(), tx.clone(), pg_cfg));

        let cs_cfg = FetchTaskConfig {
            base_interval: self.config.controller_services,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::ControllerServices),
            gated: true,
            subscriber_counter: self
                .subscribers
                .counter(ClusterEndpoint::ControllerServices),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_controller_services(
            client.clone(),
            tx.clone(),
            cs_cfg,
        ));

        let conns_cfg = FetchTaskConfig {
            base_interval: self.config.connections_by_pg,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::ConnectionsByPg),
            gated: true,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::ConnectionsByPg),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_connections_by_pg(
            client.clone(),
            tx.clone(),
            self.pg_ids_rx.clone(),
            conns_cfg,
        ));

        let bulletins_cfg = FetchTaskConfig {
            base_interval: self.config.bulletins,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::Bulletins),
            gated: false,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::Bulletins),
            batch_concurrency: self.config.batch_concurrency,
        };
        // Initialize the bulletin-fetch cursor from whatever the ring
        // already observed. On fresh startup this is `None` → 0 → the
        // fetcher sends `after_id = None`. On context switch the store
        // is new and also 0-initialized.
        let bulletins_cursor = Arc::new(std::sync::atomic::AtomicI64::new(
            self.snapshot.bulletins.last_id.unwrap_or(0),
        ));
        self.handles.push(spawn_bulletins(
            client.clone(),
            tx.clone(),
            bulletins_cursor,
            bulletins_cfg,
        ));

        let sysdiag_cfg = FetchTaskConfig {
            base_interval: self.config.system_diagnostics,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::SystemDiagnostics),
            gated: false,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::SystemDiagnostics),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_system_diagnostics(
            client.clone(),
            tx.clone(),
            sysdiag_cfg,
        ));

        let about_cfg = FetchTaskConfig {
            base_interval: self.config.about,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::About),
            gated: false,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::About),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles
            .push(spawn_about(client.clone(), tx.clone(), about_cfg));

        let cluster_nodes_cfg = FetchTaskConfig {
            base_interval: self.config.cluster_nodes,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::ClusterNodes),
            gated: true,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::ClusterNodes),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_cluster_nodes(
            client.clone(),
            tx.clone(),
            cluster_nodes_cfg,
        ));

        let tls_cfg = FetchTaskConfig {
            base_interval: self.config.tls_certs,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::TlsCerts),
            gated: true,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::TlsCerts),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_tls_certs(
            tx.clone(),
            self.node_addresses_rx.clone(),
            self.base_url.clone(),
            tls_cfg,
        ));

        let version_control_cfg = FetchTaskConfig {
            base_interval: self.config.version_control,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::VersionControl),
            gated: true,
            subscriber_counter: self.subscribers.counter(ClusterEndpoint::VersionControl),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_version_control(
            client.clone(),
            tx.clone(),
            self.pg_ids_rx.clone(),
            version_control_cfg,
        ));

        let parameter_context_bindings_cfg = FetchTaskConfig {
            base_interval: self.config.parameter_context_bindings,
            max_interval: self.config.max_interval,
            jitter_percent: self.config.jitter_percent,
            force: self.notifies.get(ClusterEndpoint::ParameterContextBindings),
            gated: true,
            subscriber_counter: self
                .subscribers
                .counter(ClusterEndpoint::ParameterContextBindings),
            batch_concurrency: self.config.batch_concurrency,
        };
        self.handles.push(spawn_parameter_context_bindings(
            client.clone(),
            tx.clone(),
            self.pg_ids_rx.clone(),
            parameter_context_bindings_cfg,
        ));
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
            ClusterUpdate::ClusterNodes(result, meta) => {
                self.snapshot.cluster_nodes.apply(result, meta)
            }
            ClusterUpdate::TlsCerts(result, meta) => self.snapshot.tls_certs.apply(result, meta),
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
            ClusterUpdate::BulletinsDelta { result, meta } => match result {
                Ok(batch) => {
                    self.snapshot.bulletins.merge(batch);
                    self.snapshot.bulletins.meta = Some(meta);
                    self.snapshot.bulletins.last_error = None;
                }
                Err(err) => {
                    self.snapshot.bulletins.last_error = Some(err.to_string());
                    self.snapshot.bulletins.meta = Some(meta);
                }
            },
            ClusterUpdate::VersionControl(result, meta) => {
                self.snapshot.version_control.apply(result, meta)
            }
            ClusterUpdate::ParameterContextBindings(result, meta) => {
                self.snapshot.parameter_context_bindings.apply(result, meta);
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
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let err = NifiLensError::WritesNotImplemented;
        let ep = store.apply_update(ClusterUpdate::About(Err(err), meta()));
        assert_eq!(ep, ClusterEndpoint::About);
        assert!(matches!(store.snapshot.about, EndpointState::Failed { .. }));
        assert!(matches!(
            store.snapshot.controller_status,
            EndpointState::Loading
        ));
    }

    #[test]
    fn controller_status_update_is_applied() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let fake_status = ControllerStatusSnapshot {
            running: 1,
            stopped: 0,
            invalid: 0,
            disabled: 0,
            active_threads: 0,
            flow_files_queued: 0,
            bytes_queued: 0,
            stale: 0,
            locally_modified: 0,
            sync_failure: 0,
            up_to_date: 0,
        };
        let ep = store.apply_update(ClusterUpdate::ControllerStatus(
            Ok(fake_status.clone()),
            meta(),
        ));
        assert_eq!(ep, ClusterEndpoint::ControllerStatus);
        match &store.snapshot.controller_status {
            EndpointState::Ready { data, .. } => assert_eq!(data.running, 1),
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn controller_services_update_is_applied() {
        use crate::client::ControllerServiceCounts;
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let fake_cs = ControllerServicesSnapshot {
            counts: ControllerServiceCounts {
                enabled: 4,
                disabled: 1,
                invalid: 2,
            },
            members: Vec::new(),
        };
        let ep = store.apply_update(ClusterUpdate::ControllerServices(Ok(fake_cs), meta()));
        assert_eq!(ep, ClusterEndpoint::ControllerServices);
        match &store.snapshot.controller_services {
            EndpointState::Ready { data, .. } => {
                assert_eq!(data.counts.enabled, 4);
                assert_eq!(data.counts.disabled, 1);
                assert_eq!(data.counts.invalid, 2);
                assert_eq!(data.counts.total(), 7);
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn root_pg_status_update_is_applied() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let fake_pg = RootPgStatusSnapshot {
            flow_files_queued: 42,
            bytes_queued: 1024,
            connections: vec![],
            process_group_count: 3,
            input_port_count: 1,
            output_port_count: 2,
            processors: crate::client::ProcessorStateCounts {
                running: 5,
                stopped: 1,
                invalid: 0,
                disabled: 0,
            },
            process_group_ids: vec![],
            nodes: vec![],
        };
        let ep = store.apply_update(ClusterUpdate::RootPgStatus(Ok(fake_pg.clone()), meta()));
        assert_eq!(ep, ClusterEndpoint::RootPgStatus);
        match &store.snapshot.root_pg_status {
            EndpointState::Ready { data, .. } => {
                assert_eq!(data.flow_files_queued, 42);
                assert_eq!(data.process_group_count, 3);
                assert_eq!(data.processors.running, 5);
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn system_diagnostics_update_is_applied() {
        use crate::client::overview::{SystemDiagAggregate, SystemDiagSnapshot};
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let fake = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: Vec::new(),
                flowfile_repo: None,
                provenance_repos: Vec::new(),
            },
            nodes: Vec::new(),
            fetched_at: Instant::now(),
        };
        let ep = store.apply_update(ClusterUpdate::SystemDiagnostics(Ok(fake), meta()));
        assert_eq!(ep, ClusterEndpoint::SystemDiagnostics);
        assert!(matches!(
            store.snapshot.system_diagnostics,
            EndpointState::Ready { .. }
        ));
    }

    #[test]
    fn about_update_is_applied() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let fake = crate::client::AboutSnapshot {
            version: "2.9.0".into(),
            title: "NiFi".into(),
        };
        let ep = store.apply_update(ClusterUpdate::About(Ok(fake), meta()));
        assert_eq!(ep, ClusterEndpoint::About);
        match &store.snapshot.about {
            EndpointState::Ready { data, .. } => assert_eq!(data.version, "2.9.0"),
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[tokio::test(flavor = "current_thread", start_paused = true)]
    async fn subscribe_wakes_waiter_on_first_subscriber() {
        let local = tokio::task::LocalSet::new();
        local
            .run_until(async {
                let mut store = ClusterStore::new(
                    ClusterPollingConfig::default(),
                    5000,
                    "https://nifi.test:8443".into(),
                );
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
                let store = ClusterStore::new(
                    ClusterPollingConfig::default(),
                    5000,
                    "https://nifi.test:8443".into(),
                );
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
                let mut store = ClusterStore::new(
                    ClusterPollingConfig::default(),
                    5000,
                    "https://nifi.test:8443".into(),
                );
                // Pretend we spawned something.
                store.handles.push(tokio::task::spawn_local(async {}));
                store.shutdown();
                assert!(store.handles.is_empty());
                // Drop would also call shutdown — must be idempotent.
                store.shutdown();
            })
            .await;
    }

    #[test]
    fn connections_update_is_applied() {
        use crate::client::{ConnectionEndpointIds, ConnectionEndpoints};
        use std::collections::HashMap;

        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let mut by_connection = HashMap::new();
        by_connection.insert(
            "conn-1".to_string(),
            ConnectionEndpointIds {
                source_id: "src-1".into(),
                destination_id: "dst-1".into(),
            },
        );
        let endpoints = ConnectionEndpoints { by_connection };
        let ep = store.apply_update(ClusterUpdate::Connections {
            pg_id: "pg-abc".into(),
            result: Ok(endpoints),
            meta: meta(),
        });
        assert_eq!(ep, ClusterEndpoint::ConnectionsByPg);
        let entry = store
            .snapshot
            .connections_by_pg
            .get("pg-abc")
            .expect("pg-abc entry must exist after apply");
        match entry {
            EndpointState::Ready { data, .. } => {
                let pair = data
                    .by_connection
                    .get("conn-1")
                    .expect("conn-1 must be populated");
                assert_eq!(pair.source_id, "src-1");
                assert_eq!(pair.destination_id, "dst-1");
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }

    #[test]
    fn publish_pg_ids_mirrors_snapshot_into_watch_channel() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        // Before any RootPgStatus update the watch channel is empty.
        assert!(store.pg_ids_rx.borrow().is_empty());

        // Seed a Ready RootPgStatus snapshot by routing an Ok update.
        let fake_pg = RootPgStatusSnapshot {
            flow_files_queued: 0,
            bytes_queued: 0,
            connections: vec![],
            process_group_count: 2,
            input_port_count: 0,
            output_port_count: 0,
            processors: crate::client::ProcessorStateCounts::default(),
            process_group_ids: vec!["root".into(), "child".into()],
            nodes: vec![],
        };
        store.apply_update(ClusterUpdate::RootPgStatus(Ok(fake_pg), meta()));

        // Publish and observe the watch-channel state.
        store.publish_pg_ids();
        let published: Vec<String> = store.pg_ids_rx.borrow().clone();
        assert_eq!(
            published,
            vec!["root".to_string(), "child".to_string()],
            "publish_pg_ids must mirror the Ready snapshot's id list"
        );
    }

    fn fake_bulletin(id: i64) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: "INFO".into(),
            message: format!("msg-{id}"),
            source_id: format!("src-{id}"),
            source_name: format!("Proc-{id}"),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-14T00:00:00Z".into(),
            timestamp_human: String::new(),
        }
    }

    #[test]
    fn bulletins_delta_ok_merges_into_ring() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            10,
            "https://nifi.test:8443".into(),
        );
        // First batch: 3 bulletins.
        let batch1 = vec![fake_bulletin(1), fake_bulletin(2), fake_bulletin(3)];
        let ep = store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Ok(batch1),
            meta: meta(),
        });
        assert_eq!(ep, ClusterEndpoint::Bulletins);
        assert_eq!(store.snapshot.bulletins.buf.len(), 3);
        assert_eq!(store.snapshot.bulletins.last_id, Some(3));
        assert!(store.snapshot.bulletins.meta.is_some());
        assert!(store.snapshot.bulletins.last_error.is_none());

        // Second batch: 2 more bulletins.
        let batch2 = vec![fake_bulletin(4), fake_bulletin(5)];
        store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Ok(batch2),
            meta: meta(),
        });
        assert_eq!(store.snapshot.bulletins.buf.len(), 5);
        assert_eq!(store.snapshot.bulletins.last_id, Some(5));
    }

    #[test]
    fn bulletins_ring_drops_oldest_at_capacity() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            3,
            "https://nifi.test:8443".into(),
        );
        let batch = vec![
            fake_bulletin(1),
            fake_bulletin(2),
            fake_bulletin(3),
            fake_bulletin(4),
            fake_bulletin(5),
        ];
        store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Ok(batch),
            meta: meta(),
        });
        assert_eq!(store.snapshot.bulletins.buf.len(), 3);
        // Oldest three dropped: only 3, 4, 5 remain.
        assert_eq!(store.snapshot.bulletins.buf.front().unwrap().id, 3);
        assert_eq!(store.snapshot.bulletins.buf.back().unwrap().id, 5);
        assert_eq!(store.snapshot.bulletins.last_id, Some(5));
    }

    #[test]
    fn cluster_nodes_update_is_applied() {
        use crate::client::overview::ClusterNodesSnapshot;
        let fake = ClusterNodesSnapshot {
            rows: vec![],
            fetched_at: std::time::Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let meta = FetchMeta {
            fetched_at: std::time::Instant::now(),
            fetch_duration: Duration::from_millis(1),
            next_interval: Duration::from_secs(5),
        };
        let ep = store.apply_update(ClusterUpdate::ClusterNodes(Ok(fake), meta));
        assert_eq!(ep, ClusterEndpoint::ClusterNodes);
        assert!(matches!(
            store.snapshot.cluster_nodes,
            crate::cluster::snapshot::EndpointState::Ready { .. }
        ));
    }

    #[test]
    fn cluster_update_tls_certs_reports_endpoint() {
        use crate::client::tls_cert::TlsCertsSnapshot;
        use std::collections::HashMap;
        let snap = TlsCertsSnapshot {
            certs: HashMap::new(),
            fetched_at: Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        let u = ClusterUpdate::TlsCerts(Ok(snap), meta());
        assert_eq!(u.endpoint(), ClusterEndpoint::TlsCerts);
    }

    #[test]
    fn bulletins_delta_err_preserves_ring_sets_last_error() {
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            10,
            "https://nifi.test:8443".into(),
        );
        // Seed with a successful batch first.
        store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Ok(vec![fake_bulletin(1), fake_bulletin(2)]),
            meta: meta(),
        });
        assert_eq!(store.snapshot.bulletins.buf.len(), 2);
        // Now a failing batch.
        store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Err(NifiLensError::WritesNotImplemented),
            meta: meta(),
        });
        assert_eq!(
            store.snapshot.bulletins.buf.len(),
            2,
            "ring must retain prior entries on failure"
        );
        assert_eq!(store.snapshot.bulletins.last_id, Some(2));
        assert!(store.snapshot.bulletins.last_error.is_some());
        // A subsequent successful batch clears last_error.
        store.apply_update(ClusterUpdate::BulletinsDelta {
            result: Ok(vec![fake_bulletin(3)]),
            meta: meta(),
        });
        assert!(store.snapshot.bulletins.last_error.is_none());
        assert_eq!(store.snapshot.bulletins.last_id, Some(3));
    }

    #[test]
    fn publish_node_addresses_sends_latest() {
        use crate::client::overview::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let snap = ClusterNodesSnapshot {
            rows: vec![ClusterNodeRow {
                node_id: "id1".into(),
                address: "node1.nifi:8443".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            }],
            fetched_at: Instant::now(),
            fetched_wall: time::OffsetDateTime::now_utc(),
        };
        store.apply_update(ClusterUpdate::ClusterNodes(Ok(snap), meta()));
        store.publish_node_addresses();
        let mut rx = store.node_addresses_receiver();
        let addrs = rx.borrow_and_update().clone();
        assert_eq!(addrs, vec!["node1.nifi:8443".to_string()]);
    }

    #[test]
    fn publish_node_addresses_falls_back_to_sysdiag_when_cluster_nodes_empty() {
        use crate::client::overview::{NodeDiagnostics, SystemDiagAggregate, SystemDiagSnapshot};
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let sysdiag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate::default(),
            nodes: vec![NodeDiagnostics {
                address: "nifi-2-6-0:8443".into(),
                heap_used_bytes: 0,
                heap_max_bytes: 0,
                gc: vec![],
                load_average: None,
                available_processors: None,
                total_threads: 0,
                uptime: String::new(),
                content_repos: vec![],
                flowfile_repo: None,
                provenance_repos: vec![],
            }],
            fetched_at: Instant::now(),
        };
        store.apply_update(ClusterUpdate::SystemDiagnostics(Ok(sysdiag), meta()));
        store.publish_node_addresses();
        let mut rx = store.node_addresses_receiver();
        let addrs = rx.borrow_and_update().clone();
        assert_eq!(addrs, vec!["nifi-2-6-0:8443".to_string()]);
    }

    #[test]
    fn version_control_update_is_applied() {
        use crate::cluster::snapshot::{EndpointState, VersionControlMap, VersionControlSummary};
        use nifi_rust_client::dynamic::types::VersionControlInformationDtoState;
        let mut store = ClusterStore::new(
            ClusterPollingConfig::default(),
            5000,
            "https://nifi.test:8443".into(),
        );
        let mut map = VersionControlMap::default();
        map.by_pg_id.insert(
            "pg-1".to_string(),
            VersionControlSummary {
                state: VersionControlInformationDtoState::LocallyModified,
                registry_name: None,
                bucket_name: None,
                branch: None,
                flow_id: None,
                flow_name: None,
                version: None,
                state_explanation: None,
            },
        );
        let ep = store.apply_update(ClusterUpdate::VersionControl(Ok(map.clone()), meta()));
        assert_eq!(ep, ClusterEndpoint::VersionControl);
        match &store.snapshot.version_control {
            EndpointState::Ready { data, .. } => {
                assert_eq!(data.by_pg_id.len(), 1);
                assert!(data.by_pg_id.contains_key("pg-1"));
            }
            other => panic!("expected Ready, got {:?}", other),
        }
    }
}
