//! Pure state for the Overview tab plus the `apply_payload` reducer.
//!
//! Everything here is synchronous and `no_run` safe — the tokio worker in
//! `super::worker` is the only place that touches the network.

use std::time::SystemTime;

use crate::client::{AboutSnapshot, ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot};
use crate::event::OverviewPayload;

pub use crate::client::Severity;

/// Size of the rolling bulletin-rate sparkline (minutes).
pub const SPARKLINE_MINUTES: usize = 15;
/// How many unhealthy queues to keep in the leaderboard.
pub const TOP_QUEUES: usize = 10;
/// How many noisy components to keep in the leaderboard.
pub const TOP_NOISY: usize = 5;

/// One-minute bulletin-rate bucket.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BulletinBucket {
    pub count: u32,
    pub max_severity: Severity,
}

#[derive(Debug, Clone)]
pub struct UnhealthyQueue {
    pub id: String,
    pub group_id: String,
    pub name: String,
    pub source_name: String,
    pub destination_name: String,
    pub fill_percent: u32,
    pub flow_files_queued: u32,
    pub bytes_queued: u64,
    pub queued_display: String,
}

impl From<QueueSnapshot> for UnhealthyQueue {
    fn from(q: QueueSnapshot) -> Self {
        Self {
            id: q.id,
            group_id: q.group_id,
            name: q.name,
            source_name: q.source_name,
            destination_name: q.destination_name,
            fill_percent: q.fill_percent,
            flow_files_queued: q.flow_files_queued,
            bytes_queued: q.bytes_queued,
            queued_display: q.queued_display,
        }
    }
}

/// One "noisy component" leaderboard row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NoisyComponent {
    pub source_id: String,
    pub source_name: String,
    pub group_id: String,
    pub count: u32,
    pub max_severity: Severity,
}

/// Snapshot of the Overview tab at one point in time. `None` until the
/// first poll completes.
#[derive(Debug, Clone, Default)]
pub struct OverviewSnapshot {
    pub about: AboutSnapshot,
    pub controller: ControllerStatusSnapshot,
    pub root_pg: RootPgStatusSnapshot,
    pub fetched_at: Option<SystemTime>,
}

#[derive(Debug, Default)]
pub struct OverviewState {
    pub snapshot: Option<OverviewSnapshot>,
    pub sparkline: [BulletinBucket; SPARKLINE_MINUTES],
    pub unhealthy: Vec<UnhealthyQueue>,
    pub noisy: Vec<NoisyComponent>,
    /// Highest bulletin id we've seen so the sparkline does not double-count
    /// across polls. The reducer keeps this updated whenever a poll arrives.
    pub last_bulletin_id: Option<i64>,
}

impl OverviewState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Fold one poll result into the state. Pure; no I/O.
pub fn apply_payload(state: &mut OverviewState, payload: OverviewPayload) {
    let OverviewPayload {
        about,
        controller,
        root_pg,
        bulletin_board,
        fetched_at,
    } = payload;

    // Unhealthy queues: take the top N (already sorted descending by the
    // client wrapper).
    state.unhealthy = root_pg
        .connections
        .iter()
        .take(TOP_QUEUES)
        .cloned()
        .map(UnhealthyQueue::from)
        .collect();

    // Sparkline: assign each bulletin to a minute bucket relative to
    // fetched_at. sparkline[0] is the OLDEST minute (SPARKLINE_MINUTES-1
    // minutes before fetched_at); sparkline[SPARKLINE_MINUTES-1] is the
    // NEWEST minute (the one containing fetched_at). Bulletins older than
    // the window are discarded for the sparkline but still count toward
    // "noisy".
    let mut sparkline = [BulletinBucket::default(); SPARKLINE_MINUTES];
    let fetched_secs = fetched_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    for b in &bulletin_board.bulletins {
        let Some(ts) = parse_iso_seconds(&b.timestamp_iso) else {
            continue;
        };
        let age_secs = fetched_secs.saturating_sub(ts);
        if age_secs < 0 {
            continue;
        }
        let minute = (age_secs / 60) as usize;
        if minute >= SPARKLINE_MINUTES {
            continue;
        }
        let bucket = &mut sparkline[SPARKLINE_MINUTES - 1 - minute];
        bucket.count = bucket.count.saturating_add(1);
        let sev = Severity::parse(&b.level);
        if sev > bucket.max_severity {
            bucket.max_severity = sev;
        }
    }
    state.sparkline = sparkline;

    // Noisy components: aggregate counts by source_id across *this poll's*
    // bulletins. Phase 1 is snapshot-style — not a running tally — because
    // that keeps the reducer pure and matches the "current noise" reading
    // the spec asks for.
    use std::collections::HashMap;
    let mut by_source: HashMap<String, NoisyComponent> = HashMap::new();
    for b in &bulletin_board.bulletins {
        if b.source_id.is_empty() {
            continue;
        }
        let entry = by_source
            .entry(b.source_id.clone())
            .or_insert_with(|| NoisyComponent {
                source_id: b.source_id.clone(),
                source_name: b.source_name.clone(),
                group_id: b.group_id.clone(),
                ..NoisyComponent::default()
            });
        entry.count = entry.count.saturating_add(1);
        let sev = Severity::parse(&b.level);
        if sev > entry.max_severity {
            entry.max_severity = sev;
        }
    }
    let mut noisy: Vec<NoisyComponent> = by_source.into_values().collect();
    noisy.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| b.max_severity.cmp(&a.max_severity))
            .then_with(|| a.source_name.cmp(&b.source_name))
    });
    noisy.truncate(TOP_NOISY);
    state.noisy = noisy;

    // Advance the bulletin-id cursor (informational — Phase 2 will actually
    // use this for the Bulletins tab's `after-id` paging).
    if let Some(max) = bulletin_board.bulletins.iter().map(|b| b.id).max() {
        state.last_bulletin_id = Some(match state.last_bulletin_id {
            Some(existing) => existing.max(max),
            None => max,
        });
    }

    state.snapshot = Some(OverviewSnapshot {
        about,
        controller,
        root_pg,
        fetched_at: Some(fetched_at),
    });
}

/// Parse an ISO-8601 / RFC-3339 timestamp into seconds since the UNIX epoch.
/// Returns `None` if the input is empty or unparseable — the reducer then
/// silently drops that bulletin from the sparkline.
fn parse_iso_seconds(s: &str) -> Option<i64> {
    if s.is_empty() {
        return None;
    }
    use time::OffsetDateTime;
    use time::format_description::well_known::Rfc3339;
    OffsetDateTime::parse(s, &Rfc3339)
        .ok()
        .map(|dt| dt.unix_timestamp())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::client::{
        AboutSnapshot, BulletinBoardSnapshot, BulletinSnapshot, ControllerStatusSnapshot,
        QueueSnapshot, RootPgStatusSnapshot,
    };
    use crate::event::OverviewPayload;
    use std::time::{Duration, UNIX_EPOCH};

    // 2026-04-11T10:14:22Z in unix seconds.
    const T0: u64 = 1_775_902_462;

    fn payload(
        controller: ControllerStatusSnapshot,
        queues: Vec<QueueSnapshot>,
        bulletins: Vec<BulletinSnapshot>,
    ) -> OverviewPayload {
        OverviewPayload {
            about: AboutSnapshot {
                version: "2.8.0".into(),
                title: "NiFi".into(),
            },
            controller,
            root_pg: RootPgStatusSnapshot {
                flow_files_queued: 0,
                bytes_queued: 0,
                connections: queues,
            },
            bulletin_board: BulletinBoardSnapshot { bulletins },
            fetched_at: UNIX_EPOCH + Duration::from_secs(T0),
        }
    }

    fn q(id: &str, pct: u32) -> QueueSnapshot {
        QueueSnapshot {
            id: id.into(),
            group_id: "root".into(),
            name: format!("{id}-name"),
            source_name: "src".into(),
            destination_name: "dst".into(),
            fill_percent: pct,
            flow_files_queued: pct * 10,
            bytes_queued: 0,
            queued_display: format!("{}%", pct),
        }
    }

    fn bulletin(id: i64, level: &str, source_id: &str, iso: &str) -> BulletinSnapshot {
        BulletinSnapshot {
            id,
            level: level.into(),
            message: "msg".into(),
            source_id: source_id.into(),
            source_name: format!("Proc-{source_id}"),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: iso.into(),
        }
    }

    #[test]
    fn apply_populates_snapshot() {
        let mut state = OverviewState::new();
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], vec![]),
        );
        let snap = state.snapshot.as_ref().unwrap();
        assert_eq!(snap.about.version, "2.8.0");
        assert!(snap.fetched_at.is_some());
    }

    #[test]
    fn unhealthy_queues_truncated_to_top_ten() {
        let mut state = OverviewState::new();
        let queues: Vec<QueueSnapshot> = (0..20)
            .map(|i| q(&format!("c{i}"), 100 - i as u32))
            .collect();
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), queues, vec![]),
        );
        assert_eq!(state.unhealthy.len(), TOP_QUEUES);
        assert_eq!(state.unhealthy[0].fill_percent, 100);
        assert_eq!(state.unhealthy[9].fill_percent, 91);
    }

    #[test]
    fn sparkline_buckets_bulletins_by_minute() {
        let mut state = OverviewState::new();
        // fetched_at = 10:14:22Z. Bulletin at 10:14:10Z is 12s ago → bucket 0
        // (minute 0 from newest). Bulletin at 10:10:00Z is ~262s ago →
        // minute 4 → bucket SPARKLINE_MINUTES-1-4.
        let bulletins = vec![
            bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
            bulletin(2, "ERROR", "b", "2026-04-11T10:10:00Z"),
            bulletin(3, "WARN", "a", "2026-04-11T10:14:20Z"),
        ];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        let newest = state.sparkline[SPARKLINE_MINUTES - 1];
        assert_eq!(
            newest.count, 2,
            "two bulletins within the most recent minute"
        );
        assert_eq!(newest.max_severity, Severity::Warning);
        let four_minutes_old = state.sparkline[SPARKLINE_MINUTES - 1 - 4];
        assert_eq!(four_minutes_old.count, 1);
        assert_eq!(four_minutes_old.max_severity, Severity::Error);
    }

    #[test]
    fn sparkline_pins_absolute_index_orientation() {
        // Pin the absolute index orientation: a bulletin aged 0s goes to
        // index SPARKLINE_MINUTES-1, nothing else. A bulletin aged
        // ~14 minutes lands at index 0 (the oldest visible bucket).
        let mut orientation_state = OverviewState::new();
        let orientation_bulletins = vec![bulletin(
            10,
            "INFO",
            "z",
            "2026-04-11T10:14:22Z", // age 0s, exactly fetched_at
        )];
        apply_payload(
            &mut orientation_state,
            payload(
                ControllerStatusSnapshot::default(),
                vec![],
                orientation_bulletins,
            ),
        );
        assert_eq!(
            orientation_state.sparkline[SPARKLINE_MINUTES - 1].count,
            1,
            "age 0s bulletin must land in the newest bucket (last index)"
        );
        for i in 0..(SPARKLINE_MINUTES - 1) {
            assert_eq!(
                orientation_state.sparkline[i].count, 0,
                "non-newest buckets must be empty"
            );
        }
    }

    #[test]
    fn sparkline_drops_bulletins_outside_window() {
        let mut state = OverviewState::new();
        // 30 minutes old — out of window.
        let bulletins = vec![bulletin(1, "INFO", "a", "2026-04-11T09:44:22Z")];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        assert!(state.sparkline.iter().all(|b| b.count == 0));
    }

    #[test]
    fn noisy_components_ranked_by_count_then_severity() {
        let mut state = OverviewState::new();
        let bulletins = vec![
            bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
            bulletin(2, "INFO", "a", "2026-04-11T10:14:11Z"),
            bulletin(3, "ERROR", "a", "2026-04-11T10:14:12Z"),
            bulletin(4, "INFO", "b", "2026-04-11T10:14:13Z"),
            bulletin(5, "INFO", "b", "2026-04-11T10:14:14Z"),
            bulletin(6, "INFO", "c", "2026-04-11T10:14:15Z"),
        ];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        assert_eq!(state.noisy[0].source_id, "a");
        assert_eq!(state.noisy[0].count, 3);
        assert_eq!(state.noisy[0].max_severity, Severity::Error);
        assert_eq!(state.noisy[1].source_id, "b");
        assert_eq!(state.noisy[1].count, 2);
        assert_eq!(state.noisy[2].source_id, "c");
    }

    #[test]
    fn noisy_leaderboard_truncated_to_top_five() {
        let mut state = OverviewState::new();
        let bulletins: Vec<BulletinSnapshot> = (0..20)
            .map(|i| bulletin(i, "INFO", &format!("s{i}"), "2026-04-11T10:14:10Z"))
            .collect();
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        assert_eq!(state.noisy.len(), TOP_NOISY);
    }

    #[test]
    fn empty_bulletin_timestamp_is_silently_skipped() {
        let mut state = OverviewState::new();
        let bulletins = vec![bulletin(1, "INFO", "a", "")];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        assert!(state.sparkline.iter().all(|b| b.count == 0));
    }

    #[test]
    fn severity_parse_is_case_insensitive() {
        assert_eq!(Severity::parse("error"), Severity::Error);
        assert_eq!(Severity::parse("Warn"), Severity::Warning);
        assert_eq!(Severity::parse("warning"), Severity::Warning);
        assert_eq!(Severity::parse("INFO"), Severity::Info);
        assert_eq!(Severity::parse("debug"), Severity::Unknown);
    }

    #[test]
    fn last_bulletin_id_monotonically_advances() {
        let mut state = OverviewState::new();
        apply_payload(
            &mut state,
            payload(
                ControllerStatusSnapshot::default(),
                vec![],
                vec![bulletin(5, "INFO", "a", "2026-04-11T10:14:10Z")],
            ),
        );
        assert_eq!(state.last_bulletin_id, Some(5));
        apply_payload(
            &mut state,
            payload(
                ControllerStatusSnapshot::default(),
                vec![],
                vec![bulletin(3, "INFO", "a", "2026-04-11T10:14:11Z")],
            ),
        );
        assert_eq!(state.last_bulletin_id, Some(5));
        apply_payload(
            &mut state,
            payload(
                ControllerStatusSnapshot::default(),
                vec![],
                vec![bulletin(9, "INFO", "a", "2026-04-11T10:14:12Z")],
            ),
        );
        assert_eq!(state.last_bulletin_id, Some(9));
    }
}
