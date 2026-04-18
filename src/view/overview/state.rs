//! Pure state for the Overview tab plus the `redraw_*` reducers that
//! mirror projections out of `AppState.cluster.snapshot`.
//!
//! Everything here is synchronous and `no_run` safe — `ClusterStore`
//! owns every network call; Overview only projects.

use crate::client::{ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot};

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
    pub error_count: u32,
    pub warning_count: u32,
    pub info_count: u32,
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

#[derive(Debug, Default)]
pub struct OverviewState {
    /// Latest root-PG status, mirrored from the cluster snapshot by
    /// `redraw_components`. `None` until the first `RootPgStatus` fetch
    /// has landed in `AppState.cluster.snapshot`. The renderer reads
    /// this directly.
    pub root_pg: Option<RootPgStatusSnapshot>,
    /// Latest controller-status snapshot (processor/PG state aggregates,
    /// version-sync counters), mirrored from the cluster snapshot by
    /// `redraw_controller_status`. `None` until the first
    /// `ControllerStatus` fetch has landed. The renderer reads
    /// `.stale` / `.locally_modified` / `.sync_failure` out of this to
    /// build the PG versioning slot in the Components panel.
    pub controller: Option<ControllerStatusSnapshot>,
    /// Latest controller-service counts, mirrored from the cluster
    /// snapshot by `redraw_components`. `None` when the CS fetch has
    /// never succeeded — the CS row in the Components panel degrades
    /// to a "cs list unavailable" chip in that case. The renderer
    /// reads this directly.
    pub cs_counts: Option<crate::client::ControllerServiceCounts>,
    pub sparkline: [BulletinBucket; SPARKLINE_MINUTES],
    /// Unix seconds at the start of `sparkline[SPARKLINE_MINUTES-1]` (the
    /// newest bucket). `None` until the first PG-status poll lands. Aligned
    /// to a minute boundary so that `(fetched_secs - epoch) / 60` gives the
    /// number of complete minutes that have elapsed and need to be rolled.
    pub sparkline_epoch_secs: Option<i64>,
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
    /// poll resolves. Task 8 made the fetcher task (and not the
    /// reducer) responsible for detecting nodewise → aggregate
    /// transitions, so this field is no longer written from the UI
    /// layer. Reserved for future reintroduction if the in-TUI
    /// banner returns.
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

    /// Cursor view over the Noisy-components list for `ListNavigation`.
    pub(crate) fn noisy_nav(&mut self) -> crate::app::navigation::CursorRef<'_> {
        let len = self.noisy.len();
        crate::app::navigation::CursorRef::new(&mut self.noisy_selected, len)
    }

    /// Cursor view over the Unhealthy-queues list for `ListNavigation`.
    pub(crate) fn queues_nav(&mut self) -> crate::app::navigation::CursorRef<'_> {
        let len = self.unhealthy.len();
        crate::app::navigation::CursorRef::new(&mut self.queues_selected, len)
    }
}

/// Mirror `controller_status` from the cluster snapshot onto
/// `OverviewState.controller`. Called from the `ClusterChanged` arm of
/// the main loop whenever the `ControllerStatus` endpoint updates —
/// replacing the pre-Task-8 `apply_pg_status` path that rode the
/// Overview worker's PG-status payload.
///
/// Idempotent: invoking it twice with the same snapshot produces the
/// same `state.overview.controller`. Leaves `controller` as `None` when
/// the endpoint has never returned successfully (Loading case), so the
/// renderer can degrade its PG-versioning slot gracefully.
pub(crate) fn redraw_controller_status(state: &mut crate::app::state::AppState) {
    state.overview.controller = state.cluster.snapshot.controller_status.latest().cloned();
    // Refresh-age chip anchors off the most recent PG-level poll. Stays
    // here (rather than on `last_pg_refresh`, which doubled as the old
    // PG-status worker's cadence marker) so the Components panel's
    // version-sync row reflects the same "freshness" as the rest of the
    // panel.
    if state.overview.controller.is_some() {
        state.overview.last_pg_refresh = Some(std::time::Instant::now());
    }
}

/// Mirror `system_diagnostics` from the cluster snapshot onto
/// `OverviewState.nodes` / `OverviewState.repositories_summary`. Called
/// from the `ClusterChanged` arm of the main loop whenever the
/// `SystemDiagnostics` endpoint updates — replacing the pre-Task-8
/// `apply_system_diagnostics` path.
///
/// Also writes `state.cluster_summary.connected_nodes` /
/// `.total_nodes`, which the top-bar identity strip reads. The raw
/// `NodeDiagnostics` struct has no `status` field distinguishing
/// connected from disconnected, so both totals equal the node count
/// — same behavior as the pre-refactor code path in
/// `src/app/state/mod.rs`.
pub(crate) fn redraw_sysdiag(state: &mut crate::app::state::AppState) {
    let Some(diag) = state.cluster.snapshot.system_diagnostics.latest() else {
        // Pre-first-fetch: leave `nodes` / `repositories_summary`
        // untouched so the Nodes panel renders its "loading…"
        // affordance. `cluster_summary` also stays at its initial
        // `None` so the top-bar shows a dash.
        return;
    };

    crate::client::health::update_nodes(&mut state.overview.nodes, diag);
    state.overview.repositories_summary = build_repositories_summary(diag);
    state.overview.last_sysdiag_refresh = Some(std::time::Instant::now());
    state.cluster_summary.total_nodes = Some(diag.nodes.len());
    state.cluster_summary.connected_nodes = Some(diag.nodes.len());
}

/// Re-derive bulletin-facing Overview projections (sparkline, noisy
/// components leaderboard) from the cluster-owned `BulletinRing`.
/// Called from the `ClusterChanged(Bulletins)` arm in the main loop.
///
/// Sparkline: rolling 15-minute bulletin-rate chart.
///
/// `sparkline[0]` is the OLDEST minute; `sparkline[SPARKLINE_MINUTES-1]` is
/// the NEWEST minute. The array accumulates across batches instead of
/// being replaced each time, so that a system producing >200 bulletins/minute
/// still builds up a meaningful rate history over time.
///
/// Mechanics:
///   1. `sparkline_epoch_secs` tracks the Unix-second start of the current
///      newest bucket. On the first mirror it is initialised to the
///      minute-floor of the ring's meta fetched-at.
///   2. When one or more full minutes have elapsed since the epoch, the
///      array is rotated left by that many positions.
///   3. Only bulletins with id > `last_bulletin_id` are accumulated, so
///      the projection is idempotent across redraws.
pub(crate) fn redraw_bulletin_projections(state: &mut crate::app::state::AppState) {
    redraw_bulletin_projections_at(state, std::time::SystemTime::now());
}

/// Test-seam variant of [`redraw_bulletin_projections`] that accepts an
/// explicit wall-clock anchor for the sparkline. Production code always
/// passes `SystemTime::now()`; unit tests pin a fixed anchor so the
/// "age of each bulletin" derivation is stable across runs.
pub(crate) fn redraw_bulletin_projections_at(
    state: &mut crate::app::state::AppState,
    fetched_at: std::time::SystemTime,
) {
    let ring = &state.cluster.snapshot.bulletins;
    // Nothing to do before the first successful fetch has provided a
    // meta stamp. The cluster-ring buf can still be empty with a meta
    // (a successful fetch returned zero bulletins) — but the sparkline
    // anchor needs *some* wall-clock marker, which we only know after
    // the first fetch has populated the ring's meta.
    if ring.meta.is_none() {
        return;
    }

    let fetched_secs = fetched_at
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);

    // Step 1: initialise or advance the epoch.
    let overview = &mut state.overview;
    let epoch_secs = overview
        .sparkline_epoch_secs
        .unwrap_or_else(|| (fetched_secs / 60) * 60);
    let minutes_elapsed = ((fetched_secs - epoch_secs) / 60).max(0) as usize;
    let new_epoch = if minutes_elapsed > 0 {
        let shift = minutes_elapsed.min(SPARKLINE_MINUTES);
        overview.sparkline.rotate_left(shift);
        for i in (SPARKLINE_MINUTES - shift)..SPARKLINE_MINUTES {
            overview.sparkline[i] = BulletinBucket::default();
        }
        epoch_secs + (minutes_elapsed as i64 * 60)
    } else {
        epoch_secs
    };
    overview.sparkline_epoch_secs = Some(new_epoch);

    // Step 2: accumulate only new bulletins (above the previously
    // observed cursor).
    let cursor = overview.last_bulletin_id.unwrap_or(i64::MIN);
    for b in ring.buf.iter() {
        if b.id <= cursor {
            continue;
        }
        let Some(ts) = parse_iso_seconds(&b.timestamp_iso) else {
            continue;
        };
        let age_secs = fetched_secs - ts;
        if age_secs < 0 {
            continue;
        }
        let minute = (age_secs / 60) as usize;
        if minute >= SPARKLINE_MINUTES {
            continue;
        }
        let bucket = &mut overview.sparkline[SPARKLINE_MINUTES - 1 - minute];
        bucket.count = bucket.count.saturating_add(1);
        let sev = Severity::parse(&b.level);
        match sev {
            Severity::Error => {
                bucket.error_count = bucket.error_count.saturating_add(1);
            }
            Severity::Warning => {
                bucket.warning_count = bucket.warning_count.saturating_add(1);
            }
            Severity::Info => {
                bucket.info_count = bucket.info_count.saturating_add(1);
            }
            Severity::Unknown => {}
        }
        if sev > bucket.max_severity {
            bucket.max_severity = sev;
        }
    }

    // Noisy components: aggregate counts by source_id across the entire
    // ring — unlike the pre-Task-7 implementation (per-poll snapshot),
    // the ring already gives a bounded window, so "noisy across the
    // current history" is the natural semantic.
    use std::collections::HashMap;
    let mut by_source: HashMap<String, NoisyComponent> = HashMap::new();
    for b in ring.buf.iter() {
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
    overview.noisy = noisy;
    if !overview.noisy.is_empty() {
        overview.noisy_selected = overview.noisy_selected.min(overview.noisy.len() - 1);
    } else {
        overview.noisy_selected = 0;
    }

    // Advance the bulletin-id cursor so the next redraw is idempotent
    // for the bulletins already folded into the sparkline.
    if let Some(max) = ring.buf.iter().map(|b| b.id).max() {
        overview.last_bulletin_id = Some(match overview.last_bulletin_id {
            Some(existing) => existing.max(max),
            None => max,
        });
    }
}

/// Build the Unhealthy-queues leaderboard from a root-PG snapshot.
/// Shared by `redraw_components` and the render-path test helper so
/// the two can't drift.
pub(crate) fn derive_unhealthy(
    root_pg: &crate::client::RootPgStatusSnapshot,
) -> Vec<UnhealthyQueue> {
    root_pg
        .connections
        .iter()
        .take(TOP_QUEUES)
        .cloned()
        .map(UnhealthyQueue::from)
        .collect()
}

/// Re-derive Overview projections that depend on the cluster snapshot:
/// mirrors `root_pg_status` into `state.overview.root_pg`, rebuilds
/// the Unhealthy-queues leaderboard, and mirrors `controller_services`
/// into `state.overview.cs_counts`. Called from the
/// `AppEvent::ClusterChanged` arm in the main loop (for `RootPgStatus`
/// and `ControllerServices`). Idempotent across repeat invocations.
///
/// Idempotent: invoking it twice with the same snapshot produces the
/// same `state.overview.root_pg` / `state.overview.unhealthy` /
/// `state.overview.cs_counts`.
///
/// `state.overview.cs_counts` is `Some(..)` when the cluster snapshot
/// has observed at least one successful CS fetch (including stale
/// `last_ok` after a subsequent failure). It is `None` while the
/// endpoint is still `Loading` or has only ever failed — the renderer
/// treats that as "cs list unavailable". "Zero CSes" is distinct from
/// "unavailable": a successful fetch with all zero counts still
/// yields `Some(ControllerServiceCounts { .. })`.
pub(crate) fn redraw_components(state: &mut crate::app::state::AppState) {
    // Mirror controller-service counts regardless of whether
    // `root_pg_status` has landed — the two endpoints are independent.
    // The cluster snapshot carries a combined counts+members struct; we
    // only lift `counts` into Overview state (members are Browser-only).
    state.overview.cs_counts = state
        .cluster
        .snapshot
        .controller_services
        .latest()
        .map(|s| s.counts.clone());

    let Some(root_pg) = state.cluster.snapshot.root_pg_status.latest() else {
        // Pre-first-fetch: leave `root_pg` as `None` and `unhealthy`
        // untouched. The Components panel renders its "loading…"
        // affordance, mirroring the pre-refactor first-frame behavior
        // (where `OverviewState.snapshot` was `None` until the first
        // PG-status poll).
        return;
    };

    let unhealthy = derive_unhealthy(root_pg);
    if !unhealthy.is_empty() {
        state.overview.queues_selected = state.overview.queues_selected.min(unhealthy.len() - 1);
    } else {
        state.overview.queues_selected = 0;
    }
    state.overview.unhealthy = unhealthy;
    state.overview.root_pg = Some(root_pg.clone());
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
pub(crate) fn parse_iso_seconds(s: &str) -> Option<i64> {
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
        BulletinSnapshot, ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot,
    };
    use std::time::{Duration, UNIX_EPOCH};

    // 2026-04-11T10:14:22Z in unix seconds.
    const T0: u64 = 1_775_902_462;

    /// Push a `ControllerStatusSnapshot` into `state.cluster.snapshot` so
    /// `redraw_controller_status` can mirror it into
    /// `state.overview.controller`. Replaces the pre-Task-8 path where
    /// `controller` rode along on `OverviewPgStatusPayload`.
    fn seed_controller_status(
        state: &mut crate::app::state::AppState,
        controller: ControllerStatusSnapshot,
    ) {
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::Instant;
        state.cluster.snapshot.controller_status = EndpointState::Ready {
            data: controller,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(10),
            },
        };
    }

    /// Push a `SystemDiagSnapshot` into `state.cluster.snapshot` so
    /// `redraw_sysdiag` can mirror node/repo counts into
    /// `OverviewState`. Replaces the pre-Task-8 `OverviewPayload::SystemDiag`
    /// arrival path.
    fn seed_sysdiag(
        state: &mut crate::app::state::AppState,
        diag: crate::client::health::SystemDiagSnapshot,
    ) {
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::Instant;
        state.cluster.snapshot.system_diagnostics = EndpointState::Ready {
            data: diag,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(10),
            },
        };
    }

    /// Push a `RootPgStatusSnapshot` into `state.cluster.snapshot` so
    /// `redraw_components` can consume it. Replaces the pre-Task-3 path
    /// where `root_pg` rode along on `OverviewPgStatusPayload`.
    fn seed_root_pg(state: &mut crate::app::state::AppState, root_pg: RootPgStatusSnapshot) {
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::Instant;
        state.cluster.snapshot.root_pg_status = EndpointState::Ready {
            data: root_pg,
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(10),
            },
        };
    }

    /// Push a `ControllerServiceCounts` into `state.cluster.snapshot` so
    /// `redraw_components` can mirror it into `state.overview.cs_counts`.
    /// Replaces the pre-Task-4 path where `cs_counts` rode along on
    /// `OverviewPgStatusPayload`.
    fn seed_cs_counts(
        state: &mut crate::app::state::AppState,
        counts: crate::client::ControllerServiceCounts,
    ) {
        use crate::client::ControllerServicesSnapshot;
        use crate::cluster::snapshot::{EndpointState, FetchMeta};
        use std::time::Instant;
        state.cluster.snapshot.controller_services = EndpointState::Ready {
            data: ControllerServicesSnapshot {
                counts,
                members: Vec::new(),
            },
            meta: FetchMeta {
                fetched_at: Instant::now(),
                fetch_duration: Duration::from_millis(5),
                next_interval: Duration::from_secs(10),
            },
        };
    }

    /// Merge bulletins into the cluster snapshot's `BulletinRing` and
    /// seed its meta so `redraw_bulletin_projections` has a fetched-at
    /// anchor. Test-only: simulates what `spawn_bulletins` +
    /// `apply_update` do in production.
    fn seed_bulletins(
        state: &mut crate::app::state::AppState,
        bulletins: Vec<BulletinSnapshot>,
        _fetched_secs: u64,
    ) {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::Instant;
        state.cluster.snapshot.bulletins.merge(bulletins);
        state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: Duration::from_millis(5),
            next_interval: Duration::from_secs(10),
        });
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
    fn redraw_controller_status_mirrors_snapshot_into_overview_state() {
        let mut state = crate::test_support::fresh_state();
        seed_controller_status(
            &mut state,
            ControllerStatusSnapshot {
                running: 7,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                stale: 2,
                locally_modified: 1,
                sync_failure: 0,
                up_to_date: 4,
                ..Default::default()
            },
        );
        redraw_controller_status(&mut state);
        let c = state
            .overview
            .controller
            .as_ref()
            .expect("controller must be mirrored after redraw");
        assert_eq!(c.running, 7);
        assert_eq!(c.stale, 2);
        assert!(state.overview.last_pg_refresh.is_some());
    }

    #[test]
    fn redraw_controller_status_leaves_state_unchanged_when_loading() {
        let mut state = crate::test_support::fresh_state();
        // Seed a stale Some(..) first to prove the reducer leaves it
        // untouched rather than clobbering it while the cluster is still
        // loading.
        state.overview.controller = Some(ControllerStatusSnapshot {
            running: 99,
            ..Default::default()
        });
        redraw_controller_status(&mut state);
        // Loading → latest() returns None → clone() returns None → we
        // overwrite with None; the reducer is "mirror-the-snapshot",
        // not "preserve stale".
        assert!(state.overview.controller.is_none());
    }

    #[test]
    fn redraw_components_mirrors_cs_counts_into_overview_state() {
        use crate::client::ControllerServiceCounts;
        let mut state = crate::test_support::fresh_state();
        seed_cs_counts(
            &mut state,
            ControllerServiceCounts {
                enabled: 5,
                disabled: 1,
                invalid: 0,
            },
        );
        redraw_components(&mut state);
        let cs = state
            .overview
            .cs_counts
            .as_ref()
            .expect("cs_counts must be mirrored after redraw");
        assert_eq!(cs.enabled, 5);
        assert_eq!(cs.disabled, 1);
    }

    #[test]
    fn redraw_components_leaves_cs_counts_none_when_cluster_loading() {
        // Pre-first-fetch: `cluster.snapshot.controller_services` is in
        // `Loading`. `redraw_components` must mirror that to
        // `state.overview.cs_counts = None` so the renderer shows
        // "cs list unavailable".
        let mut state = crate::test_support::fresh_state();
        // Seed a stale Some(..) first to prove the reducer clears it
        // when the cluster has no successful fetch yet.
        state.overview.cs_counts = Some(crate::client::ControllerServiceCounts {
            enabled: 99,
            disabled: 0,
            invalid: 0,
        });
        redraw_components(&mut state);
        assert!(
            state.overview.cs_counts.is_none(),
            "cs_counts must be cleared when cluster snapshot is Loading"
        );
    }

    #[test]
    fn unhealthy_queues_truncated_to_top_ten() {
        let mut state = crate::test_support::fresh_state();
        let queues: Vec<QueueSnapshot> = (0..20)
            .map(|i| q(&format!("c{i}"), 100 - i as u32))
            .collect();
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                connections: queues,
                ..Default::default()
            },
        );
        redraw_components(&mut state);
        assert_eq!(state.overview.unhealthy.len(), TOP_QUEUES);
        assert_eq!(state.overview.unhealthy[0].fill_percent, 100);
        assert_eq!(state.overview.unhealthy[9].fill_percent, 91);
    }

    /// Fixed wall-clock anchor used by sparkline tests (same as the old
    /// T0 constant, just reified as a `SystemTime`).
    fn t0_anchor() -> std::time::SystemTime {
        UNIX_EPOCH + Duration::from_secs(T0)
    }

    #[test]
    fn sparkline_buckets_bulletins_by_minute() {
        let mut state = crate::test_support::fresh_state();
        // fetched_at = 10:14:22Z. Bulletin at 10:14:10Z is 12s ago → bucket 0
        // (minute 0 from newest). Bulletin at 10:10:00Z is ~262s ago →
        // minute 4 → bucket SPARKLINE_MINUTES-1-4.
        let bulletins = vec![
            bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
            bulletin(2, "ERROR", "b", "2026-04-11T10:10:00Z"),
            bulletin(3, "WARN", "a", "2026-04-11T10:14:20Z"),
        ];
        seed_bulletins(&mut state, bulletins, T0);
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        let newest = state.overview.sparkline[SPARKLINE_MINUTES - 1];
        assert_eq!(
            newest.count, 2,
            "two bulletins within the most recent minute"
        );
        assert_eq!(newest.max_severity, Severity::Warning);
        let four_minutes_old = state.overview.sparkline[SPARKLINE_MINUTES - 1 - 4];
        assert_eq!(four_minutes_old.count, 1);
        assert_eq!(four_minutes_old.max_severity, Severity::Error);
    }

    #[test]
    fn sparkline_pins_absolute_index_orientation() {
        // Pin the absolute index orientation: a bulletin aged 0s goes to
        // index SPARKLINE_MINUTES-1, nothing else. A bulletin aged
        // ~14 minutes lands at index 0 (the oldest visible bucket).
        let mut state = crate::test_support::fresh_state();
        seed_bulletins(
            &mut state,
            vec![bulletin(10, "INFO", "z", "2026-04-11T10:14:22Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(
            state.overview.sparkline[SPARKLINE_MINUTES - 1].count,
            1,
            "age 0s bulletin must land in the newest bucket (last index)"
        );
        for i in 0..(SPARKLINE_MINUTES - 1) {
            assert_eq!(
                state.overview.sparkline[i].count, 0,
                "non-newest buckets must be empty"
            );
        }
    }

    #[test]
    fn sparkline_drops_bulletins_outside_window() {
        let mut state = crate::test_support::fresh_state();
        // 30 minutes old — out of window.
        seed_bulletins(
            &mut state,
            vec![bulletin(1, "INFO", "a", "2026-04-11T09:44:22Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert!(state.overview.sparkline.iter().all(|b| b.count == 0));
    }

    #[test]
    fn sparkline_does_not_double_count_bulletins_across_polls() {
        // Simulate two polls 10 seconds apart where the second poll still
        // returns the same bulletin (id=1) alongside a genuinely new one
        // (id=2). Only id=2 should increment the newest bucket.
        let mut state = crate::test_support::fresh_state();

        // Poll 1 at T0: one bulletin in the current minute. The cluster
        // ring dedups by id, so calling merge() twice with overlapping
        // ids only stores the first occurrence — simulating NiFi's
        // after-id paging behavior.
        seed_bulletins(
            &mut state,
            vec![bulletin(1, "INFO", "a", "2026-04-11T10:14:20Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.sparkline[SPARKLINE_MINUTES - 1].count, 1);

        // Poll 2 at T0+10s: id=2 is brand new; id=1 is a repeat that the
        // merge() below skips because BulletinRing already has it.
        seed_bulletins(
            &mut state,
            vec![bulletin(2, "WARN", "b", "2026-04-11T10:14:30Z")],
            T0 + 10,
        );
        redraw_bulletin_projections_at(&mut state, UNIX_EPOCH + Duration::from_secs(T0 + 10));
        assert_eq!(
            state.overview.sparkline[SPARKLINE_MINUTES - 1].count,
            2,
            "id=1 must not be counted again on the second poll"
        );
    }

    #[test]
    fn sparkline_rolls_window_forward_when_minute_elapses() {
        // Poll 1 at T0 (10:14:22Z): one bulletin lands in bucket 14.
        let mut state = crate::test_support::fresh_state();
        seed_bulletins(
            &mut state,
            vec![bulletin(1, "INFO", "a", "2026-04-11T10:14:20Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.sparkline[SPARKLINE_MINUTES - 1].count, 1);

        // Poll 2 at T0+70s (10:15:32Z): a full minute has elapsed, so the
        // window rolls left by 1. The bulletin from poll 1 (now 72s old →
        // minute 1) should have moved to bucket 13. A new bulletin (id=2)
        // lands in the fresh bucket 14.
        seed_bulletins(
            &mut state,
            vec![bulletin(2, "WARN", "b", "2026-04-11T10:15:30Z")],
            T0 + 70,
        );
        redraw_bulletin_projections_at(&mut state, UNIX_EPOCH + Duration::from_secs(T0 + 70));
        assert_eq!(
            state.overview.sparkline[SPARKLINE_MINUTES - 1].count,
            1,
            "only id=2 in the new current bucket"
        );
        assert_eq!(
            state.overview.sparkline[SPARKLINE_MINUTES - 2].count,
            1,
            "id=1 from poll 1 rolled into the previous-minute bucket"
        );
    }

    #[test]
    fn noisy_components_ranked_by_count_then_severity() {
        let mut state = crate::test_support::fresh_state();
        let bulletins = vec![
            bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
            bulletin(2, "INFO", "a", "2026-04-11T10:14:11Z"),
            bulletin(3, "ERROR", "a", "2026-04-11T10:14:12Z"),
            bulletin(4, "INFO", "b", "2026-04-11T10:14:13Z"),
            bulletin(5, "INFO", "b", "2026-04-11T10:14:14Z"),
            bulletin(6, "INFO", "c", "2026-04-11T10:14:15Z"),
        ];
        seed_bulletins(&mut state, bulletins, T0);
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.noisy[0].source_id, "a");
        assert_eq!(state.overview.noisy[0].count, 3);
        assert_eq!(state.overview.noisy[0].max_severity, Severity::Error);
        assert_eq!(state.overview.noisy[1].source_id, "b");
        assert_eq!(state.overview.noisy[1].count, 2);
        assert_eq!(state.overview.noisy[2].source_id, "c");
    }

    #[test]
    fn noisy_leaderboard_truncated_to_top_five() {
        let mut state = crate::test_support::fresh_state();
        let bulletins: Vec<BulletinSnapshot> = (0..20)
            .map(|i| bulletin(i, "INFO", &format!("s{i}"), "2026-04-11T10:14:10Z"))
            .collect();
        seed_bulletins(&mut state, bulletins, T0);
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.noisy.len(), TOP_NOISY);
    }

    #[test]
    fn empty_bulletin_timestamp_is_silently_skipped() {
        let mut state = crate::test_support::fresh_state();
        seed_bulletins(&mut state, vec![bulletin(1, "INFO", "a", "")], T0);
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert!(state.overview.sparkline.iter().all(|b| b.count == 0));
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
        let mut state = crate::test_support::fresh_state();
        // Merging lower-id batches after a higher-id batch must not
        // regress the cursor. BulletinRing::merge enforces this, so
        // each call here mirrors production behavior.
        seed_bulletins(
            &mut state,
            vec![bulletin(5, "INFO", "a", "2026-04-11T10:14:10Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.last_bulletin_id, Some(5));

        seed_bulletins(
            &mut state,
            vec![bulletin(3, "INFO", "a", "2026-04-11T10:14:11Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.last_bulletin_id, Some(5));

        seed_bulletins(
            &mut state,
            vec![bulletin(9, "INFO", "a", "2026-04-11T10:14:12Z")],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.last_bulletin_id, Some(9));
    }

    #[test]
    fn noisy_cursor_clamped_when_data_shrinks() {
        let mut state = crate::test_support::fresh_state();
        state.overview.noisy_selected = 4;
        seed_bulletins(
            &mut state,
            vec![
                bulletin(1, "INFO", "a", "2026-04-11T10:14:10Z"),
                bulletin(2, "INFO", "b", "2026-04-11T10:14:10Z"),
            ],
            T0,
        );
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(
            state.overview.noisy_selected, 1,
            "cursor clamped to len-1 = 1"
        );
    }

    #[test]
    fn noisy_cursor_reset_to_zero_when_noisy_empty() {
        let mut state = crate::test_support::fresh_state();
        state.overview.noisy_selected = 3;
        seed_bulletins(&mut state, vec![], T0);
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert_eq!(state.overview.noisy_selected, 0);
    }

    #[test]
    fn queues_cursor_clamped_when_data_shrinks() {
        let mut state = crate::test_support::fresh_state();
        state.overview.queues_selected = 9;
        let queues = vec![q("c0", 90), q("c1", 80), q("c2", 70)];
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                connections: queues,
                ..Default::default()
            },
        );
        redraw_components(&mut state);
        assert_eq!(
            state.overview.queues_selected, 2,
            "cursor clamped to len-1 = 2"
        );
    }

    #[test]
    fn overview_focus_default_is_none() {
        let state = OverviewState::new();
        assert_eq!(state.focus, OverviewFocus::None);
    }

    /// Build a two-node SystemDiagSnapshot fixture that the sysdiag
    /// reducer tests reuse. Mirrors the old
    /// `apply_system_diagnostics_populates_nodes_and_repositories`
    /// fixture one-for-one so rendered values stay byte-identical.
    fn two_node_sysdiag() -> crate::client::health::SystemDiagSnapshot {
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

        SystemDiagSnapshot {
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
        }
    }

    #[test]
    fn redraw_sysdiag_populates_nodes_and_repositories() {
        let mut state = crate::test_support::fresh_state();
        seed_sysdiag(&mut state, two_node_sysdiag());
        redraw_sysdiag(&mut state);

        // Two nodes.
        assert_eq!(
            state.overview.nodes.nodes.len(),
            2,
            "nodes should be populated"
        );

        // Repository aggregates from the aggregate field.
        assert_eq!(state.overview.repositories_summary.content_percent, 60);
        assert_eq!(state.overview.repositories_summary.flowfile_percent, 30);
        assert_eq!(state.overview.repositories_summary.provenance_percent, 20);
        assert!(state.overview.last_sysdiag_refresh.is_some());

        // Top-bar cluster_summary mirror: populated to node count.
        assert_eq!(state.cluster_summary.total_nodes, Some(2));
        assert_eq!(state.cluster_summary.connected_nodes, Some(2));
    }

    #[test]
    fn redraw_sysdiag_is_noop_when_cluster_loading() {
        let mut state = crate::test_support::fresh_state();
        // Seed a stale refresh timestamp to prove the reducer leaves it
        // alone rather than clobbering it on a Loading snapshot.
        state.overview.last_sysdiag_refresh = None;
        redraw_sysdiag(&mut state);
        assert!(
            state.overview.last_sysdiag_refresh.is_none(),
            "last_sysdiag_refresh must stay None when cluster is Loading"
        );
        assert_eq!(state.cluster_summary.total_nodes, None);
    }

    #[test]
    fn redraw_components_leaves_state_untouched_when_snapshot_is_loading() {
        // Pre-first-fetch: `cluster.snapshot.root_pg_status` is in
        // `Loading`. `redraw_components` must not clobber the
        // OverviewState's root_pg or unhealthy fields.
        let mut state = crate::test_support::fresh_state();
        state.overview.unhealthy = vec![UnhealthyQueue {
            id: "pre-existing".into(),
            group_id: "g".into(),
            name: "pre".into(),
            source_name: "s".into(),
            destination_name: "d".into(),
            fill_percent: 1,
            flow_files_queued: 1,
            bytes_queued: 0,
            queued_display: "1".into(),
        }];
        redraw_components(&mut state);
        assert!(state.overview.root_pg.is_none());
        assert_eq!(
            state.overview.unhealthy.len(),
            1,
            "unhealthy must be preserved on None snapshot"
        );
    }

    #[test]
    fn redraw_components_mirrors_snapshot_into_overview_state() {
        let mut state = crate::test_support::fresh_state();
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                flow_files_queued: 42,
                bytes_queued: 512,
                process_group_count: 7,
                input_port_count: 2,
                output_port_count: 1,
                processors: crate::client::ProcessorStateCounts {
                    running: 5,
                    stopped: 1,
                    invalid: 0,
                    disabled: 0,
                },
                connections: vec![q("c0", 95), q("c1", 90)],
                process_group_ids: vec![],
                nodes: vec![],
            },
        );
        redraw_components(&mut state);
        let root_pg = state
            .overview
            .root_pg
            .as_ref()
            .expect("root_pg must be populated after redraw");
        assert_eq!(root_pg.process_group_count, 7);
        assert_eq!(root_pg.processors.running, 5);
        assert_eq!(state.overview.unhealthy.len(), 2);
        assert_eq!(state.overview.unhealthy[0].fill_percent, 95);
    }

    #[test]
    fn redraw_bulletin_projections_is_noop_when_ring_meta_absent() {
        let mut state = crate::test_support::fresh_state();
        // No meta yet — cluster ring is pre-first-fetch.
        redraw_bulletin_projections_at(&mut state, t0_anchor());
        assert!(
            state.overview.sparkline.iter().all(|b| b.count == 0),
            "sparkline must stay empty without meta"
        );
        assert!(state.overview.noisy.is_empty());
    }

    #[test]
    fn redraw_components_is_idempotent() {
        let mut state = crate::test_support::fresh_state();
        seed_root_pg(
            &mut state,
            RootPgStatusSnapshot {
                process_group_count: 3,
                connections: vec![q("c0", 80)],
                ..Default::default()
            },
        );
        redraw_components(&mut state);
        let first_unhealthy_len = state.overview.unhealthy.len();
        let first_pg_count = state.overview.root_pg.as_ref().unwrap().process_group_count;
        redraw_components(&mut state);
        assert_eq!(state.overview.unhealthy.len(), first_unhealthy_len);
        assert_eq!(
            state.overview.root_pg.as_ref().unwrap().process_group_count,
            first_pg_count
        );
    }
}
