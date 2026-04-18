//! Cluster-wide snapshot owned by `ClusterStore` and mutated only on
//! the UI task. Each endpoint field preserves the last successful value
//! even when the latest fetch failed, so views render with a staleness
//! chip rather than blanking out.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::client::health::SystemDiagSnapshot;
use crate::client::{
    AboutSnapshot, BulletinSnapshot, ConnectionEndpoints, ControllerServiceCounts,
    ControllerStatusSnapshot, RootPgStatusSnapshot,
};
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

/// The cluster snapshot. Holds the raw client-level snapshot types
/// (already normalized by `NifiClient`) — views project from these.
#[derive(Debug, Default, Clone)]
pub struct ClusterSnapshot {
    pub about: EndpointState<AboutSnapshot>,
    pub controller_status: EndpointState<ControllerStatusSnapshot>,
    pub root_pg_status: EndpointState<RootPgStatusSnapshot>,
    pub controller_services: EndpointState<ControllerServiceCounts>,
    pub system_diagnostics: EndpointState<SystemDiagSnapshot>,
    pub connections_by_pg: HashMap<String, EndpointState<ConnectionEndpoints>>,
    pub bulletins: EndpointState<Vec<BulletinSnapshot>>,
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
}
