//! Cluster-wide snapshot owned by `ClusterStore` and mutated only on
//! the UI task. Each endpoint field preserves the last successful value
//! even when the latest fetch failed, so views render with a staleness
//! chip rather than blanking out.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::client::health::{ClusterNodesSnapshot, SystemDiagSnapshot};
use crate::client::tls_cert::TlsCertsSnapshot;
use crate::client::{
    AboutSnapshot, BulletinSnapshot, ConnectionEndpoints, ControllerServicesSnapshot,
    ControllerStatusSnapshot, RootPgStatusSnapshot,
};
use crate::cluster::ClusterEndpoint;
use crate::error::NifiLensError;

/// Metadata about a single fetch — timing, measured duration, and the
/// adaptive-cadence result that informs the next tick. `next_interval`
/// is filled in by Task 9 (adaptive cadence); Task 1 sets it equal to
/// the base interval.
#[derive(Debug, Clone, Copy)]
pub struct FetchMeta {
    pub fetched_at: Instant,
    pub fetch_duration: Duration,
    pub next_interval: Duration,
}

/// Per-endpoint state. `Loading` is the pre-first-fetch placeholder.
/// `Ready` carries the latest successful value. `Failed` carries the
/// error plus (if present) the last successful value and its meta — the
/// renderer uses `last_ok` to continue drawing stale data with a chip.
#[derive(Debug, Clone, Default)]
pub enum EndpointState<T> {
    #[default]
    Loading,
    Ready {
        data: T,
        meta: FetchMeta,
    },
    Failed {
        error: String,
        meta: FetchMeta,
        last_ok: Option<(T, FetchMeta)>,
    },
}

impl<T: Clone> EndpointState<T> {
    /// Returns the latest data regardless of whether the most recent
    /// fetch succeeded. Returns `None` only when the endpoint has never
    /// returned successfully.
    pub fn latest(&self) -> Option<&T> {
        match self {
            Self::Loading => None,
            Self::Ready { data, .. } => Some(data),
            Self::Failed {
                last_ok: Some((data, _)),
                ..
            } => Some(data),
            Self::Failed { last_ok: None, .. } => None,
        }
    }

    /// Apply a fresh fetch result, preserving `last_ok` on failure.
    pub fn apply(&mut self, result: Result<T, NifiLensError>, meta: FetchMeta) {
        match result {
            Ok(data) => *self = Self::Ready { data, meta },
            Err(err) => {
                let last_ok = match std::mem::replace(self, Self::Loading) {
                    Self::Ready {
                        data,
                        meta: last_meta,
                    } => Some((data, last_meta)),
                    Self::Failed { last_ok, .. } => last_ok,
                    Self::Loading => None,
                };
                *self = Self::Failed {
                    error: err.to_string(),
                    meta,
                    last_ok,
                };
            }
        }
    }
}

/// Rolling append-only ring of bulletins. Unlike the other cluster
/// endpoints — which use `EndpointState<T>` — bulletins merge each
/// successful fetch into a capacity-bounded `VecDeque` so the Overview
/// sparkline and the Bulletins tab can see history beyond the latest
/// batch. The cursor (`last_id`) is owned here so the fetcher task
/// resumes correctly across restarts / context switches.
#[derive(Debug, Default, Clone)]
pub struct BulletinRing {
    /// Bulletins in monotonic arrival order (front = oldest).
    pub buf: VecDeque<BulletinSnapshot>,
    /// Upper bound on `buf.len()`. Sourced from
    /// `config.bulletins.ring_size` at store construction.
    pub capacity: usize,
    /// Maximum bulletin id observed so far. `None` until the first
    /// non-empty batch lands. The fetcher uses this as the `after_id`
    /// cursor for its next call.
    pub last_id: Option<i64>,
    /// Metadata from the most recent fetch (success or failure).
    /// `None` until the first fetch completes.
    pub meta: Option<FetchMeta>,
    /// Human-readable error string from the most recent *failing*
    /// fetch. Cleared on the next successful fetch.
    pub last_error: Option<String>,
}

impl BulletinRing {
    pub fn new(capacity: usize) -> Self {
        Self {
            buf: VecDeque::new(),
            capacity,
            last_id: None,
            meta: None,
            last_error: None,
        }
    }

    /// Merge one fetch batch into the ring. Advances `last_id` to the
    /// maximum id seen, appends new bulletins, and trims from the front
    /// when over `capacity`.
    pub fn merge(&mut self, batch: Vec<BulletinSnapshot>) {
        for b in batch {
            if Some(b.id) > self.last_id {
                self.last_id = Some(b.id);
            }
            self.buf.push_back(b);
            while self.buf.len() > self.capacity {
                self.buf.pop_front();
            }
        }
    }
}

/// The cluster snapshot. Holds the raw client-level snapshot types
/// (already normalized by `NifiClient`) — views project from these.
#[derive(Debug, Default, Clone)]
pub struct ClusterSnapshot {
    pub about: EndpointState<AboutSnapshot>,
    pub controller_status: EndpointState<ControllerStatusSnapshot>,
    pub root_pg_status: EndpointState<RootPgStatusSnapshot>,
    pub controller_services: EndpointState<ControllerServicesSnapshot>,
    pub cluster_nodes: EndpointState<ClusterNodesSnapshot>,
    pub tls_certs: EndpointState<TlsCertsSnapshot>,
    pub system_diagnostics: EndpointState<SystemDiagSnapshot>,
    pub connections_by_pg: HashMap<String, EndpointState<ConnectionEndpoints>>,
    pub bulletins: BulletinRing,
}

impl ClusterSnapshot {
    /// Construct a fresh snapshot whose `BulletinRing` is sized for the
    /// configured `bulletins.ring_size`. Every other endpoint remains
    /// at its default (`Loading`).
    pub fn with_bulletins_capacity(bulletins_capacity: usize) -> Self {
        Self {
            bulletins: BulletinRing::new(bulletins_capacity),
            ..Self::default()
        }
    }

    /// Returns the `next_interval` from the latest `FetchMeta` for
    /// `endpoint`, or `None` if the endpoint has never been polled.
    /// Used by the F12 debug dump to surface the adaptive-cadence state.
    ///
    /// For the fan-out `ConnectionsByPg` endpoint this returns the
    /// maximum `next_interval` across all per-PG entries — useful as a
    /// worst-case indicator at a glance.
    pub fn next_interval_for(&self, endpoint: ClusterEndpoint) -> Option<Duration> {
        fn meta_of<T>(state: &EndpointState<T>) -> Option<&FetchMeta> {
            match state {
                EndpointState::Ready { meta, .. } | EndpointState::Failed { meta, .. } => {
                    Some(meta)
                }
                EndpointState::Loading => None,
            }
        }
        match endpoint {
            ClusterEndpoint::About => meta_of(&self.about).map(|m| m.next_interval),
            ClusterEndpoint::ControllerStatus => {
                meta_of(&self.controller_status).map(|m| m.next_interval)
            }
            ClusterEndpoint::RootPgStatus => meta_of(&self.root_pg_status).map(|m| m.next_interval),
            ClusterEndpoint::ControllerServices => {
                meta_of(&self.controller_services).map(|m| m.next_interval)
            }
            ClusterEndpoint::SystemDiagnostics => {
                meta_of(&self.system_diagnostics).map(|m| m.next_interval)
            }
            ClusterEndpoint::ConnectionsByPg => self
                .connections_by_pg
                .values()
                .filter_map(meta_of)
                .map(|m| m.next_interval)
                .max(),
            ClusterEndpoint::Bulletins => self.bulletins.meta.map(|m| m.next_interval),
            ClusterEndpoint::ClusterNodes => meta_of(&self.cluster_nodes).map(|m| m.next_interval),
            ClusterEndpoint::TlsCerts => meta_of(&self.tls_certs).map(|m| m.next_interval),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_meta() -> FetchMeta {
        FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(50),
            next_interval: Duration::from_secs(10),
        }
    }

    #[test]
    fn loading_apply_ok_becomes_ready() {
        let mut state: EndpointState<u32> = EndpointState::Loading;
        state.apply(Ok(42), fake_meta());
        assert!(matches!(state, EndpointState::Ready { data: 42, .. }));
        assert_eq!(state.latest(), Some(&42));
    }

    #[test]
    fn ready_apply_err_preserves_last_ok() {
        let mut state: EndpointState<u32> = EndpointState::Ready {
            data: 42,
            meta: fake_meta(),
        };
        state.apply(Err(NifiLensError::WritesNotImplemented), fake_meta());
        match &state {
            EndpointState::Failed {
                last_ok: Some((data, _)),
                ..
            } => assert_eq!(*data, 42),
            other => panic!("expected Failed with last_ok, got {:?}", other),
        }
        assert_eq!(state.latest(), Some(&42));
    }

    #[test]
    fn failed_apply_ok_clears_last_ok() {
        let mut state: EndpointState<u32> = EndpointState::Failed {
            error: "boom".into(),
            meta: fake_meta(),
            last_ok: Some((1, fake_meta())),
        };
        state.apply(Ok(99), fake_meta());
        assert!(matches!(state, EndpointState::Ready { data: 99, .. }));
        assert_eq!(state.latest(), Some(&99));
    }

    #[test]
    fn next_interval_for_returns_none_on_loading() {
        let snap = ClusterSnapshot::default();
        assert!(
            snap.next_interval_for(ClusterEndpoint::RootPgStatus)
                .is_none()
        );
    }

    #[test]
    fn next_interval_for_returns_meta_interval_on_ready() {
        let mut snap = ClusterSnapshot::default();
        let meta = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(10),
            next_interval: Duration::from_secs(20),
        };
        snap.controller_status
            .apply(Ok(ControllerStatusSnapshot::default()), meta);
        assert_eq!(
            snap.next_interval_for(ClusterEndpoint::ControllerStatus),
            Some(Duration::from_secs(20)),
        );
    }

    #[test]
    fn next_interval_for_preserves_meta_on_failed() {
        let mut snap = ClusterSnapshot::default();
        let meta = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(15),
        };
        snap.controller_status
            .apply(Err(NifiLensError::WritesNotImplemented), meta);
        assert_eq!(
            snap.next_interval_for(ClusterEndpoint::ControllerStatus),
            Some(Duration::from_secs(15)),
        );
    }

    #[test]
    fn next_interval_for_cluster_nodes() {
        use crate::client::health::ClusterNodesSnapshot;
        let mut snap = ClusterSnapshot::default();
        let meta = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(5),
        };
        snap.cluster_nodes.apply(
            Ok(ClusterNodesSnapshot {
                rows: vec![],
                fetched_at: Instant::now(),
                fetched_wall: time::OffsetDateTime::now_utc(),
            }),
            meta,
        );
        assert_eq!(
            snap.next_interval_for(ClusterEndpoint::ClusterNodes),
            Some(Duration::from_secs(5)),
        );
    }

    #[test]
    fn next_interval_for_tls_certs() {
        use crate::client::tls_cert::TlsCertsSnapshot;
        use std::collections::HashMap;
        let mut snap = ClusterSnapshot::default();
        let meta = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(30),
            next_interval: Duration::from_secs(3600),
        };
        snap.tls_certs.apply(
            Ok(TlsCertsSnapshot {
                certs: HashMap::new(),
                fetched_at: Instant::now(),
                fetched_wall: time::OffsetDateTime::now_utc(),
            }),
            meta,
        );
        assert_eq!(
            snap.next_interval_for(ClusterEndpoint::TlsCerts),
            Some(Duration::from_secs(3600)),
        );
    }

    #[test]
    fn next_interval_for_connections_returns_max() {
        let mut snap = ClusterSnapshot::default();
        let meta_a = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(10),
        };
        let meta_b = FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(30),
        };
        snap.connections_by_pg
            .entry("pg_a".into())
            .or_default()
            .apply(Ok(ConnectionEndpoints::default()), meta_a);
        snap.connections_by_pg
            .entry("pg_b".into())
            .or_default()
            .apply(Ok(ConnectionEndpoints::default()), meta_b);
        assert_eq!(
            snap.next_interval_for(ClusterEndpoint::ConnectionsByPg),
            Some(Duration::from_secs(30)),
        );
    }
}
