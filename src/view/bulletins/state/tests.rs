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
        timestamp_human: String::new(),
    }
}

/// Test-only helper that mimics the pre-Task-7 `apply_payload` on
/// a bare `BulletinsState`. Production code goes through
/// `redraw_bulletins(&mut AppState)`; these reducer-shape tests
/// don't need the full `AppState` stack.
fn apply_payload_test(state: &mut BulletinsState, bulletins: Vec<BulletinSnapshot>) {
    let before_len = state.ring.len();
    let existing_ids: HashSet<i64> = state.ring.iter().map(|b| b.id).collect();
    for bulletin in bulletins {
        if existing_ids.contains(&bulletin.id) {
            continue;
        }
        state.ring.push_back(bulletin);
    }
    let mut new_matching = 0u32;
    if !state.auto_scroll {
        for bulletin in state.ring.iter().skip(before_len) {
            if state.row_matches(bulletin) {
                new_matching = new_matching.saturating_add(1);
            }
        }
    }
    while state.ring.len() > state.ring_capacity {
        state.ring.pop_front();
    }
    state.last_fetched_at = Some(UNIX_EPOCH + Duration::from_secs(T0));
    if state.auto_scroll {
        let max = state.grouped_view().len().saturating_sub(1);
        state.selected = max;
        state.new_since_pause = 0;
    } else {
        state.new_since_pause = state.new_since_pause.saturating_add(new_matching);
    }
}

#[test]
fn apply_payload_seeds_empty_ring_with_initial_batch() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]);
    assert_eq!(s.ring.len(), 3);
    assert_eq!(s.ring[0].id, 1);
    assert_eq!(s.ring[2].id, 3);
    assert!(s.last_fetched_at.is_some());
}

#[test]
fn apply_payload_dedups_on_id() {
    // Cursor-based dedup is now the cluster ring's job; this shim
    // just filters out ids already in the mirror, matching render
    // expectations.
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    apply_payload_test(&mut s, vec![b(2, "INFO"), b(3, "INFO")]);
    assert_eq!(s.ring.len(), 3);
}

#[test]
fn apply_payload_drops_oldest_at_capacity() {
    let mut s = BulletinsState::with_capacity(4);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]);
    apply_payload_test(&mut s, vec![b(4, "INFO"), b(5, "INFO"), b(6, "INFO")]);
    assert_eq!(s.ring.len(), 4);
    assert_eq!(s.ring.front().unwrap().id, 3);
    assert_eq!(s.ring.back().unwrap().id, 6);
}

#[test]
fn apply_payload_empty_batch_is_noop_except_for_fetched_at() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![]);
    assert!(s.ring.is_empty());
    assert!(s.last_fetched_at.is_some());
}

#[test]
fn redraw_bulletins_mirrors_cluster_ring_into_view_state() {
    use crate::cluster::snapshot::FetchMeta;
    use std::time::Instant;
    let mut state = crate::test_support::fresh_state();
    // Seed the cluster ring with 3 bulletins + meta.
    state
        .cluster
        .snapshot
        .bulletins
        .merge(vec![b(1, "INFO"), b(2, "WARN"), b(3, "ERROR")]);
    state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: crate::test_support::default_fetch_duration(),
        next_interval: Duration::from_secs(5),
    });
    crate::view::bulletins::state::redraw_bulletins(&mut state);
    assert_eq!(state.bulletins.ring.len(), 3);
    assert_eq!(state.bulletins.ring[0].id, 1);
    assert_eq!(state.bulletins.ring[2].id, 3);
    assert!(state.bulletins.last_fetched_at.is_some());
}

#[test]
fn redraw_bulletins_advances_new_since_pause_when_paused() {
    use crate::cluster::snapshot::FetchMeta;
    use std::time::Instant;
    let mut state = crate::test_support::fresh_state();
    state.bulletins.auto_scroll = false;
    // First mirror: 2 bulletins.
    state
        .cluster
        .snapshot
        .bulletins
        .merge(vec![b(1, "INFO"), b(2, "INFO")]);
    state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: crate::test_support::default_fetch_duration(),
        next_interval: Duration::from_secs(5),
    });
    crate::view::bulletins::state::redraw_bulletins(&mut state);
    let badge_after_first = state.bulletins.new_since_pause;

    // Second mirror: 2 more bulletins — the badge advances by the
    // newly matching rows only.
    state
        .cluster
        .snapshot
        .bulletins
        .merge(vec![b(3, "INFO"), b(4, "INFO")]);
    crate::view::bulletins::state::redraw_bulletins(&mut state);
    assert!(
        state.bulletins.new_since_pause > badge_after_first,
        "new_since_pause must grow as fresh matching rows arrive"
    );
}

#[test]
fn redraw_bulletins_with_grouping_preserves_render_time_dedup() {
    use crate::cluster::snapshot::FetchMeta;
    use std::time::Instant;
    let mut state = crate::test_support::fresh_state();
    // Three bulletins sharing source+message stem collapse to one
    // grouped row under the default `SourceAndMessage` mode.
    let shared = BulletinSnapshot {
        id: 0,
        level: "ERROR".into(),
        message: "Proc[id=p] same stem".into(),
        source_id: "src-same".into(),
        source_name: "Proc".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-11T10:14:22Z".into(),
        timestamp_human: String::new(),
    };
    let build = |id: i64| BulletinSnapshot {
        id,
        ..shared.clone()
    };
    state
        .cluster
        .snapshot
        .bulletins
        .merge(vec![build(1), build(2), build(3)]);
    state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
        fetched_at: Instant::now(),
        fetch_duration: crate::test_support::default_fetch_duration(),
        next_interval: Duration::from_secs(5),
    });
    crate::view::bulletins::state::redraw_bulletins(&mut state);
    assert_eq!(state.bulletins.ring.len(), 3);
    let groups = state.bulletins.grouped_view();
    assert_eq!(
        groups.len(),
        1,
        "render-time dedup must fold repeating stems into one group"
    );
    assert_eq!(groups[0].count, 3);
}

fn b_full(
    id: i64,
    level: &str,
    source_type: &str,
    source_name: &str,
    message: &str,
) -> BulletinSnapshot {
    BulletinSnapshot {
        id,
        level: level.into(),
        message: message.into(),
        source_id: format!("src-{id}"),
        source_name: source_name.into(),
        source_type: source_type.into(),
        group_id: "root".into(),
        timestamp_iso: "2026-04-11T10:14:22Z".into(),
        timestamp_human: String::new(),
    }
}

fn seed(capacity: usize, rows: Vec<BulletinSnapshot>) -> BulletinsState {
    let mut s = BulletinsState::with_capacity(capacity);
    apply_payload_test(&mut s, rows);
    s
}

#[test]
fn severity_toggle_removes_matching_rows_from_filtered_view() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "INFO", "PROCESSOR", "A", "info msg"),
            b_full(2, "WARN", "PROCESSOR", "B", "warn msg"),
            b_full(3, "ERROR", "PROCESSOR", "C", "error msg"),
        ],
    );
    assert_eq!(s.filtered_indices().len(), 3);
    s.toggle_error();
    assert_eq!(s.filtered_indices().len(), 2);
    assert!(
        s.filtered_indices()
            .iter()
            .all(|&i| s.ring[i].level != "ERROR")
    );
}

#[test]
fn unknown_severity_rides_with_info_chip() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "DEBUG", "PROCESSOR", "A", "unknown-level"),
            b_full(2, "INFO", "PROCESSOR", "B", "info"),
        ],
    );
    assert_eq!(s.filtered_indices().len(), 2);
    s.toggle_info();
    assert_eq!(
        s.filtered_indices().len(),
        0,
        "toggling off Info also hides Unknown-level rows"
    );
}

#[test]
fn component_type_cycle_advances_through_five_states() {
    let mut s = BulletinsState::with_capacity(100);
    assert_eq!(s.filters.component_type, None);
    s.cycle_component_type();
    assert_eq!(s.filters.component_type, Some(ComponentType::Processor));
    s.cycle_component_type();
    assert_eq!(
        s.filters.component_type,
        Some(ComponentType::ControllerService)
    );
    s.cycle_component_type();
    assert_eq!(s.filters.component_type, Some(ComponentType::ReportingTask));
    s.cycle_component_type();
    assert_eq!(s.filters.component_type, Some(ComponentType::Other));
    s.cycle_component_type();
    assert_eq!(s.filters.component_type, None);
}

#[test]
fn component_type_filter_maps_unknown_source_type_to_other() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "INFO", "PROCESSOR", "A", "m"),
            b_full(2, "INFO", "INPUT_PORT", "B", "m"),
            b_full(3, "INFO", "", "C", "m"),
        ],
    );
    s.filters.component_type = Some(ComponentType::Other);
    let filtered = s.filtered_indices();
    assert_eq!(filtered.len(), 2);
    assert!(
        filtered
            .iter()
            .any(|&i| s.ring[i].source_type == "INPUT_PORT")
    );
    assert!(filtered.iter().any(|&i| s.ring[i].source_type.is_empty()));
}

#[test]
fn text_filter_substring_case_insensitive_matches_message() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "ERROR", "PROCESSOR", "A", "IOException thrown"),
            b_full(2, "ERROR", "PROCESSOR", "B", "other failure"),
        ],
    );
    s.filters.text = "ioex".into();
    let filtered = s.filtered_indices();
    assert_eq!(filtered.len(), 1);
    assert_eq!(s.ring[filtered[0]].id, 1);
}

#[test]
fn text_filter_substring_matches_source_name() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "INFO", "PROCESSOR", "PutDatabase", "ok"),
            b_full(2, "INFO", "PROCESSOR", "PutKafka", "ok"),
        ],
    );
    s.filters.text = "kafka".into();
    let filtered = s.filtered_indices();
    assert_eq!(filtered.len(), 1);
    assert_eq!(s.ring[filtered[0]].id, 2);
}

#[test]
fn clear_filters_resets_all_four_dimensions_and_mutes() {
    let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
    s.toggle_error();
    s.toggle_warning();
    s.cycle_component_type();
    s.filters.text = "xyz".into();
    s.mutes.insert("src-1".into());
    s.clear_filters();
    assert!(s.filters.show_error);
    assert!(s.filters.show_warning);
    assert!(s.filters.show_info);
    assert_eq!(s.filters.component_type, None);
    assert_eq!(s.filters.text, "");
    assert!(s.mutes.is_empty());
}

#[test]
fn reconcile_selection_snaps_to_nearest_older_then_newer() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "INFO", "PROCESSOR", "A", "msg-A"),
            b_full(2, "INFO", "PROCESSOR", "B", "msg-B"),
            b_full(3, "ERROR", "PROCESSOR", "C", "msg-C"),
            b_full(4, "INFO", "PROCESSOR", "D", "msg-D"),
            b_full(5, "INFO", "PROCESSOR", "E", "msg-E"),
        ],
    );
    // Selection at filtered index 2 (the ERROR row at ring index 2).
    s.auto_scroll = false;
    s.selected = 2;
    s.toggle_info(); // Hide INFO. Visible ring: [2]. Filtered idx = 0.
    assert_eq!(s.filtered_indices(), vec![2]);
    assert_eq!(s.selected, 0);
    // Restore filters.
    s.clear_filters();
    assert_eq!(s.filtered_indices().len(), 5);
    // Select ring index 3 (the D row).
    s.selected = 3;
    s.auto_scroll = false;
    // Apply a text filter that matches only "B". `toggle_*` helpers
    // capture prev automatically; to simulate that for the direct
    // text assignment we call reconcile_selection with an explicit
    // prior ring index.
    let prev = s.selected_ring_index();
    s.filters.text = "B".into();
    s.reconcile_selection(prev);
    assert_eq!(s.filtered_indices(), vec![1]);
    assert_eq!(s.selected, 0);
}

#[test]
fn reconcile_selection_handles_empty_filtered_list() {
    let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
    s.selected = 0;
    s.auto_scroll = true;
    s.filters.text = "nomatch".into();
    s.reconcile_selection(None);
    assert_eq!(s.filtered_indices().len(), 0);
    assert_eq!(s.selected, 0);
    assert!(s.auto_scroll, "auto_scroll unchanged when empty");
}

#[test]
fn entering_text_input_mode_routes_keys_into_buffer() {
    let mut s = BulletinsState::with_capacity(100);
    s.enter_text_input_mode();
    assert!(s.text_input.is_some());
    s.push_text_input('f', None);
    s.push_text_input('o', None);
    s.push_text_input('o', None);
    assert_eq!(s.text_input.as_deref(), Some("foo"));
    s.pop_text_input(None);
    assert_eq!(s.text_input.as_deref(), Some("fo"));
}

#[test]
fn enter_commits_text_input_and_updates_filter() {
    let mut s = BulletinsState::with_capacity(100);
    s.enter_text_input_mode();
    s.push_text_input('I', None);
    s.push_text_input('O', None);
    s.commit_text_input(None);
    assert!(s.text_input.is_none());
    assert_eq!(s.filters.text, "IO");
}

#[test]
fn escape_cancels_text_input_without_committing() {
    let mut s = BulletinsState::with_capacity(100);
    s.filters.text = "keep".into();
    s.enter_text_input_mode();
    s.push_text_input('x', None);
    s.cancel_text_input(None);
    assert!(s.text_input.is_none());
    assert_eq!(s.filters.text, "keep");
}

#[test]
fn auto_scroll_on_keeps_selection_at_bottom() {
    let mut s = BulletinsState::with_capacity(100);
    s.auto_scroll = true;
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    assert_eq!(s.selected, 1);
    apply_payload_test(&mut s, vec![b(3, "INFO"), b(4, "INFO")]);
    assert_eq!(s.selected, 3);
}

#[test]
fn auto_scroll_off_counts_new_since_pause() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    s.auto_scroll = false;
    s.selected = 0;
    apply_payload_test(&mut s, vec![b(3, "INFO"), b(4, "INFO")]);
    assert_eq!(s.new_since_pause, 2);
    assert_eq!(s.selected, 0);
}

#[test]
fn auto_scroll_off_ignores_non_matching_for_badge() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
    s.auto_scroll = false;
    s.toggle_info();
    s.toggle_warning();
    // Only ERROR is visible now.
    apply_payload_test(&mut s, vec![b_full(2, "INFO", "PROCESSOR", "B", "m")]);
    assert_eq!(s.new_since_pause, 0);
}

#[test]
fn g_and_end_resume_auto_scroll_and_clear_badge() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    s.auto_scroll = false;
    s.new_since_pause = 7;
    s.selected = 0;
    s.goto_newest();
    assert!(s.auto_scroll);
    assert_eq!(s.new_since_pause, 0);
    assert_eq!(s.selected, 1);
}

#[test]
fn p_toggles_auto_scroll_without_goto() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    s.selected = 0;
    s.auto_scroll = true;
    s.toggle_pause();
    assert!(!s.auto_scroll);
    assert_eq!(s.selected, 0);
    s.toggle_pause();
    assert!(s.auto_scroll);
}

#[test]
fn upward_navigation_pauses_auto_scroll() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO"), b(3, "INFO")]);
    assert_eq!(s.selected, 2);
    assert!(s.auto_scroll);
    s.move_selection_up();
    assert_eq!(s.selected, 1);
    assert!(!s.auto_scroll);
}

#[test]
fn move_selection_down_on_empty_filtered_list_is_noop() {
    // Paused, `+N new` showing, empty filtered view — pressing down
    // must not silently resume auto-scroll or clear the badge.
    let mut s = seed(100, vec![b_full(1, "INFO", "PROCESSOR", "A", "m")]);
    s.auto_scroll = false;
    s.new_since_pause = 5;
    s.filters.text = "nomatch".into();
    s.reconcile_selection(None);
    assert!(s.filtered_indices().is_empty());
    s.move_selection_down();
    assert!(!s.auto_scroll, "auto_scroll must stay paused");
    assert_eq!(s.new_since_pause, 5, "badge count must not be cleared");
}

#[test]
fn grouped_view_returns_empty_when_ring_empty() {
    let s = BulletinsState::with_capacity(10);
    assert!(s.grouped_view().is_empty());
}

#[test]
fn grouped_view_no_consecutive_duplicates() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "INFO", "PROCESSOR", "A", "m"),
            b_full(2, "INFO", "PROCESSOR", "B", "m"),
            b_full(3, "INFO", "PROCESSOR", "C", "m"),
        ],
    );
    s.group_mode = GroupMode::Source;
    // Each bulletin has a distinct source_id via `src-{id}` — no grouping.
    let out = s.grouped_view();
    assert_eq!(out.len(), 3);
    assert!(out.iter().all(|g| g.count == 1));
    assert_eq!(out[0].first_ring_idx, 0);
    assert_eq!(out[0].latest_ring_idx, 0);
    assert_eq!(out[2].first_ring_idx, 2);
}

#[test]
fn grouped_view_collapses_same_source_run() {
    // Build a seed with three bulletins sharing source_id "src-same".
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(
        &mut s,
        vec![
            BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "first".into(),
                source_id: "src-same".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 2,
                level: "ERROR".into(),
                message: "second".into(),
                source_id: "src-same".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:23Z".into(),
                timestamp_human: String::new(),
            },
            BulletinSnapshot {
                id: 3,
                level: "ERROR".into(),
                message: "third".into(),
                source_id: "src-same".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:24Z".into(),
                timestamp_human: String::new(),
            },
        ],
    );
    s.group_mode = GroupMode::Source;
    let out = s.grouped_view();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].count, 3);
    assert_eq!(out[0].first_ring_idx, 0);
    assert_eq!(out[0].latest_ring_idx, 2);
}

#[test]
fn grouped_view_interleaved_folds_non_consecutive() {
    // A, B, A pattern — non-consecutive dedup folds into 2 groups:
    // src-a (ring_idx 0 and 2) and src-b (ring_idx 1).
    let mut s = BulletinsState::with_capacity(100);
    let mk = |id: i64, src: &str| BulletinSnapshot {
        id,
        level: "ERROR".into(),
        message: "m".into(),
        source_id: src.into(),
        source_name: src.into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: "2026-04-11T10:14:22Z".into(),
        timestamp_human: String::new(),
    };
    apply_payload_test(&mut s, vec![mk(1, "src-a"), mk(2, "src-b"), mk(3, "src-a")]);
    s.group_mode = GroupMode::Source;
    let out = s.grouped_view();
    // Non-consecutive dedup: src-a (ring_idx 0+2) folds into one group.
    assert_eq!(out.len(), 2);
    assert_eq!(out[0].count, 2);
    assert_eq!(out[0].first_ring_idx, 0);
    assert_eq!(out[0].latest_ring_idx, 2);
    assert_eq!(out[1].count, 1);
    assert_eq!(out[1].first_ring_idx, 1);
}

#[test]
fn grouped_view_respects_filters() {
    // ERROR + INFO + ERROR all with same source_id. Toggling INFO off
    // should collapse the two ERRORs into a single group (they are
    // consecutive in the filtered list).
    let mut s = BulletinsState::with_capacity(100);
    let mk = |id: i64, level: &str| BulletinSnapshot {
        id,
        level: level.into(),
        message: format!("msg-{id}"),
        source_id: "src-same".into(),
        source_name: "P".into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: "2026-04-11T10:14:22Z".into(),
        timestamp_human: String::new(),
    };
    apply_payload_test(&mut s, vec![mk(1, "ERROR"), mk(2, "INFO"), mk(3, "ERROR")]);
    s.group_mode = GroupMode::Source;
    // All three share source_id; grouping folds them to one.
    assert_eq!(s.grouped_view().len(), 1);
    // Toggle INFO off — still one group because filtered list is
    // ring[0] and ring[2], both same source_id.
    s.toggle_info();
    let out = s.grouped_view();
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].count, 2);
    assert_eq!(out[0].first_ring_idx, 0);
    assert_eq!(out[0].latest_ring_idx, 2);
}

fn seed_grouping_fixture() -> BulletinsState {
    // Ring layout by ring_idx and source_id:
    //   0: src-a   (count group #0)
    //   1: src-a   (count group #0)
    //   2: src-b   (count group #1)
    //   3: src-c   (count group #2)
    //   4: src-c   (count group #2)
    //   5: src-c   (count group #2)
    let mut s = BulletinsState::with_capacity(100);
    let mk = |id: i64, src: &str| BulletinSnapshot {
        id,
        level: "ERROR".into(),
        message: format!("msg-{id}"),
        source_id: src.into(),
        source_name: src.into(),
        source_type: "PROCESSOR".into(),
        group_id: "root".into(),
        timestamp_iso: format!("2026-04-11T10:14:{:02}Z", id),
        timestamp_human: String::new(),
    };
    apply_payload_test(
        &mut s,
        vec![
            mk(1, "src-a"),
            mk(2, "src-a"),
            mk(3, "src-b"),
            mk(4, "src-c"),
            mk(5, "src-c"),
            mk(6, "src-c"),
        ],
    );
    s.auto_scroll = false;
    s
}

#[test]
fn cycle_group_mode_on_preserves_selection_to_enclosing_group() {
    let mut s = seed_grouping_fixture();
    // Off mode: 6 visible rows. Select ring_idx 4 (second "src-c", msg-5).
    s.group_mode = GroupMode::Off;
    assert_eq!(s.grouped_view().len(), 6);
    s.selected = 4; // Points at ring_idx 4 in flat mode.
    assert_eq!(s.selected_ring_index(), Some(4));

    s.cycle_group_mode();

    // SourceAndMessage mode: fixture messages are msg-1..msg-6 (all distinct),
    // so each (source_id, message) pair is its own group → 6 groups.
    // Selection reconciles to the group whose latest_ring_idx == 4 (singleton),
    // which is at position 4.
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    assert_eq!(s.grouped_view().len(), 6);
    assert_eq!(s.selected, 4);
    assert_eq!(s.selected_ring_index(), Some(4));
}

#[test]
fn cycle_group_mode_off_preserves_selection_to_latest_bulletin() {
    let mut s = seed_grouping_fixture();
    s.group_mode = GroupMode::Source;
    // Grouped mode: 3 visible groups. Select group #2 (src-c run).
    s.selected = 2;
    assert_eq!(s.selected_ring_index(), Some(5));

    s.cycle_group_mode();

    // Off mode: 6 visible rows. Selection should land on the latest
    // bulletin of the previously-selected group — ring_idx 5, flat
    // position 5.
    assert_eq!(s.group_mode, GroupMode::Off);
    assert_eq!(s.selected, 5);
    assert_eq!(s.selected_ring_index(), Some(5));
}

#[test]
fn cycle_group_mode_from_first_row_stays_on_first_row() {
    let mut s = seed_grouping_fixture();
    s.group_mode = GroupMode::Off;
    s.selected = 0; // src-a flat ring_idx 0
    s.cycle_group_mode();
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    // Fixture messages are all distinct, so ring_idx 0 is its own group
    // at position 0; selection stays at 0.
    assert_eq!(s.selected, 0);
}

#[test]
fn cycle_group_mode_with_no_visible_rows_is_safe() {
    let mut s = BulletinsState::with_capacity(100);
    // Empty ring — nothing to preserve.
    s.cycle_group_mode();
    assert_eq!(s.group_mode, GroupMode::Source);
    assert_eq!(s.selected, 0);
}

#[test]
fn strip_component_prefix_strips_standard_nifi_format() {
    let msg = "UpdateRecord[id=85cecfc6-019d-1000-ffff-ffffe8c7c778] field 'customer.id' missing in input record";
    assert_eq!(
        strip_component_prefix(msg),
        "field 'customer.id' missing in input record"
    );
}

#[test]
fn strip_component_prefix_returns_original_when_prefix_absent() {
    let msg = "plain message with no brackets";
    assert_eq!(
        strip_component_prefix(msg),
        "plain message with no brackets"
    );
}

#[test]
fn strip_component_prefix_handles_name_with_spaces() {
    let msg = "Route On Attribute[id=aaaaaaaa-1111-2222-3333-444444444444] routed to failure";
    assert_eq!(strip_component_prefix(msg), "routed to failure");
}

#[test]
fn strip_component_prefix_returns_original_when_id_bracket_missing() {
    let msg = "Garbled[no-id-here] still garbled";
    assert_eq!(
        strip_component_prefix(msg),
        "Garbled[no-id-here] still garbled"
    );
}

#[test]
fn strip_component_prefix_returns_original_when_no_trailing_space() {
    // Malformed: no space after the closing bracket.
    let msg = "Proc[id=aaaaaaaa-1111-2222-3333-444444444444]no-space";
    assert_eq!(
        strip_component_prefix(msg),
        "Proc[id=aaaaaaaa-1111-2222-3333-444444444444]no-space"
    );
}

#[test]
fn strip_component_prefix_is_idempotent_on_already_clean_message() {
    let msg = "already clean";
    assert_eq!(strip_component_prefix(msg), "already clean");
}

#[test]
fn normalize_dynamic_brackets_replaces_single_bracket_region() {
    assert_eq!(
        normalize_dynamic_brackets("Failed to process FlowFile[filename=abc.txt]"),
        "Failed to process FlowFile[\u{2026}]"
    );
}

#[test]
fn normalize_dynamic_brackets_replaces_multiple_bracket_regions() {
    let input = "a FlowFile[id=x] and StandardFlowFileRecord[uuid=y]";
    assert_eq!(
        normalize_dynamic_brackets(input),
        "a FlowFile[\u{2026}] and StandardFlowFileRecord[\u{2026}]"
    );
}

#[test]
fn normalize_dynamic_brackets_handles_nested_braces_inside_bracket() {
    let input = "StandardFlowFileRecord[uuid=abc, attributes={k=v, k2=v2}]";
    assert_eq!(
        normalize_dynamic_brackets(input),
        "StandardFlowFileRecord[\u{2026}]"
    );
}

#[test]
fn normalize_dynamic_brackets_returns_unchanged_when_no_brackets() {
    assert_eq!(
        normalize_dynamic_brackets("will route to failure"),
        "will route to failure"
    );
}

#[test]
fn normalize_dynamic_brackets_returns_unchanged_on_unclosed_bracket() {
    assert_eq!(
        normalize_dynamic_brackets("something [unclosed but no close"),
        "something [unclosed but no close"
    );
}

#[test]
fn normalize_dynamic_brackets_handles_empty_string() {
    assert_eq!(normalize_dynamic_brackets(""), "");
}

#[test]
fn normalize_dynamic_brackets_handles_bracket_at_end() {
    assert_eq!(
        normalize_dynamic_brackets("prefix [suffix]"),
        "prefix [\u{2026}]"
    );
}

#[test]
fn normalize_dynamic_brackets_preserves_text_between_brackets() {
    let input = "before FlowFile[a=1]; middle StandardFlowFileRecord[b=2]; after";
    assert_eq!(
        normalize_dynamic_brackets(input),
        "before FlowFile[\u{2026}]; middle StandardFlowFileRecord[\u{2026}]; after"
    );
}

#[test]
fn grouped_view_collapses_across_flowfile_attrs() {
    // Reproduces the user-reported bug: two bulletins from the same
    // source with the same message shape but different embedded
    // flowfile attributes should collapse into a single grouped row.
    use crate::client::BulletinSnapshot;
    let mut s = BulletinsState::with_capacity(100);
    s.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "ERROR".into(),
        message: "UpdateRecord[id=pid] Failed to process FlowFile[filename=a.txt]; \
                  will route to failure"
            .into(),
        source_id: "pid".into(),
        source_name: "UpdateRecord".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g1".into(),
        timestamp_iso: "2026-04-14T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    s.ring.push_back(BulletinSnapshot {
        id: 2,
        level: "ERROR".into(),
        message: "UpdateRecord[id=pid] Failed to process FlowFile[filename=b.txt]; \
                  will route to failure"
            .into(),
        source_id: "pid".into(),
        source_name: "UpdateRecord".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g1".into(),
        timestamp_iso: "2026-04-14T10:00:01Z".into(),
        timestamp_human: String::new(),
    });
    let rows = s.grouped_view();
    assert_eq!(
        rows.len(),
        1,
        "two same-shape bulletins must collapse into one row"
    );
    assert_eq!(rows[0].count, 2, "count must reflect both occurrences");
}

#[test]
fn source_and_message_mode_dedups_identical_stems_across_ring() {
    let mut s = BulletinsState::with_capacity(100);
    // Three bulletins from src-1 with an identical stripped stem
    // interleaved with one bulletin from src-2.
    let prefix_a = "ProcA[id=aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa] ";
    let prefix_b = "ProcB[id=bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb] ";
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: format!("{prefix_a}same stem"),
            source_id: "src-1".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: format!("{prefix_b}other stem"),
            source_id: "src-2".into(),
            source_name: "ProcB".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:23Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 3,
            level: "ERROR".into(),
            message: format!("{prefix_a}same stem"),
            source_id: "src-1".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:24Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 4,
            level: "ERROR".into(),
            message: format!("{prefix_a}same stem"),
            source_id: "src-1".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:25Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    let rows = s.grouped_view();
    // Two unique groups: src-1/"same stem" ×3, src-2/"other stem" ×1.
    assert_eq!(rows.len(), 2);
    // Stable ordering: groups appear in order of first-seen ring index.
    assert_eq!(rows[0].count, 3, "src-1 group should fold 3 bulletins");
    assert_eq!(rows[0].first_ring_idx, 0);
    assert_eq!(rows[0].latest_ring_idx, 3);
    assert_eq!(rows[1].count, 1);
    assert_eq!(rows[1].first_ring_idx, 1);
    assert_eq!(rows[1].latest_ring_idx, 1);
}

#[test]
fn source_mode_folds_all_messages_from_one_source() {
    let mut s = BulletinsState::with_capacity(100);
    s.group_mode = GroupMode::Source;
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "ProcA[id=a] msg one".into(),
            source_id: "src-1".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "WARN".into(),
            message: "ProcA[id=a] msg two".into(),
            source_id: "src-1".into(),
            source_name: "ProcA".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g1".into(),
            timestamp_iso: "2026-04-11T10:14:23Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    let rows = s.grouped_view();
    assert_eq!(rows.len(), 1, "Source mode collapses different stems");
    assert_eq!(rows[0].count, 2);
}

#[test]
fn off_mode_emits_one_row_per_bulletin() {
    let mut s = BulletinsState::with_capacity(100);
    s.group_mode = GroupMode::Off;
    let rows = vec![
        b(1, "INFO"),
        b(2, "INFO"), // note: `b()` test helper gives different source_ids
    ];
    apply_payload_test(&mut s, rows);
    let rows = s.grouped_view();
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|g| g.count == 1));
}

#[test]
fn dedup_is_non_consecutive() {
    // Regression guard vs the old consecutive-only grouping.
    let mut s = BulletinsState::with_capacity(100);
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "P[id=a] boom".into(),
            source_id: "src-1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:14:22Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: "Q[id=b] boom".into(),
            source_id: "src-2".into(),
            source_name: "Q".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:14:23Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 3,
            level: "ERROR".into(),
            message: "P[id=a] boom".into(),
            source_id: "src-1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:14:24Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    let rows = s.grouped_view();
    // Old code would have produced 3 groups (P, Q, P). New dedup
    // collapses P across the interruption → 2 groups.
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].count, 2, "P rows fold across the Q interruption");
    assert_eq!(rows[1].count, 1);
}

#[test]
fn group_mode_default_is_source_and_message() {
    let s = BulletinsState::with_capacity(100);
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
}

#[test]
fn cycle_group_mode_walks_source_and_message_then_source_then_off() {
    let mut s = BulletinsState::with_capacity(100);
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
    s.cycle_group_mode();
    assert_eq!(s.group_mode, GroupMode::Source);
    s.cycle_group_mode();
    assert_eq!(s.group_mode, GroupMode::Off);
    s.cycle_group_mode();
    assert_eq!(s.group_mode, GroupMode::SourceAndMessage);
}

#[test]
fn mute_selected_source_hides_matching_rows() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
            b_full(2, "ERROR", "PROCESSOR", "B", "ProcB[id=b] crash"),
            b_full(3, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
        ],
    );
    // Use Off mode to see all rows individually.
    s.group_mode = GroupMode::Off;
    // Select the first row (src-1) and mute it.
    s.selected = 0;
    s.mute_selected_source();
    let rows = s.grouped_view();
    // src-1 is filtered out, leaving src-2 and src-3 (2 rows in Off mode).
    assert_eq!(rows.len(), 2);
    assert_eq!(s.ring[rows[0].latest_ring_idx].source_id, "src-2");
    assert_eq!(s.ring[rows[1].latest_ring_idx].source_id, "src-3");
    // Selection must have snapped forward to the surviving row.
    assert_eq!(s.selected, 0);
}

#[test]
fn mute_selected_source_is_a_toggle_on_repress() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "ERROR", "PROCESSOR", "A", "ProcA[id=a] boom"),
            b_full(2, "ERROR", "PROCESSOR", "B", "ProcB[id=b] crash"),
        ],
    );
    s.selected = 0;
    s.mute_selected_source();
    assert_eq!(s.grouped_view().len(), 1);
    // Navigate back onto src-1 is impossible while muted. Unmute by
    // API — the handler-side toggle path is covered separately.
    s.mutes.remove("src-1");
    assert_eq!(s.grouped_view().len(), 2);
}

#[test]
fn mute_noop_when_no_selection() {
    let mut s = BulletinsState::with_capacity(100);
    // Empty ring → no selection.
    s.mute_selected_source();
    assert!(s.mutes.is_empty());
}

#[test]
fn severity_counts_returns_raw_ring_totals_ignoring_other_filters() {
    let mut s = seed(
        100,
        vec![
            b_full(1, "ERROR", "PROCESSOR", "A", "a"),
            b_full(2, "ERROR", "PROCESSOR", "A", "a"),
            b_full(3, "WARN", "PROCESSOR", "B", "b"),
            b_full(4, "INFO", "PROCESSOR", "C", "c"),
        ],
    );
    // Apply a text filter — should NOT affect severity counts.
    s.filters.text = "zzz".into();
    let counts = s.severity_counts();
    assert_eq!(counts.error, 2);
    assert_eq!(counts.warning, 1);
    assert_eq!(counts.info, 1);
}

#[test]
fn group_details_returns_first_and_last_seen_for_dedup_group() {
    let mut s = BulletinsState::with_capacity(100);
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "P[id=a] same stem".into(),
            source_id: "src-1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:00:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: "P[id=a] same stem".into(),
            source_id: "src-1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:05:00Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    s.selected = 0;
    let d = s.group_details().expect("group exists");
    assert_eq!(d.count, 2);
    assert_eq!(d.first_seen_iso, "2026-04-11T10:00:00Z");
    assert_eq!(d.last_seen_iso, "2026-04-11T10:05:00Z");
    assert_eq!(d.source_id, "src-1");
    assert_eq!(d.group_id, "g");
    assert_eq!(d.stripped_message, "same stem");
    assert_eq!(d.raw_message, "P[id=a] same stem");
}

#[test]
fn recent_for_source_id_returns_newest_first_up_to_limit() {
    let mut s = BulletinsState::with_capacity(100);
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "INFO".into(),
            message: "a".into(),
            source_id: "p1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:00:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: "b".into(),
            source_id: "p2".into(),
            source_name: "Q".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:01:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 3,
            level: "WARN".into(),
            message: "c".into(),
            source_id: "p1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:02:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 4,
            level: "WARN".into(),
            message: "d".into(),
            source_id: "p1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-11T10:03:00Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    let hits = recent_for_source_id(&s.ring, "p1", 2);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, 4, "newest first");
    assert_eq!(hits[1].id, 3);
}

#[test]
fn recent_for_source_id_limit_zero_returns_empty() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO")]);
    let hits = recent_for_source_id(&s.ring, "src-1", 0);
    assert!(hits.is_empty());
}

#[test]
fn recent_for_source_id_no_match_returns_empty() {
    let mut s = BulletinsState::with_capacity(100);
    apply_payload_test(&mut s, vec![b(1, "INFO"), b(2, "INFO")]);
    let hits = recent_for_source_id(&s.ring, "nonexistent", 10);
    assert!(hits.is_empty());
}

#[test]
fn recent_for_group_id_filters_by_group_id() {
    let mut s = BulletinsState::with_capacity(100);
    let rows = vec![
        BulletinSnapshot {
            id: 1,
            level: "INFO".into(),
            message: "a".into(),
            source_id: "p1".into(),
            source_name: "P".into(),
            source_type: "PROCESSOR".into(),
            group_id: "noisy".into(),
            timestamp_iso: "2026-04-11T10:00:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 2,
            level: "ERROR".into(),
            message: "b".into(),
            source_id: "p2".into(),
            source_name: "Q".into(),
            source_type: "PROCESSOR".into(),
            group_id: "healthy".into(),
            timestamp_iso: "2026-04-11T10:01:00Z".into(),
            timestamp_human: String::new(),
        },
        BulletinSnapshot {
            id: 3,
            level: "WARN".into(),
            message: "c".into(),
            source_id: "p3".into(),
            source_name: "R".into(),
            source_type: "PROCESSOR".into(),
            group_id: "noisy".into(),
            timestamp_iso: "2026-04-11T10:02:00Z".into(),
            timestamp_human: String::new(),
        },
    ];
    apply_payload_test(&mut s, rows);
    let hits = recent_for_group_id(&s.ring, "noisy", 10);
    assert_eq!(hits.len(), 2);
    assert_eq!(hits[0].id, 3, "newest first");
    assert_eq!(hits[1].id, 1);
}

#[test]
fn detail_modal_state_default_is_empty() {
    let state = BulletinsState::with_capacity(10);
    assert!(state.detail_modal.is_none());
}

#[test]
fn group_key_equality_ignores_case_of_source_id_is_exact() {
    // Sanity: GroupKey uses exact string equality for source_id + message_stem.
    let a = GroupKey {
        source_id: "abc".into(),
        message_stem: "boom".into(),
        mode: GroupMode::SourceAndMessage,
    };
    let b = a.clone();
    assert_eq!(a, b);
}

#[test]
fn open_detail_modal_captures_group_key_and_details() {
    let mut state = BulletinsState::with_capacity(10);
    // Seed a single bulletin.
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "ERROR".into(),
        message: "PutDb[id=abc] boom".into(),
        source_id: "src-1".into(),
        source_name: "PutDb".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.auto_scroll = false;

    assert!(state.open_detail_modal());
    let modal = state.detail_modal.as_ref().expect("modal open");
    assert_eq!(modal.group_key.source_id, "src-1");
    assert_eq!(modal.details.raw_message, "PutDb[id=abc] boom");
    assert_eq!(modal.scroll.offset, 0);
    assert!(modal.search.is_none());
}

#[test]
fn open_detail_modal_noops_on_empty_list() {
    let mut state = BulletinsState::with_capacity(10);
    assert!(!state.open_detail_modal());
    assert!(state.detail_modal.is_none());
}

#[test]
fn close_detail_modal_clears_state() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "msg".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.auto_scroll = false;
    state.open_detail_modal();
    assert!(state.detail_modal.is_some());

    state.close_detail_modal();
    assert!(state.detail_modal.is_none());
}

#[test]
fn modal_scroll_down_advances_offset() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "a".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_scroll_by(1);
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 1);
    state.modal_scroll_by(3);
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 4);
}

#[test]
fn modal_scroll_up_clamps_at_zero() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "a".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_scroll_by(-5);
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 0);
}

#[test]
fn modal_page_scroll_uses_last_viewport_rows() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "a".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    // Simulate a render having measured viewport_rows = 10.
    state
        .detail_modal
        .as_mut()
        .unwrap()
        .scroll
        .last_viewport_rows = 10;
    state.modal_page_down();
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 10);
    state.modal_page_up();
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 0);
}

#[test]
fn modal_jump_top_and_bottom() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "a".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.detail_modal.as_mut().unwrap().scroll.offset = 5;
    state.modal_jump_top();
    assert_eq!(state.detail_modal.as_ref().unwrap().scroll.offset, 0);

    // `modal_jump_bottom` sets offset to usize::MAX; renderer clamps
    // against real max. State-level test only verifies the sentinel.
    state.modal_jump_bottom();
    assert_eq!(
        state.detail_modal.as_ref().unwrap().scroll.offset,
        usize::MAX
    );
}

#[test]
fn modal_copy_message_returns_full_raw_message() {
    let mut state = BulletinsState::with_capacity(10);
    let long = "line one\nline two with a long tail of text".to_string();
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "ERROR".into(),
        message: long.clone(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    let msg = state.modal_copy_message().expect("modal open");
    assert_eq!(msg, long);
}

#[test]
fn modal_copy_message_none_when_closed() {
    let state = BulletinsState::with_capacity(10);
    assert!(state.modal_copy_message().is_none());
}

#[test]
fn modal_search_open_flips_input_active() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "error here".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_search_open();
    let s = state
        .detail_modal
        .as_ref()
        .unwrap()
        .search
        .as_ref()
        .unwrap();
    assert!(s.input_active);
    assert_eq!(s.query, "");
}

#[test]
fn modal_search_push_updates_matches_live() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "error here".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_search_open();
    state.modal_search_push('e');
    state.modal_search_push('r');
    state.modal_search_push('r');
    let s = state
        .detail_modal
        .as_ref()
        .unwrap()
        .search
        .as_ref()
        .unwrap();
    assert_eq!(s.query, "err");
    assert_eq!(s.matches.len(), 1);
    assert_eq!(s.current, Some(0));
}

#[test]
fn modal_search_commit_flips_committed_and_keeps_matches() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "aaa bbb aaa".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_search_open();
    for c in "aaa".chars() {
        state.modal_search_push(c);
    }
    state.modal_search_commit();
    let s = state
        .detail_modal
        .as_ref()
        .unwrap()
        .search
        .as_ref()
        .unwrap();
    assert!(s.committed);
    assert!(!s.input_active);
    assert_eq!(s.matches.len(), 2);
}

#[test]
fn modal_search_cancel_clears_query_and_matches() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "aaa".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_search_open();
    for c in "aaa".chars() {
        state.modal_search_push(c);
    }
    state.modal_search_cancel();
    assert!(state.detail_modal.as_ref().unwrap().search.is_none());
}

#[test]
fn modal_search_cycle_wraps_both_directions() {
    let mut state = BulletinsState::with_capacity(10);
    state.ring.push_back(BulletinSnapshot {
        id: 1,
        level: "INFO".into(),
        message: "a x a x a".into(),
        source_id: "s".into(),
        source_name: "S".into(),
        source_type: "PROCESSOR".into(),
        group_id: "g".into(),
        timestamp_iso: "2026-04-20T10:00:00Z".into(),
        timestamp_human: String::new(),
    });
    state.selected = 0;
    state.open_detail_modal();
    state.modal_search_open();
    state.modal_search_push('a');
    state.modal_search_commit();
    // current starts at 0, there are 3 matches
    state.modal_search_cycle_next();
    assert_eq!(
        state
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current,
        Some(1)
    );
    state.modal_search_cycle_next();
    assert_eq!(
        state
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current,
        Some(2)
    );
    state.modal_search_cycle_next();
    assert_eq!(
        state
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current,
        Some(0)
    ); // wrap

    state.modal_search_cycle_prev();
    assert_eq!(
        state
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current,
        Some(2)
    ); // wrap
}
