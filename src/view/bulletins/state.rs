//! Pure state for the Bulletins tab.
//!
//! Everything here is synchronous and no-I/O. The tokio worker in
//! `super::worker` is the only place that touches the network.

use std::collections::VecDeque;
use std::time::SystemTime;

use crate::client::BulletinSnapshot;
use crate::event::BulletinsPayload;

/// Bulletins ring buffer + cursor. `FilterState` and UI-mode fields land
/// in Task 6.
#[derive(Debug)]
pub struct BulletinsState {
    /// Oldest at the front, newest at the back.
    pub ring: VecDeque<BulletinSnapshot>,
    pub ring_capacity: usize,
    /// Monotonic cursor. `None` = "first poll, ask for 1000 most-recent".
    pub last_id: Option<i64>,
    /// `fetched_at` from the most recent payload.
    pub last_fetched_at: Option<SystemTime>,
}

impl BulletinsState {
    pub fn with_capacity(ring_capacity: usize) -> Self {
        Self {
            ring: VecDeque::with_capacity(ring_capacity),
            ring_capacity,
            last_id: None,
            last_fetched_at: None,
        }
    }
}

/// Fold one poll result into the state. Pure; no I/O.
pub fn apply_payload(state: &mut BulletinsState, payload: BulletinsPayload) {
    // Dedup + advance cursor.
    let cursor = state.last_id.unwrap_or(i64::MIN);
    let mut max_seen = cursor;
    for b in payload.bulletins {
        if b.id <= cursor {
            continue;
        }
        if b.id > max_seen {
            max_seen = b.id;
        }
        state.ring.push_back(b);
    }
    while state.ring.len() > state.ring_capacity {
        state.ring.pop_front();
    }
    if max_seen > cursor {
        state.last_id = Some(max_seen);
    }
    state.last_fetched_at = Some(payload.fetched_at);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::BulletinSnapshot;
    use std::time::{Duration, UNIX_EPOCH};

    const T0: u64 = 1_775_902_462; // 2026-04-11T10:14:22Z

    fn b(id: i64, level: &str) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: level.into(),
            message: format!("msg-{id}"),
            source_id: format!("src-{id}"),
            source_name: format!("Proc-{id}"),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
        }
    }

    fn payload(bulletins: Vec<BulletinSnapshot>) -> BulletinsPayload {
        BulletinsPayload {
            bulletins,
            fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
        }
    }

    #[test]
    fn apply_payload_seeds_empty_ring_with_initial_batch() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(
            &mut s,
            payload(vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]),
        );
        assert_eq!(s.ring.len(), 3);
        assert_eq!(s.ring[0].id, 1);
        assert_eq!(s.ring[2].id, 3);
        assert_eq!(s.last_id, Some(3));
        assert!(s.last_fetched_at.is_some());
    }

    #[test]
    fn apply_payload_dedups_on_id() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(1, "INFO"), b(2, "INFO")]));
        apply_payload(&mut s, payload(vec![b(2, "INFO"), b(3, "INFO")]));
        assert_eq!(s.ring.len(), 3);
        assert_eq!(s.last_id, Some(3));
    }

    #[test]
    fn apply_payload_drops_oldest_at_capacity() {
        let mut s = BulletinsState::with_capacity(4);
        apply_payload(
            &mut s,
            payload(vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]),
        );
        apply_payload(
            &mut s,
            payload(vec![b(4, "INFO"), b(5, "INFO"), b(6, "INFO")]),
        );
        assert_eq!(s.ring.len(), 4);
        assert_eq!(s.ring.front().unwrap().id, 3);
        assert_eq!(s.ring.back().unwrap().id, 6);
    }

    #[test]
    fn apply_payload_advances_last_id_monotonically() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![b(10, "INFO")]));
        assert_eq!(s.last_id, Some(10));
        // Stale batch (server reordered or wrapped): cursor stays at 10.
        apply_payload(&mut s, payload(vec![b(5, "INFO")]));
        assert_eq!(s.last_id, Some(10));
        // New bulletins above the cursor: advances.
        apply_payload(&mut s, payload(vec![b(11, "INFO"), b(15, "INFO")]));
        assert_eq!(s.last_id, Some(15));
    }

    #[test]
    fn apply_payload_empty_batch_is_noop_except_for_fetched_at() {
        let mut s = BulletinsState::with_capacity(100);
        apply_payload(&mut s, payload(vec![]));
        assert!(s.ring.is_empty());
        assert_eq!(s.last_id, None);
        assert!(s.last_fetched_at.is_some());
    }
}
