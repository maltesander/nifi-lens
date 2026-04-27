use super::super::state::{BulletinBucket, NoisyComponent, Severity};
use crate::client::{
    BulletinSnapshot, ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot,
};
use crate::view::overview::state::OverviewState;
use ratatui::Terminal;
use ratatui::backend::TestBackend;

// 2026-04-11T10:14:22Z in unix seconds. Same constant as the reducer
// tests so time-dependent rendering stays deterministic. Verified
// with `date -u -d @1775902462`.
const T0: u64 = 1_775_902_462;

fn render_to_string(state: &OverviewState) -> String {
    let backend = TestBackend::new(100, 25);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| super::render(f, f.area(), state)).unwrap();
    format!("{}", term.backend())
}

/// Mirror `redraw_components`'s effect on the `OverviewState`
/// (without threading through `&mut AppState`): set `root_pg` and
/// derive `unhealthy` via the shared `derive_unhealthy` helper so
/// this test path can't drift from the live reducer.
fn seed_root_pg(state: &mut OverviewState, root_pg: RootPgStatusSnapshot) {
    state.unhealthy = crate::view::overview::state::derive_unhealthy(&root_pg);
    state.root_pg = Some(root_pg);
}

/// Mirror what `redraw_controller_status` would do: write the
/// supplied snapshot into `state.controller`. Replaces the pre-Task-8
/// `apply_payload(PgStatus(..))` path in render tests.
fn seed_controller_status(state: &mut OverviewState, controller: ControllerStatusSnapshot) {
    state.controller = Some(controller);
}

/// Mirror what `redraw_sysdiag` would do: write the `nodes` /
/// `repositories_summary` projections without threading an
/// `&mut AppState`. Replaces the pre-Task-8
/// `apply_payload(SystemDiag(..))` path in render tests.
fn seed_sysdiag(state: &mut OverviewState, diag: &crate::client::overview::SystemDiagSnapshot) {
    use crate::view::overview::state::RepositoriesSummary;
    crate::client::overview::update_nodes(&mut state.nodes, diag, None, None);
    let avg = |repos: &[crate::client::overview::RepoUsage]| -> u32 {
        if repos.is_empty() {
            0
        } else {
            repos.iter().map(|r| r.utilization_percent).sum::<u32>() / repos.len() as u32
        }
    };
    let agg = &diag.aggregate;
    state.repositories_summary = RepositoriesSummary {
        content_percent: avg(&agg.content_repos),
        flowfile_percent: agg
            .flowfile_repo
            .as_ref()
            .map(|r| r.utilization_percent)
            .unwrap_or(0),
        provenance_percent: avg(&agg.provenance_repos),
    };
}

/// Render-test shim: build the sparkline + noisy-components
/// projections that `redraw_bulletin_projections` would produce
/// against the cluster ring, without constructing a full
/// `AppState`. Keeps the pre-Task-7 snapshot expectations stable
/// for snapshot tests that feed bulletins as a vector.
fn seed_bulletin_projections_from_bulletins(
    state: &mut OverviewState,
    bulletins: &[BulletinSnapshot],
    fetched_secs: i64,
) {
    use super::super::state::{SPARKLINE_MINUTES, parse_iso_seconds};
    use std::collections::HashMap;

    // Sparkline — mirror the bulk of `redraw_bulletin_projections`.
    let epoch_secs = state
        .sparkline_epoch_secs
        .unwrap_or_else(|| (fetched_secs / 60) * 60);
    let minutes_elapsed = ((fetched_secs - epoch_secs) / 60).max(0) as usize;
    let new_epoch = if minutes_elapsed > 0 {
        let shift = minutes_elapsed.min(SPARKLINE_MINUTES);
        state.sparkline.rotate_left(shift);
        for i in (SPARKLINE_MINUTES - shift)..SPARKLINE_MINUTES {
            state.sparkline[i] = BulletinBucket::default();
        }
        epoch_secs + (minutes_elapsed as i64 * 60)
    } else {
        epoch_secs
    };
    state.sparkline_epoch_secs = Some(new_epoch);

    for b in bulletins {
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
        let bucket = &mut state.sparkline[SPARKLINE_MINUTES - 1 - minute];
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

    // Noisy components.
    let mut by_source: HashMap<String, NoisyComponent> = HashMap::new();
    for b in bulletins {
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
    noisy.truncate(super::super::state::TOP_NOISY);
    state.noisy = noisy;
}

#[test]
fn snapshot_empty_state() {
    let state = OverviewState::new();
    insta::assert_snapshot!("overview_empty", render_to_string(&state));
}

#[test]
fn snapshot_healthy_cluster() {
    let mut state = OverviewState::new();
    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
            running: 42,
            stopped: 3,
            invalid: 0,
            disabled: 1,
            active_threads: 5,
            flow_files_queued: 120,
            bytes_queued: 4096,
            stale: 0,
            locally_modified: 0,
            sync_failure: 0,
            up_to_date: 0,
        },
    );
    // Render reads `state.root_pg` and `state.cs_counts` directly
    // — seed the projections that `redraw_components` would
    // normally populate from the cluster snapshot.
    seed_root_pg(
        &mut state,
        RootPgStatusSnapshot {
            flow_files_queued: 120,
            bytes_queued: 4096,
            connections: vec![QueueSnapshot {
                id: "c1".into(),
                group_id: "root".into(),
                name: "ingest → enrich".into(),
                source_name: "Generate".into(),
                destination_name: "Enrich".into(),
                fill_percent: 12,
                flow_files_queued: 40,
                bytes_queued: 512,
                queued_display: "40 / 512 B".into(),
            }],
            process_group_count: 5,
            input_port_count: 2,
            output_port_count: 1,
            processors: crate::client::ProcessorStateCounts {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
            },
            process_group_ids: vec![],
            nodes: vec![],
        },
    );
    state.cs_counts = Some(crate::client::ControllerServiceCounts {
        enabled: 12,
        disabled: 0,
        invalid: 0,
    });
    insta::assert_snapshot!("overview_healthy", render_to_string(&state));
}

#[test]
fn snapshot_drift() {
    use crate::client::{ControllerServiceCounts, ProcessorStateCounts};
    let mut state = OverviewState::new();
    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
            running: 42,
            stopped: 3,
            invalid: 0,
            disabled: 1,
            stale: 1,
            locally_modified: 2,
            sync_failure: 0,
            up_to_date: 4,
            ..Default::default()
        },
    );
    seed_root_pg(
        &mut state,
        RootPgStatusSnapshot {
            process_group_count: 7,
            input_port_count: 2,
            output_port_count: 1,
            processors: ProcessorStateCounts {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
            },
            ..Default::default()
        },
    );
    state.cs_counts = Some(ControllerServiceCounts {
        enabled: 12,
        disabled: 0,
        invalid: 0,
    });
    let out = render_to_string(&state);
    // Per AGENTS.md, the Components panel must surface version-drift counts
    // (stale, locally-modified, sync-failure) inline. Assert both markers exist.
    assert!(
        out.contains("STALE") && out.contains("1"),
        "drift panel must surface stale count: {out}"
    );
    assert!(
        out.contains("MODIFIED") && out.contains("2"),
        "drift panel must surface locally-modified count: {out}"
    );
    insta::assert_snapshot!("overview_drift", out);
}

#[test]
fn snapshot_cs_unavailable() {
    let mut state = OverviewState::new();
    seed_controller_status(&mut state, ControllerStatusSnapshot::default());
    seed_root_pg(&mut state, RootPgStatusSnapshot::default());
    // `state.cs_counts` is left as the default `None` to exercise
    // the "cs list unavailable" degradation path.
    let out = render_to_string(&state);
    // Per AGENTS.md, the CS row should collapse to a "cs list unavailable" chip
    // when the fetch fails.
    assert!(
        out.contains("cs list unavailable"),
        "CS-unavailable degradation chip must render: {out}"
    );
    insta::assert_snapshot!("overview_cs_unavailable", out);
}

#[test]
fn snapshot_unhealthy_cluster() {
    let mut state = OverviewState::new();
    let queues = (0..5)
        .map(|i| QueueSnapshot {
            id: format!("c{i}"),
            group_id: "root".into(),
            name: format!("q{i}"),
            source_name: "Generate".into(),
            destination_name: format!("Proc{i}"),
            fill_percent: 99 - i,
            flow_files_queued: 9_000 + i * 100,
            bytes_queued: 1_000_000,
            queued_display: format!("{}k / 1 MB", 9 + i),
        })
        .collect();
    // Pre-Task-7 the bulletins rode on the PG-status payload and
    // drove sparkline+noisy via `apply_payload`. Task 7 moved that
    // path to `redraw_bulletin_projections` on `&mut AppState`.
    // This render test drives `OverviewState` directly, so we
    // pre-populate the projections that the reducer would have
    // built, keeping the rendered output stable.
    let bulletins: Vec<BulletinSnapshot> = (0..6)
        .map(|i| BulletinSnapshot {
            id: i,
            level: if i % 2 == 0 {
                "ERROR".into()
            } else {
                "WARN".into()
            },
            message: "msg".into(),
            source_id: format!("proc-{}", i % 3),
            source_name: format!("Proc{}", i % 3),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "2026-04-11T10:14:10Z".into(),
            timestamp_human: String::new(),
        })
        .collect();
    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
            running: 20,
            stopped: 10,
            invalid: 2,
            disabled: 0,
            active_threads: 17,
            flow_files_queued: 50_000,
            bytes_queued: 8_000_000,
            stale: 0,
            locally_modified: 0,
            sync_failure: 0,
            up_to_date: 0,
        },
    );
    // Hand-build the bulletin-derived projections matching the
    // pre-Task-7 output.
    seed_bulletin_projections_from_bulletins(&mut state, &bulletins, T0 as i64);
    seed_root_pg(
        &mut state,
        RootPgStatusSnapshot {
            flow_files_queued: 50_000,
            bytes_queued: 8_000_000,
            connections: queues,
            process_group_count: 4,
            input_port_count: 0,
            output_port_count: 0,
            processors: crate::client::ProcessorStateCounts {
                running: 20,
                stopped: 10,
                invalid: 2,
                disabled: 0,
            },
            process_group_ids: vec![],
            nodes: vec![],
        },
    );
    state.cs_counts = Some(crate::client::ControllerServiceCounts {
        enabled: 6,
        disabled: 1,
        invalid: 1,
    });
    insta::assert_snapshot!("overview_unhealthy", render_to_string(&state));
}

#[test]
fn snapshot_with_nodes_populated() {
    use crate::client::overview::{
        GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
    };
    use std::time::Instant;

    let mut state = OverviewState::new();

    // Seed controller_status so the processor info line has data.
    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
            running: 42,
            stopped: 3,
            invalid: 0,
            disabled: 1,
            active_threads: 5,
            flow_files_queued: 120,
            bytes_queued: 4096,
            stale: 0,
            locally_modified: 0,
            sync_failure: 0,
            up_to_date: 0,
        },
    );

    // Then a two-node sysdiag projection. Fixture mirrors the
    // reducer test in src/view/overview/state.rs.
    let node = |address: &str| NodeDiagnostics {
        address: address.into(),
        heap_used_bytes: crate::bytes::FIXTURE_HEAP_USED,
        heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
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
    seed_sysdiag(&mut state, &diag);
    seed_root_pg(
        &mut state,
        RootPgStatusSnapshot {
            flow_files_queued: 120,
            bytes_queued: 4096,
            connections: vec![],
            process_group_count: 5,
            input_port_count: 2,
            output_port_count: 1,
            processors: crate::client::ProcessorStateCounts {
                running: 42,
                stopped: 3,
                invalid: 0,
                disabled: 1,
            },
            process_group_ids: vec![],
            nodes: vec![],
        },
    );
    state.cs_counts = Some(crate::client::ControllerServiceCounts {
        enabled: 12,
        disabled: 0,
        invalid: 0,
    });

    let out = render_to_string(&state);
    // Per AGENTS.md, the Nodes panel renders seeded node addresses and health metrics.
    // Spot-check that both node addresses survived rendering, indicating the table rows
    // were properly populated.
    assert!(
        out.contains("node1:8080"),
        "Nodes panel must render first node address: {out}"
    );
    assert!(
        out.contains("node2:8080"),
        "Nodes panel must render second node address: {out}"
    );
    insta::assert_snapshot!("overview_with_nodes", out);
}

#[test]
fn nodes_panel_scrolls_to_selected() {
    use crate::client::overview::{
        GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
    };
    use std::time::Instant;

    let mut state = OverviewState::new();
    state.focus = crate::view::overview::state::OverviewFocus::Nodes;

    let node = |i: usize| NodeDiagnostics {
        address: format!("node{}:8080", i),
        heap_used_bytes: 256 * crate::bytes::MIB,
        heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
        gc: vec![GcSnapshot {
            name: "G1 Young".into(),
            collection_count: 1,
            collection_millis: 5,
        }],
        load_average: Some(0.5),
        available_processors: Some(4),
        total_threads: 20,
        uptime: "1h".into(),
        content_repos: vec![RepoUsage {
            identifier: "c".into(),
            used_bytes: 10,
            total_bytes: 100,
            free_bytes: 90,
            utilization_percent: 10,
        }],
        flowfile_repo: Some(RepoUsage {
            identifier: "f".into(),
            used_bytes: 10,
            total_bytes: 100,
            free_bytes: 90,
            utilization_percent: 10,
        }),
        provenance_repos: vec![RepoUsage {
            identifier: "p".into(),
            used_bytes: 10,
            total_bytes: 100,
            free_bytes: 90,
            utilization_percent: 10,
        }],
    };

    let diag = SystemDiagSnapshot {
        aggregate: SystemDiagAggregate {
            content_repos: vec![RepoUsage {
                identifier: "c".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }],
            flowfile_repo: Some(RepoUsage {
                identifier: "f".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }),
            provenance_repos: vec![RepoUsage {
                identifier: "p".into(),
                used_bytes: 10,
                total_bytes: 100,
                free_bytes: 90,
                utilization_percent: 10,
            }],
        },
        nodes: (0..10).map(node).collect(),
        fetched_at: Instant::now(),
    };
    seed_sysdiag(&mut state, &diag);
    state.nodes.selected = 9;

    let output = render_to_string(&state);

    assert!(
        output.contains("node9:8080"),
        "selected row must be visible after scroll"
    );
    assert!(
        !output.contains("node0:8080"),
        "node0 must be scrolled out of view"
    );
    assert!(
        !output.contains("node1:8080"),
        "node1 must be scrolled out of view"
    );
    assert!(
        !output.contains("more"),
        "'... +N more' placeholder must not appear"
    );
}

#[test]
fn noisy_panel_scrolls_to_selected() {
    use crate::client::{ControllerStatusSnapshot, RootPgStatusSnapshot};
    use crate::view::overview::state::{NoisyComponent, Severity as OvSev};

    let mut state = OverviewState::new();
    state.focus = crate::view::overview::state::OverviewFocus::Noisy;

    // Populate enough state for the layout to render properly.
    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
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
        },
    );
    seed_root_pg(&mut state, RootPgStatusSnapshot::default());

    // Five noisy components with distinct names. zone[2] inner height=4,
    // visible_rows=3, so selected=4 forces scroll_offset=2. "alfa" scrolls away; "echo" appears.
    state.noisy = vec![
        NoisyComponent {
            source_id: "a".into(),
            group_id: "g".into(),
            source_name: "alfa".into(),
            count: 1,
            max_severity: OvSev::Info,
        },
        NoisyComponent {
            source_id: "b".into(),
            group_id: "g".into(),
            source_name: "bravo".into(),
            count: 1,
            max_severity: OvSev::Info,
        },
        NoisyComponent {
            source_id: "c".into(),
            group_id: "g".into(),
            source_name: "charlie".into(),
            count: 1,
            max_severity: OvSev::Info,
        },
        NoisyComponent {
            source_id: "d".into(),
            group_id: "g".into(),
            source_name: "delta".into(),
            count: 1,
            max_severity: OvSev::Info,
        },
        NoisyComponent {
            source_id: "e".into(),
            group_id: "g".into(),
            source_name: "echo".into(),
            count: 1,
            max_severity: OvSev::Info,
        },
    ];
    state.noisy_selected = 4;

    let output = render_to_string(&state);

    assert!(
        output.contains("echo"),
        "selected row 'echo' must be visible after scroll"
    );
    assert!(
        !output.contains("alfa"),
        "'alfa' must be scrolled out of view"
    );
}

#[test]
fn queues_panel_scrolls_to_selected() {
    use crate::client::{ControllerStatusSnapshot, QueueSnapshot, RootPgStatusSnapshot};

    let mut state = OverviewState::new();
    state.focus = crate::view::overview::state::OverviewFocus::Queues;

    // Ten queues with distinct names. With 0 nodes the queues inner area
    // is 10 rows tall, giving visible_rows=9 (one row is the header).
    // selected=9 forces scroll_offset=1. "alfa" scrolls away; "juliet" appears.
    let names = [
        "alfa", "bravo", "charlie", "delta", "echo", "foxtrot", "golf", "hotel", "india", "juliet",
    ];
    let connections: Vec<QueueSnapshot> = names
        .iter()
        .enumerate()
        .map(|(i, &name)| QueueSnapshot {
            id: format!("c{i}"),
            group_id: "root".into(),
            name: name.into(),
            source_name: "Src".into(),
            destination_name: "Dst".into(),
            fill_percent: 99,
            flow_files_queued: 100,
            bytes_queued: 0,
            queued_display: "100".into(),
        })
        .collect();

    seed_controller_status(
        &mut state,
        ControllerStatusSnapshot {
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
        },
    );
    seed_root_pg(
        &mut state,
        RootPgStatusSnapshot {
            flow_files_queued: 0,
            bytes_queued: 0,
            connections,
            ..Default::default()
        },
    );
    state.queues_selected = 9;

    let output = render_to_string(&state);

    assert!(
        output.contains("juliet"),
        "selected row 'juliet' must be visible after scroll"
    );
    assert!(
        !output.contains("alfa"),
        "'alfa' must be scrolled out of view"
    );
}

// ── T21 helpers and snapshot tests ───────────────────────────────────────

/// Build a two-node `AppState` with sysdiag pre-seeded and basic
/// controller/root-pg/cs data for a complete render.  The cluster-nodes
/// snapshot is NOT yet applied, so every `NodeHealthRow` has
/// `cluster = None` — this is the `any_cluster = false` baseline.
fn seed_state_with_two_nodes() -> crate::app::state::AppState {
    use crate::client::overview::{
        GcSnapshot, NodeDiagnostics, RepoUsage, SystemDiagAggregate, SystemDiagSnapshot,
    };
    use crate::cluster::snapshot::{EndpointState, FetchMeta};
    use std::time::Instant;

    let mut state = crate::test_support::fresh_state();

    // Seed controller_status.
    state.cluster.snapshot.controller_status = EndpointState::Ready {
        data: ControllerStatusSnapshot {
            running: 42,
            stopped: 3,
            invalid: 0,
            disabled: 1,
            active_threads: 5,
            flow_files_queued: 120,
            bytes_queued: 4096,
            stale: 0,
            locally_modified: 0,
            sync_failure: 0,
            up_to_date: 0,
        },
        meta: FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: std::time::Duration::from_secs(10),
        },
    };
    crate::view::overview::state::redraw_controller_status(&mut state);

    // Seed root-pg status.
    let root_pg = RootPgStatusSnapshot {
        flow_files_queued: 120,
        bytes_queued: 4096,
        connections: vec![],
        process_group_count: 5,
        input_port_count: 2,
        output_port_count: 1,
        processors: crate::client::ProcessorStateCounts {
            running: 42,
            stopped: 3,
            invalid: 0,
            disabled: 1,
        },
        process_group_ids: vec![],
        nodes: vec![],
    };
    state.cluster.snapshot.root_pg_status = EndpointState::Ready {
        data: root_pg,
        meta: FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: std::time::Duration::from_secs(10),
        },
    };
    crate::view::overview::state::redraw_components(&mut state);

    // Seed sysdiag with two nodes.
    let node = |address: &str| NodeDiagnostics {
        address: address.into(),
        heap_used_bytes: crate::bytes::FIXTURE_HEAP_USED,
        heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
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
    state.cluster.snapshot.system_diagnostics = EndpointState::Ready {
        data: diag,
        meta: FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: std::time::Duration::from_secs(10),
        },
    };
    crate::view::overview::state::redraw_sysdiag(&mut state);

    state
}

#[test]
fn snapshot_overview_with_cluster_roles() {
    use crate::client::overview::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
    use crate::cluster::snapshot::FetchMeta;

    // Seed two-node sysdiag, then apply cluster-nodes with primary +
    // coordinator.  Title: "Nodes (2/2 connected)".
    let mut state = seed_state_with_two_nodes();
    let cluster = ClusterNodesSnapshot {
        rows: vec![
            ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8080".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: true,
                is_coordinator: false,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 4,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            },
            ClusterNodeRow {
                node_id: "id-2".into(),
                address: "node2:8080".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: false,
                is_coordinator: true,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 3,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            },
        ],
        fetched_at: std::time::Instant::now(),
        fetched_wall: time::OffsetDateTime::now_utc(),
    };
    state.cluster.snapshot.cluster_nodes.apply(
        Ok(cluster),
        FetchMeta {
            fetched_at: std::time::Instant::now(),
            fetch_duration: std::time::Duration::from_millis(1),
            next_interval: std::time::Duration::from_secs(5),
        },
    );
    crate::view::overview::state::redraw_cluster_nodes(&mut state);
    insta::assert_snapshot!(
        "overview_with_cluster_roles",
        render_to_string(&state.overview)
    );
}

#[test]
fn snapshot_overview_with_dead_node() {
    use crate::client::overview::{ClusterNodeRow, ClusterNodeStatus, ClusterNodesSnapshot};
    use crate::cluster::snapshot::FetchMeta;

    // node1 connected primary+coordinator; node2 disconnected.
    // Expected dim/─── cells on the dead row; title "Nodes (1/2 connected)".
    let mut state = seed_state_with_two_nodes();
    let cluster = ClusterNodesSnapshot {
        rows: vec![
            ClusterNodeRow {
                node_id: "id-1".into(),
                address: "node1:8080".into(),
                status: ClusterNodeStatus::Connected,
                is_primary: true,
                is_coordinator: true,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 4,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            },
            ClusterNodeRow {
                node_id: "id-2".into(),
                address: "node2:8080".into(),
                status: ClusterNodeStatus::Disconnected,
                is_primary: false,
                is_coordinator: false,
                heartbeat_iso: None,
                node_start_iso: None,
                active_thread_count: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                events: vec![],
            },
        ],
        fetched_at: std::time::Instant::now(),
        fetched_wall: time::OffsetDateTime::now_utc(),
    };
    state.cluster.snapshot.cluster_nodes.apply(
        Ok(cluster),
        FetchMeta {
            fetched_at: std::time::Instant::now(),
            fetch_duration: std::time::Duration::from_millis(1),
            next_interval: std::time::Duration::from_secs(5),
        },
    );
    crate::view::overview::state::redraw_cluster_nodes(&mut state);
    insta::assert_snapshot!("overview_with_dead_node", render_to_string(&state.overview));
}

#[test]
fn snapshot_overview_standalone_no_badges() {
    // No cluster-nodes snapshot applied. Every row has cluster = None,
    // so any_cluster = false and the pre-T20 4-column layout is
    // preserved (no badge column, old title format).
    let state = seed_state_with_two_nodes();
    insta::assert_snapshot!(
        "overview_standalone_no_badges",
        render_to_string(&state.overview)
    );
}

// ── T24 cert-chip column tests ────────────────────────────────────────────

/// Fixed "now" shared with node_detail tests: 2026-04-24T00:00Z.
fn fixed_now() -> time::OffsetDateTime {
    time::macros::datetime!(2026-04-24 00:00 UTC)
}

/// Build a minimal `NodeHealthRow` with the given address and `tls_cert`.
fn row_with_tls(
    address: &str,
    tls_cert: Option<
        Result<crate::client::tls_cert::NodeCertChain, crate::client::tls_cert::TlsProbeError>,
    >,
) -> crate::client::overview::NodeHealthRow {
    use crate::client::overview::Severity;
    crate::client::overview::NodeHealthRow {
        node_address: address.into(),
        heap_used_bytes: crate::bytes::FIXTURE_HEAP_USED,
        heap_max_bytes: crate::bytes::FIXTURE_HEAP_MAX,
        heap_percent: 50,
        heap_severity: Severity::Green,
        gc_collection_count: 5,
        gc_delta: None,
        gc_millis: 20,
        load_average: Some(0.5),
        available_processors: Some(4),
        uptime: "1h".into(),
        total_threads: 20,
        gc: vec![],
        content_repos: vec![],
        flowfile_repo: None,
        provenance_repos: vec![],
        cluster: None,
        tls_cert,
    }
}

/// Build a `NodeCertChain` whose earliest `not_after` is `fixed_now() + days`.
fn chain_expiring_in(days: i64) -> crate::client::tls_cert::NodeCertChain {
    use crate::client::tls_cert::{CertEntry, NodeCertChain};
    NodeCertChain {
        entries: vec![CertEntry {
            subject_cn: Some("n".into()),
            not_after: fixed_now() + time::Duration::days(days),
            is_leaf: true,
        }],
    }
}

/// Build a minimal `OverviewState` containing exactly the supplied node rows.
fn state_with_node_rows(
    rows: Vec<crate::client::overview::NodeHealthRow>,
) -> crate::view::overview::state::OverviewState {
    use crate::view::overview::state::{OverviewState, RepositoriesSummary};
    let mut state = OverviewState::new();
    state.nodes.nodes = rows;
    state.repositories_summary = RepositoriesSummary {
        content_percent: 42,
        flowfile_percent: 18,
        provenance_percent: 7,
    };
    state
}

#[test]
fn snapshot_nodes_list_cert_chips_mixed() {
    use crate::client::tls_cert::TlsProbeError;
    let rows = vec![
        // n1: no TLS data → empty chip
        row_with_tls("n1:8443", None),
        // n2: expires in 400 days → silent (>= 30d threshold)
        row_with_tls("n3:8443", Some(Ok(chain_expiring_in(400)))),
        // n3: expires in 14 days → yellow "cert 14d"
        row_with_tls("n3:8443", Some(Ok(chain_expiring_in(14)))),
        // n4: expires in 3 days → red/bold "cert 3d"
        row_with_tls("n4:8443", Some(Ok(chain_expiring_in(3)))),
        // n5: expired 2 days ago → red/bold "cert expired"
        row_with_tls("n5:8443", Some(Ok(chain_expiring_in(-2)))),
        // n6: probe failed → empty chip (silent)
        row_with_tls(
            "n6:8443",
            Some(Err(TlsProbeError::Connect("refused".into()))),
        ),
    ];
    let state = state_with_node_rows(rows);
    let backend = ratatui::backend::TestBackend::new(110, 12);
    let mut term = Terminal::new(backend).unwrap();
    term.draw(|f| super::nodes::render_nodes_zone_at(f, f.area(), &state, false, fixed_now()))
        .unwrap();
    insta::assert_snapshot!("nodes_list_cert_chips_mixed", format!("{}", term.backend()));
}
