//! State for the Browser tab's sparkline strip. Selection-scoped:
//! re-created whenever the user moves to a different
//! processor / PG / connection row. Populated asynchronously by the
//! per-selection worker (`spawn_sparkline_fetch_loop`) via
//! `AppEvent::SparklineUpdate` / `SparklineEndpointMissing`.

use std::time::Instant;

use crate::client::history::{ComponentKind, StatusHistorySeries};

#[derive(Debug, Clone)]
pub struct SparklineState {
    /// Component this sparkline tracks. The reducer compares
    /// `AppEvent::SparklineUpdate.kind` against this to drop stale
    /// emits.
    pub kind: ComponentKind,
    /// Component id (UUID) — same purpose as `kind` for the stale-emit
    /// guard.
    pub id: String,
    /// Most-recent series; `None` until the first fetch lands.
    pub series: Option<StatusHistorySeries>,
    /// Set after a 404 from NiFi (no `/status/history` for this
    /// component). Renderer shows "no history yet". Sticky until the
    /// selection changes.
    pub endpoint_missing: bool,
    /// When the last successful update landed.
    pub last_fetched_at: Option<Instant>,
}

impl SparklineState {
    /// Construct a fresh pending state for a newly-selected component.
    pub fn pending(kind: ComponentKind, id: String) -> Self {
        Self {
            kind,
            id,
            series: None,
            endpoint_missing: false,
            last_fetched_at: None,
        }
    }

    /// Apply a successful series replace. Stale `(kind, id)` mismatches
    /// return early without modifying state — caller should also guard,
    /// but this is defense-in-depth for the brief window between worker
    /// abort and exit.
    pub fn apply_update(&mut self, kind: ComponentKind, id: &str, series: StatusHistorySeries) {
        if self.kind != kind || self.id != id {
            return;
        }
        self.series = Some(series);
        self.endpoint_missing = false;
        self.last_fetched_at = Some(Instant::now());
    }

    /// Apply a 404 / endpoint-missing emit.
    pub fn apply_endpoint_missing(&mut self, kind: ComponentKind, id: &str) {
        if self.kind != kind || self.id != id {
            return;
        }
        self.endpoint_missing = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    fn series_with_buckets(n: usize) -> StatusHistorySeries {
        StatusHistorySeries {
            buckets: (0..n)
                .map(|i| crate::client::history::Bucket {
                    timestamp: SystemTime::now(),
                    in_count: i as u64,
                    out_count: i as u64,
                    queued_count: None,
                    task_time_ns: Some(i as u64),
                })
                .collect(),
            generated_at: SystemTime::now(),
        }
    }

    #[test]
    fn pending_initial_state_has_no_series() {
        let s = SparklineState::pending(ComponentKind::Processor, "p-1".into());
        assert!(s.series.is_none());
        assert!(!s.endpoint_missing);
        assert!(s.last_fetched_at.is_none());
    }

    #[test]
    fn apply_update_replaces_series() {
        let mut s = SparklineState::pending(ComponentKind::Processor, "p-1".into());
        s.apply_update(ComponentKind::Processor, "p-1", series_with_buckets(3));
        assert_eq!(s.series.as_ref().unwrap().buckets.len(), 3);
        assert!(s.last_fetched_at.is_some());
    }

    #[test]
    fn apply_update_clears_endpoint_missing() {
        let mut s = SparklineState::pending(ComponentKind::Processor, "p-1".into());
        s.apply_endpoint_missing(ComponentKind::Processor, "p-1");
        assert!(s.endpoint_missing);
        s.apply_update(ComponentKind::Processor, "p-1", series_with_buckets(1));
        assert!(
            !s.endpoint_missing,
            "successful update must clear sticky 404"
        );
    }

    #[test]
    fn apply_update_drops_stale_kind_or_id() {
        let mut s = SparklineState::pending(ComponentKind::Processor, "p-1".into());
        s.apply_update(ComponentKind::ProcessGroup, "p-1", series_with_buckets(2));
        assert!(s.series.is_none(), "kind mismatch must drop");
        s.apply_update(ComponentKind::Processor, "OTHER", series_with_buckets(2));
        assert!(s.series.is_none(), "id mismatch must drop");
    }

    #[test]
    fn apply_endpoint_missing_drops_stale_kind_or_id() {
        let mut s = SparklineState::pending(ComponentKind::Processor, "p-1".into());
        s.apply_endpoint_missing(ComponentKind::Connection, "p-1");
        assert!(!s.endpoint_missing, "kind mismatch must drop");
        s.apply_endpoint_missing(ComponentKind::Processor, "OTHER");
        assert!(!s.endpoint_missing, "id mismatch must drop");
    }
}
