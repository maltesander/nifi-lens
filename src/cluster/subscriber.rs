//! Tracks which views are currently subscribed to each cluster
//! endpoint. Writers live on the UI task; readers are the per-endpoint
//! fetch tasks. Cross-task handshake is an `Arc<AtomicUsize>` per
//! endpoint — lock-free, and preserves the "state mutated only on the
//! UI task" invariant.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use crate::app::state::ViewId;
use crate::cluster::ClusterEndpoint;

/// Subscriber identity. v0.1 needs only `ViewId`; the newtype leaves
/// room for future subscribers without churning the public surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SubscriberId(pub ViewId);

#[derive(Debug, Default)]
pub struct SubscriberRegistry {
    /// Canonical per-endpoint subscriber sets, owned by the UI task.
    /// Used for diagnostics (F12 dump).
    sets: HashMap<ClusterEndpoint, HashSet<SubscriberId>>,
    /// Lock-free counters read by fetch tasks. Indexed by
    /// `ClusterEndpoint` discriminant.
    counters: [Arc<AtomicUsize>; ClusterEndpoint::COUNT],
}

impl SubscriberRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns an `Arc<AtomicUsize>` that the fetch task for `endpoint`
    /// can read via `load(Ordering::Relaxed)`. Clone-cheap.
    pub fn counter(&self, endpoint: ClusterEndpoint) -> Arc<AtomicUsize> {
        self.counters[endpoint as usize].clone()
    }

    /// Register `sub` as a subscriber of `endpoint`. Idempotent.
    /// Returns `true` if this call added a new subscriber (0 → 1 transition).
    pub fn subscribe(&mut self, endpoint: ClusterEndpoint, sub: SubscriberId) -> bool {
        let inserted = self.sets.entry(endpoint).or_default().insert(sub);
        if inserted {
            self.counters[endpoint as usize].fetch_add(1, Ordering::Relaxed);
        }
        inserted
    }

    /// Unsubscribe `sub` from `endpoint`. Idempotent. Returns `true` if
    /// this call was the last subscriber (N → 0 transition).
    pub fn unsubscribe(&mut self, endpoint: ClusterEndpoint, sub: SubscriberId) -> bool {
        let removed = self
            .sets
            .get_mut(&endpoint)
            .map(|set| set.remove(&sub))
            .unwrap_or(false);
        if removed {
            let prev = self.counters[endpoint as usize].fetch_sub(1, Ordering::Relaxed);
            return prev == 1;
        }
        false
    }

    /// Current subscriber count for an endpoint (UI-task view).
    pub fn count(&self, endpoint: ClusterEndpoint) -> usize {
        self.counters[endpoint as usize].load(Ordering::Relaxed)
    }

    /// Ordered dump for the `F12` debug keymap dump.
    pub fn debug_snapshot(&self) -> Vec<(ClusterEndpoint, Vec<SubscriberId>)> {
        let mut out = Vec::with_capacity(self.sets.len());
        for (ep, set) in &self.sets {
            let mut subs: Vec<_> = set.iter().copied().collect();
            subs.sort_by_key(|s| s.0 as u8);
            out.push((*ep, subs));
        }
        out.sort_by_key(|(ep, _)| *ep as u8);
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sub(view: ViewId) -> SubscriberId {
        SubscriberId(view)
    }

    #[test]
    fn subscribe_increments_counter() {
        let mut reg = SubscriberRegistry::new();
        let counter = reg.counter(ClusterEndpoint::RootPgStatus);
        assert_eq!(counter.load(Ordering::Relaxed), 0);

        assert!(reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview)));
        assert_eq!(counter.load(Ordering::Relaxed), 1);

        assert!(reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Browser)));
        assert_eq!(counter.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn repeat_subscribe_is_idempotent() {
        let mut reg = SubscriberRegistry::new();
        reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview));
        assert!(!reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview)));
        assert_eq!(reg.count(ClusterEndpoint::RootPgStatus), 1);
    }

    #[test]
    fn unsubscribe_returns_true_on_last() {
        let mut reg = SubscriberRegistry::new();
        reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview));
        reg.subscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Browser));
        assert!(!reg.unsubscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview)));
        assert!(reg.unsubscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Browser)));
        assert_eq!(reg.count(ClusterEndpoint::RootPgStatus), 0);
    }

    #[test]
    fn unsubscribe_unknown_is_noop() {
        let mut reg = SubscriberRegistry::new();
        assert!(!reg.unsubscribe(ClusterEndpoint::RootPgStatus, sub(ViewId::Overview)));
        assert_eq!(reg.count(ClusterEndpoint::RootPgStatus), 0);
    }
}
