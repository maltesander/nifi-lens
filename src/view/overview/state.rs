//! Pure state for the Overview tab plus the `apply_payload` reducer.
//!
//! Everything here is synchronous and `no_run` safe — the tokio worker in
//! `super::worker` is the only place that touches the network.

use std::time::SystemTime;

use crate::client::{AboutSnapshot, ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot};
use crate::event::{OverviewPayload, OverviewPgStatusPayload};

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

/// Which overview panel (if any) currently holds keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OverviewFocus {
    /// No panel focused — default.
    #[default]
    None,
    /// The Nodes panel is focused; row cursor = `OverviewState.nodes.selected`.
    Nodes,
    /// The Noisy components panel is focused; row cursor = `OverviewState.noisy_selected`.
    Noisy,
    /// The Unhealthy queues panel is focused; row cursor = `OverviewState.queues_selected`.
    Queues,
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

    // New in Phase 3 — populated by the SystemDiag payload variant.
    pub nodes: crate::client::health::NodesState,
    pub repositories_summary: RepositoriesSummary,
    pub last_pg_refresh: Option<std::time::Instant>,
    pub last_sysdiag_refresh: Option<std::time::Instant>,

    /// Last observed sysdiag mode. `None` until the first sysdiag
    /// poll resolves. Driven by the app-level reducer in
    /// `src/app/state/mod.rs`, not by `apply_payload`, because the
    /// warn-once banner write is an AppState-level side effect.
    pub sysdiag_mode: Option<SysdiagMode>,

    /// Which panel holds focus. `None` by default.
    pub focus: OverviewFocus,
    /// Selected row index in the Noisy components panel.
    pub noisy_selected: usize,
    /// Selected row index in the Unhealthy queues panel.
    pub queues_selected: usize,
}

/// Cluster-aggregate repository fill bars shown in the Overview "Nodes"
/// zone. Phase 3 displays only the aggregate; per-node breakdown was
/// part of the old Health detail pane and is not in scope for Overview.
#[derive(Debug, Clone, Default)]
pub struct RepositoriesSummary {
    pub content_percent: u32,
    pub flowfile_percent: u32,
    pub provenance_percent: u32,
}

/// Whether the last successful system-diagnostics poll came from the
/// nodewise endpoint or the aggregate-only fallback. Used by the
/// app-level reducer to fire the "nodewise unavailable" warning banner
/// only when the mode *transitions* into `Aggregate`, not on every
/// fallback tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SysdiagMode {
    Nodewise,
    Aggregate,
}

impl OverviewState {
    pub fn new() -> Self {
        Self::default()
    }
}

/// Fold one poll result into the state. Pure; no I/O.
pub fn apply_payload(state: &mut OverviewState, payload: OverviewPayload) {
    match payload {
        OverviewPayload::PgStatus(pg) => apply_pg_status(state, pg),
        OverviewPayload::SystemDiag(diag) => apply_system_diagnostics(state, diag),
        OverviewPayload::SystemDiagFallback { diag, warning: _ } => {
            // The warning is surfaced via a banner by the AppState dispatch
            // in P3.T3. For the reducer, both variants update the same
            // fields the same way.
            apply_system_diagnostics(state, diag);
        }
    }
}

/// Fold one PG-status poll into the existing Overview state.
/// Pre-Phase-3 this was the entire body of `apply_payload`.
fn apply_pg_status(state: &mut OverviewState, payload: OverviewPgStatusPayload) {
    let OverviewPgStatusPayload {
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
    if !state.unhealthy.is_empty() {
        state.queues_selected = state.queues_selected.min(state.unhealthy.len() - 1);
    } else {
        state.queues_selected = 0;
    }

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
    if !state.noisy.is_empty() {
        state.noisy_selected = state.noisy_selected.min(state.noisy.len() - 1);
    } else {
        state.noisy_selected = 0;
    }

    // Advance the bulletin-id cursor (informational — Phase 2 will actually
    // use this for the Bulletins tab's `after-id` paging).
    if let Some(max) = bulletin_board.bulletins.iter().map(|b| b.id).max() {
        state.last_bulletin_id = Some(match state.last_bulletin_id {
            Some(existing) => existing.max(max),
            None => max,
        });
    }

    state.last_pg_refresh = Some(std::time::Instant::now());

    state.snapshot = Some(OverviewSnapshot {
        about,
        controller,
        root_pg,
        fetched_at: Some(fetched_at),
    });
}

fn apply_system_diagnostics(
    state: &mut OverviewState,
    diag: crate::client::health::SystemDiagSnapshot,
) {
    // Build the per-node row set from the diagnostics snapshot.
    crate::client::health::update_nodes(&mut state.nodes, &diag);

    // Build the cluster-aggregate repository summary (NOT per-node).
    state.repositories_summary = build_repositories_summary(&diag);

    state.last_sysdiag_refresh = Some(std::time::Instant::now());
}

/// Compute cluster-aggregate fill percentages from the server-provided
/// aggregate (already summed/averaged across nodes by NiFi).
/// We use the average utilization across the repos within each repo type.
fn build_repositories_summary(
    diag: &crate::client::health::SystemDiagSnapshot,
) -> RepositoriesSummary {
    let agg = &diag.aggregate;

    let content_percent = avg_repo_percent(&agg.content_repos);
    let flowfile_percent = agg
        .flowfile_repo
        .as_ref()
        .map(|r| r.utilization_percent)
        .unwrap_or(0);
    let provenance_percent = avg_repo_percent(&agg.provenance_repos);

    RepositoriesSummary {
        content_percent,
        flowfile_percent,
        provenance_percent,
    }
}

fn avg_repo_percent(repos: &[crate::client::health::RepoUsage]) -> u32 {
    if repos.is_empty() {
        return 0;
    }
    let sum: u32 = repos.iter().map(|r| r.utilization_percent).sum();
    sum / repos.len() as u32
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
    use crate::event::{OverviewPayload, OverviewPgStatusPayload};
    use std::time::{Duration, UNIX_EPOCH};

    // 2026-04-11T10:14:22Z in unix seconds.
    const T0: u64 = 1_775_902_462;

    fn payload(
        controller: ControllerStatusSnapshot,
        queues: Vec<QueueSnapshot>,
        bulletins: Vec<BulletinSnapshot>,
    ) -> OverviewPayload {
        OverviewPayload::PgStatus(OverviewPgStatusPayload {
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
        })
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
            timestamp_human: String::new(),
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

    #[test]
    fn noisy_cursor_clamped_when_data_shrinks() {
        let mut state = OverviewState::new();
        state.noisy_selected = 4;
        // Payload with 2 noisy sources (ids "a" and "b").
        let bulletins = vec![
            bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
            bulletin(2, "INFO", "b", "2026-04-11T10:14:10Z"),
        ];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], bulletins),
        );
        assert_eq!(state.noisy_selected, 1, "cursor clamped to len-1 = 1");
    }

    #[test]
    fn noisy_cursor_reset_to_zero_when_noisy_empty() {
        let mut state = OverviewState::new();
        state.noisy_selected = 3;
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), vec![], vec![]),
        );
        assert_eq!(state.noisy_selected, 0);
    }

    #[test]
    fn queues_cursor_clamped_when_data_shrinks() {
        let mut state = OverviewState::new();
        state.queues_selected = 9;
        let queues = vec![q("c0", 90), q("c1", 80), q("c2", 70)];
        apply_payload(
            &mut state,
            payload(ControllerStatusSnapshot::default(), queues, vec![]),
        );
        assert_eq!(state.queues_selected, 2, "cursor clamped to len-1 = 2");
    }

    #[test]
    fn overview_focus_default_is_none() {
        let state = OverviewState::new();
        assert_eq!(state.focus, OverviewFocus::None);
    }

    #[test]
    fn apply_system_diagnostics_populates_nodes_and_repositories() {
        use crate::client::health::{
            GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
        };
        use std::time::Instant;

        let node = |address: &str| NodeDiagnostics {
            address: address.into(),
            heap_used_bytes: 512 * 1024 * 1024,
            heap_max_bytes: 1024 * 1024 * 1024,
            gc: vec![GcSnapshot {
                name: "G1 Young".into(),
                collection_count: 10,
                collection_millis: 50,
            }],
            load_average: Some(1.5),
            available_processors: Some(4),
            total_threads: 50,
            uptime: "1h".into(),
            content_repos: vec![RepoUsage {
                identifier: "content".into(),
                used_bytes: 60,
                total_bytes: 100,
                free_bytes: 40,
                utilization_percent: 60,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "flowfile".into(),
                used_bytes: 30,
                total_bytes: 100,
                free_bytes: 70,
                utilization_percent: 30,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "provenance".into(),
                used_bytes: 20,
                total_bytes: 100,
                free_bytes: 80,
                utilization_percent: 20,
            }],
        };

        let diag = SystemDiagSnapshot {
            aggregate: SystemDiagAggregate {
                content_repos: vec![RepoUsage {
                    identifier: "content".into(),
                    used_bytes: 60,
                    total_bytes: 100,
                    free_bytes: 40,
                    utilization_percent: 60,
                }],
                flowfile_repo: Some(RepoUsage {
                    identifier: "flowfile".into(),
                    used_bytes: 30,
                    total_bytes: 100,
                    free_bytes: 70,
                    utilization_percent: 30,
                }),
                provenance_repos: vec![RepoUsage {
                    identifier: "provenance".into(),
                    used_bytes: 20,
                    total_bytes: 100,
                    free_bytes: 80,
                    utilization_percent: 20,
                }],
            },
            nodes: vec![node("node1:8080"), node("node2:8080")],
            fetched_at: Instant::now(),
        };

        let mut state = OverviewState::new();
        apply_payload(&mut state, OverviewPayload::SystemDiag(diag));

        // The nodes state should have 2 entries.
        assert_eq!(state.nodes.nodes.len(), 2, "nodes should be populated");

        // Repository aggregates come from the aggregate field.
        assert_eq!(state.repositories_summary.content_percent, 60);
        assert_eq!(state.repositories_summary.flowfile_percent, 30);
        assert_eq!(state.repositories_summary.provenance_percent, 20);
        assert!(state.last_sysdiag_refresh.is_some());
    }
}
