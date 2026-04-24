use super::*;

use crate::client::{LatestEventsSnapshot, ProvenanceEventDetail, ProvenanceEventSummary};

#[test]
fn default_state_is_entry_empty() {
    let state = TracerState::new();
    assert!(matches!(state.mode, TracerMode::Entry(ref e) if e.input.is_empty()));
    assert!(state.last_error.is_none());
}

#[test]
fn entry_typing_accumulates() {
    let mut state = TracerState::new();
    handle_entry_char(&mut state, 'a');
    handle_entry_char(&mut state, 'b');
    handle_entry_char(&mut state, 'c');
    let TracerMode::Entry(EntryState { input }) = &state.mode else {
        panic!("expected Entry mode");
    };
    assert_eq!(input, "abc");
}

#[test]
fn entry_backspace_removes_last_char() {
    let mut state = TracerState::new();
    handle_entry_char(&mut state, 'a');
    handle_entry_char(&mut state, 'b');
    handle_entry_backspace(&mut state);
    let TracerMode::Entry(EntryState { input }) = &state.mode else {
        panic!("expected Entry mode");
    };
    assert_eq!(input, "a");
}

#[test]
fn entry_ctrl_u_clears() {
    let mut state = TracerState::new();
    handle_entry_char(&mut state, 'a');
    handle_entry_char(&mut state, 'b');
    handle_entry_clear(&mut state);
    let TracerMode::Entry(EntryState { input }) = &state.mode else {
        panic!("expected Entry mode");
    };
    assert!(input.is_empty());
}

#[test]
fn entry_submit_valid_uuid_returns_validated() {
    let mut state = TracerState::new();
    for ch in "7a2e8b9c-1234-4abc-9def-0123456789ab".chars() {
        handle_entry_char(&mut state, ch);
    }
    let result = entry_submit(&mut state);
    assert_eq!(
        result.as_deref(),
        Some("7a2e8b9c-1234-4abc-9def-0123456789ab")
    );
    assert!(state.last_error.is_none());
}

#[test]
fn entry_submit_invalid_uuid_sets_banner_and_returns_none() {
    let mut state = TracerState::new();
    for ch in "not-a-uuid".chars() {
        handle_entry_char(&mut state, ch);
    }
    let result = entry_submit(&mut state);
    assert!(result.is_none());
    assert_eq!(
        state.last_error.as_deref(),
        Some("invalid UUID: expected 8-4-4-4-12 hex")
    );
}

#[test]
fn attribute_diff_mode_toggles() {
    let mode = AttributeDiffMode::All;
    let toggled = mode.toggle();
    assert_eq!(toggled, AttributeDiffMode::Changed);
    assert_eq!(toggled.toggle(), AttributeDiffMode::All);
}

// ── LineageRunning reducer tests ────────────────────────────────────────

const TEST_UUID: &str = "7a2e8b9c-1234-4abc-9def-0123456789ab";

#[test]
fn start_lineage_transitions_entry_to_running_with_empty_query_id() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    let TracerMode::LineageRunning(ref running) = state.mode else {
        panic!("expected LineageRunning mode");
    };
    assert_eq!(running.uuid, TEST_UUID);
    assert!(running.query_id.is_empty());
    assert_eq!(running.percent, 0);
    assert!(running.abort.is_none());
    assert!(state.last_error.is_none());
}

#[test]
fn lineage_submitted_fills_query_id_when_uuid_matches() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);

    let followup = apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );
    assert!(followup.is_none());

    let TracerMode::LineageRunning(ref running) = state.mode else {
        panic!("expected LineageRunning mode");
    };
    assert_eq!(running.query_id, "q-42");
}

#[test]
fn lineage_submitted_stale_uuid_is_dropped() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);

    let followup = apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: "stale-uuid".to_string(),
            query_id: "q-99".to_string(),
            cluster_node_id: None,
        },
    );
    assert!(followup.is_none());

    let TracerMode::LineageRunning(ref running) = state.mode else {
        panic!("expected LineageRunning mode");
    };
    assert!(running.query_id.is_empty(), "query_id should remain empty");
}

#[test]
fn lineage_partial_updates_percent_when_query_id_matches() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    let followup = apply_payload(
        &mut state,
        TracerPayload::LineagePartial {
            query_id: "q-42".to_string(),
            percent: 55,
        },
    );
    assert!(followup.is_none());

    let TracerMode::LineageRunning(ref running) = state.mode else {
        panic!("expected LineageRunning mode");
    };
    assert_eq!(running.percent, 55);
}

#[test]
fn lineage_partial_stale_query_id_is_dropped() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    apply_payload(
        &mut state,
        TracerPayload::LineagePartial {
            query_id: "q-stale".to_string(),
            percent: 99,
        },
    );

    let TracerMode::LineageRunning(ref running) = state.mode else {
        panic!("expected LineageRunning mode");
    };
    assert_eq!(running.percent, 0, "percent should stay at 0");
}

#[test]
fn lineage_done_transitions_to_lineage_view() {
    use crate::client::LineageSnapshot;

    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    let snapshot = LineageSnapshot {
        events: vec![],
        percent_completed: 100,
        finished: true,
    };
    let followup = apply_payload(
        &mut state,
        TracerPayload::LineageDone {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            snapshot,
            fetched_at: SystemTime::now(),
        },
    );

    assert!(matches!(state.mode, TracerMode::Lineage(_)));
    assert!(
        matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id, .. }) if query_id == "q-42")
    );
}

#[test]
fn lineage_done_stale_query_id_emits_delete_followup() {
    use crate::client::LineageSnapshot;

    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    let snapshot = LineageSnapshot {
        events: vec![],
        percent_completed: 100,
        finished: true,
    };
    let followup = apply_payload(
        &mut state,
        TracerPayload::LineageDone {
            uuid: TEST_UUID.to_string(),
            query_id: "q-stale".to_string(),
            snapshot,
            fetched_at: SystemTime::now(),
        },
    );

    // State should remain LineageRunning (stale query_id doesn't match)
    assert!(matches!(state.mode, TracerMode::LineageRunning(_)));
    // But we still emit delete to clean up the stale query on the server
    assert!(
        matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id, .. }) if query_id == "q-stale")
    );
}

#[test]
fn lineage_done_before_submitted_still_transitions() {
    use crate::client::LineageSnapshot;

    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    // Do NOT send LineageSubmitted — simulate the race where
    // LineageDone arrives first (query_id on state is still "").
    let snapshot = LineageSnapshot {
        events: vec![],
        percent_completed: 100,
        finished: true,
    };
    let followup = apply_payload(
        &mut state,
        TracerPayload::LineageDone {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            snapshot,
            fetched_at: SystemTime::now(),
        },
    );

    assert!(
        matches!(state.mode, TracerMode::Lineage(_)),
        "LineageDone with empty query_id on state must still transition"
    );
    assert!(
        matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id, .. }) if query_id == "q-42")
    );
}

#[test]
fn lineage_partial_before_submitted_still_updates_percent() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    // Do NOT send LineageSubmitted.
    apply_payload(
        &mut state,
        TracerPayload::LineagePartial {
            query_id: "q-42".to_string(),
            percent: 50,
        },
    );

    if let TracerMode::LineageRunning(ref running) = state.mode {
        assert_eq!(running.percent, 50);
    } else {
        panic!("expected LineageRunning mode");
    }
}

#[test]
fn lineage_failed_returns_to_entry_with_error() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    apply_payload(
        &mut state,
        TracerPayload::LineageFailed {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            error: "server error".to_string(),
        },
    );

    assert!(matches!(state.mode, TracerMode::Entry(_)));
    assert_eq!(state.last_error.as_deref(), Some("server error"));
}

#[test]
fn cancel_lineage_transitions_to_entry_and_emits_delete_when_query_id_known() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);
    apply_payload(
        &mut state,
        TracerPayload::LineageSubmitted {
            uuid: TEST_UUID.to_string(),
            query_id: "q-42".to_string(),
            cluster_node_id: None,
        },
    );

    let followup = cancel_lineage(&mut state);
    assert!(matches!(state.mode, TracerMode::Entry(_)));
    assert!(
        matches!(followup, Some(Followup::DeleteLineageQuery { ref query_id, .. }) if query_id == "q-42")
    );
}

#[test]
fn cancel_lineage_before_submission_does_not_emit_delete() {
    let mut state = TracerState::new();
    start_lineage(&mut state, TEST_UUID.to_string(), None);

    let followup = cancel_lineage(&mut state);
    assert!(matches!(state.mode, TracerMode::Entry(_)));
    assert!(followup.is_none());
}

// ── LatestEvents reducer tests ──────────────────────────────────────────

const COMP_ID: &str = "comp-aaaa-bbbb-cccc-dddddddddddd";

fn fake_summary(id: i64, uuid: &str) -> ProvenanceEventSummary {
    ProvenanceEventSummary {
        event_id: id,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CREATE".to_string(),
        component_id: COMP_ID.to_string(),
        component_name: "MyProcessor".to_string(),
        component_type: "GenerateFlowFile".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: uuid.to_string(),
        relationship: None,
        details: None,
    }
}

#[test]
fn start_latest_events_transitions_into_loading_view() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    let TracerMode::LatestEvents(ref view) = state.mode else {
        panic!("expected LatestEvents mode");
    };
    assert_eq!(view.component_id, COMP_ID);
    assert_eq!(view.component_label, COMP_ID);
    assert!(view.events.is_empty());
    assert_eq!(view.selected, 0);
    assert!(view.loading);
    assert!(state.last_error.is_none());
}

#[test]
fn latest_events_payload_populates_matching_component() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    let snap = LatestEventsSnapshot {
        component_id: COMP_ID.to_string(),
        component_label: "MyProcessor".to_string(),
        events: vec![fake_summary(1, "uuid-1111"), fake_summary(2, "uuid-2222")],
        fetched_at: SystemTime::now(),
    };
    let followup = apply_payload(&mut state, TracerPayload::LatestEvents(snap));
    assert!(followup.is_none());

    let TracerMode::LatestEvents(ref view) = state.mode else {
        panic!("expected LatestEvents mode");
    };
    assert_eq!(view.component_label, "MyProcessor");
    assert_eq!(view.events.len(), 2);
    assert!(!view.loading);
}

#[test]
fn latest_events_payload_with_mismatched_component_is_dropped() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    let snap = LatestEventsSnapshot {
        component_id: "other-component".to_string(),
        component_label: "Other".to_string(),
        events: vec![fake_summary(99, "uuid-9999")],
        fetched_at: SystemTime::now(),
    };
    apply_payload(&mut state, TracerPayload::LatestEvents(snap));

    let TracerMode::LatestEvents(ref view) = state.mode else {
        panic!("expected LatestEvents mode");
    };
    assert!(view.events.is_empty(), "events should remain empty");
    assert!(view.loading, "loading should remain true");
}

#[test]
fn latest_events_j_k_moves_selection_and_wraps() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    // Populate with 3 events via payload
    let snap = LatestEventsSnapshot {
        component_id: COMP_ID.to_string(),
        component_label: "MyProcessor".to_string(),
        events: vec![
            fake_summary(1, "uuid-1111"),
            fake_summary(2, "uuid-2222"),
            fake_summary(3, "uuid-3333"),
        ],
        fetched_at: SystemTime::now(),
    };
    apply_payload(&mut state, TracerPayload::LatestEvents(snap));

    // Move down: 0 → 1 → 2 → wraps to 0
    latest_events_move_down(&mut state);
    assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 1));
    latest_events_move_down(&mut state);
    latest_events_move_down(&mut state); // wraps
    assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 0));

    // Move up from 0 wraps to last (2)
    latest_events_move_up(&mut state);
    assert!(matches!(&state.mode, TracerMode::LatestEvents(v) if v.selected == 2));
}

#[test]
fn latest_events_selected_uuid_returns_row_uuid() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    let snap = LatestEventsSnapshot {
        component_id: COMP_ID.to_string(),
        component_label: "MyProcessor".to_string(),
        events: vec![fake_summary(1, "uuid-1111"), fake_summary(2, "uuid-2222")],
        fetched_at: SystemTime::now(),
    };
    apply_payload(&mut state, TracerPayload::LatestEvents(snap));

    assert_eq!(
        latest_events_selected_uuid(&state).as_deref(),
        Some("uuid-1111")
    );

    latest_events_move_down(&mut state);
    assert_eq!(
        latest_events_selected_uuid(&state).as_deref(),
        Some("uuid-2222")
    );
}

#[test]
fn latest_events_failed_payload_clears_loading_and_sets_banner() {
    let mut state = TracerState::new();
    start_latest_events(&mut state, COMP_ID.to_string());

    let followup = apply_payload(
        &mut state,
        TracerPayload::LatestEventsFailed {
            component_id: COMP_ID.to_string(),
            error: "connection refused".to_string(),
        },
    );
    assert!(followup.is_none());

    let TracerMode::LatestEvents(ref view) = state.mode else {
        panic!("expected LatestEvents mode");
    };
    assert!(!view.loading);
    assert_eq!(state.last_error.as_deref(), Some("connection refused"));
}

// ── Lineage reducer tests ───────────────────────────────────────────────

fn fake_detail(event_id: i64) -> ProvenanceEventDetail {
    ProvenanceEventDetail {
        summary: fake_summary(event_id, "uuid-detail"),
        attributes: vec![],
        transit_uri: None,
        input_available: false,
        output_available: false,
        input_size: None,
        output_size: None,
    }
}

fn seed_lineage(state: &mut TracerState, event_ids: &[i64]) {
    use crate::client::LineageSnapshot;
    let events = event_ids
        .iter()
        .map(|&id| fake_summary(id, &format!("uuid-{id}")))
        .collect();
    state.mode = TracerMode::Lineage(Box::new(LineageView {
        uuid: TEST_UUID.to_string(),
        snapshot: LineageSnapshot {
            events,
            percent_completed: 100,
            finished: true,
        },
        selected_event: 0,
        event_detail: EventDetail::default(),
        loaded_details: std::collections::HashMap::new(),
        diff_mode: AttributeDiffMode::default(),
        fetched_at: SystemTime::now(),
        focus: LineageFocus::default(),
        active_detail_tab: DetailTab::default(),
    }));
}

#[test]
fn lineage_j_k_moves_selection_and_resets_event_detail() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[10, 20, 30]);

    // Load detail on event 0
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.event_detail = EventDetail::Loaded {
            event: Box::new(fake_detail(10)),
            content: ContentPane::default(),
        };
    }
    // Move down — detail should be reset
    lineage_move_down(&mut state);
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.selected_event, 1);
        assert!(matches!(view.event_detail, EventDetail::NotLoaded));
    }

    // Move up back to 0
    lineage_move_up(&mut state);
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.selected_event, 0);
        assert!(matches!(view.event_detail, EventDetail::NotLoaded));
    }

    // Wrap: move up from 0 lands at last (2)
    lineage_move_up(&mut state);
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.selected_event, 2);
    }
}

#[test]
fn lineage_enter_marks_event_detail_loading() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[10, 20]);

    lineage_mark_detail_loading(&mut state);

    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert!(matches!(view.event_detail, EventDetail::Loading));
}

#[test]
fn event_detail_payload_populates_when_event_id_matches_selection() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[42, 99]);

    lineage_mark_detail_loading(&mut state);

    let followup = apply_payload(
        &mut state,
        TracerPayload::EventDetail {
            event_id: 42,
            detail: Box::new(fake_detail(42)),
        },
    );
    assert!(followup.is_none());

    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert!(matches!(
        view.event_detail,
        EventDetail::Loaded { ref event, .. } if event.summary.event_id == 42
    ));
}

#[test]
fn event_detail_payload_stale_event_id_is_dropped() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[42, 99]);

    lineage_mark_detail_loading(&mut state);

    // Deliver detail for event 99 while selection is at 42
    apply_payload(
        &mut state,
        TracerPayload::EventDetail {
            event_id: 99,
            detail: Box::new(fake_detail(99)),
        },
    );

    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    // Still Loading because the event_id didn't match
    assert!(matches!(view.event_detail, EventDetail::Loading));
}

#[test]
fn content_payload_populates_content_pane_when_event_id_matches() {
    use crate::client::{ContentRender, ContentSnapshot};

    let mut state = TracerState::new();
    seed_lineage(&mut state, &[42]);

    // Set up Loaded event detail
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.event_detail = EventDetail::Loaded {
            event: Box::new(fake_detail(42)),
            content: ContentPane::LoadingOutput,
        };
    }

    let snap = ContentSnapshot {
        event_id: 42,
        side: ContentSide::Output,
        render: ContentRender::Text {
            text: "hello".to_string(),
            pretty_printed: false,
        },
        bytes_fetched: 5,
        truncated: false,
    };
    let followup = apply_payload(&mut state, TracerPayload::Content(snap));
    assert!(followup.is_none());

    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert!(matches!(
        view.event_detail,
        EventDetail::Loaded {
            content: ContentPane::Shown {
                side: ContentSide::Output,
                bytes_fetched: 5,
                ..
            },
            ..
        }
    ));
}

// ── LineageFocus + attribute class tests ───────────────────────────────

fn fake_detail_with_attrs(event_id: i64, attrs: Vec<AttributeTriple>) -> ProvenanceEventDetail {
    ProvenanceEventDetail {
        summary: fake_summary(event_id, "uuid-detail"),
        attributes: attrs,
        transit_uri: None,
        input_available: false,
        output_available: false,
        input_size: None,
        output_size: None,
    }
}

fn triple(key: &str, prev: Option<&str>, curr: Option<&str>) -> AttributeTriple {
    AttributeTriple {
        key: key.to_string(),
        previous: prev.map(String::from),
        current: curr.map(String::from),
    }
}

#[test]
fn attribute_class_added_deleted_unchanged() {
    assert_eq!(
        AttributeClass::of(&triple("k", None, Some("v"))),
        AttributeClass::Added
    );
    assert_eq!(
        AttributeClass::of(&triple("k", Some("v"), None)),
        AttributeClass::Deleted
    );
    assert_eq!(
        AttributeClass::of(&triple("k", Some("v"), Some("v"))),
        AttributeClass::Unchanged
    );
    // Modified (both sides present, values differ) → Updated.
    assert_eq!(
        AttributeClass::of(&triple("k", Some("old"), Some("new"))),
        AttributeClass::Updated
    );
}

fn load_detail_with_attrs(state: &mut TracerState, attrs: Vec<AttributeTriple>) {
    let TracerMode::Lineage(ref mut view) = state.mode else {
        panic!("expected Lineage mode");
    };
    let selected_id = view.snapshot.events[view.selected_event].event_id;
    view.event_detail = EventDetail::Loaded {
        event: Box::new(fake_detail_with_attrs(selected_id, attrs)),
        content: ContentPane::default(),
    };
}

#[test]
fn lineage_focus_attributes_rejects_when_detail_not_loaded() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    assert!(!lineage_focus_attributes(&mut state));
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Timeline);
}

#[test]
fn lineage_focus_attributes_rejects_when_visible_list_empty() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    // Loaded but no attributes at all.
    load_detail_with_attrs(&mut state, vec![]);
    assert!(!lineage_focus_attributes(&mut state));

    // Loaded with only unchanged attrs but filter = Changed → empty visible.
    load_detail_with_attrs(&mut state, vec![triple("k", Some("v"), Some("v"))]);
    if let TracerMode::Lineage(ref mut view) = state.mode {
        view.diff_mode = AttributeDiffMode::Changed;
    }
    assert!(!lineage_focus_attributes(&mut state));
}

#[test]
fn lineage_focus_attributes_enters_and_returns_to_timeline() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(
        &mut state,
        vec![
            triple("added", None, Some("new")),
            triple("removed", Some("old"), None),
            triple("kept", Some("v"), Some("v")),
        ],
    );

    assert!(lineage_focus_attributes(&mut state));
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Attributes { row: 0 });

    lineage_focus_timeline(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Timeline);
}

#[test]
fn lineage_attr_move_wraps_and_is_noop_without_focus() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(
        &mut state,
        vec![
            triple("a", None, Some("1")),
            triple("b", None, Some("2")),
            triple("c", None, Some("3")),
        ],
    );
    // No focus yet — attr nav is a no-op.
    lineage_attr_move_down(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Timeline);

    assert!(lineage_focus_attributes(&mut state));
    lineage_attr_move_down(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Attributes { row: 1 });

    lineage_attr_move_down(&mut state);
    lineage_attr_move_down(&mut state); // wraps 2 → 0
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Attributes { row: 0 });

    // Up from 0 wraps to last (2).
    lineage_attr_move_up(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Attributes { row: 2 });
}

#[test]
fn lineage_focused_attribute_value_reads_current_or_previous() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(
        &mut state,
        vec![
            triple("added", None, Some("new-val")),
            triple("removed", Some("old-val"), None),
        ],
    );
    assert!(lineage_focus_attributes(&mut state));
    // Row 0 = added → current
    assert_eq!(
        lineage_focused_attribute_value(&state).as_deref(),
        Some("new-val")
    );
    lineage_attr_move_down(&mut state);
    // Row 1 = removed → previous (because current is None)
    assert_eq!(
        lineage_focused_attribute_value(&state).as_deref(),
        Some("old-val")
    );
}

#[test]
fn lineage_move_selection_resets_focus_and_detail() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1, 2]);
    load_detail_with_attrs(&mut state, vec![triple("k", None, Some("v"))]);
    assert!(lineage_focus_attributes(&mut state));

    lineage_move_down(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Timeline);
    assert!(matches!(view.event_detail, EventDetail::NotLoaded));
}

// ── Content focus tests ─────────────────────────────────────────────────

fn load_content_shown(state: &mut TracerState, body: &str) {
    use crate::client::tracer::{ContentRender, ContentSide};
    let TracerMode::Lineage(ref mut view) = state.mode else {
        panic!("expected Lineage mode");
    };
    if let EventDetail::Loaded {
        ref mut content, ..
    } = view.event_detail
    {
        *content = ContentPane::Shown {
            side: ContentSide::Output,
            render: ContentRender::Text {
                text: body.to_string(),
                pretty_printed: false,
            },
            bytes_fetched: body.len(),
            truncated: false,
        };
    } else {
        panic!("detail must be Loaded before loading content");
    }
}

#[test]
fn lineage_focus_content_rejects_when_not_shown() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    // Still ContentPane::Collapsed by default.
    assert!(!lineage_focus_content(&mut state));
}

#[test]
fn lineage_focus_content_enters_content_and_resets_scroll() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    load_content_shown(&mut state, "a\nb\nc\n");
    assert!(lineage_focus_content(&mut state));
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Content { scroll: 0 });
}

#[test]
fn lineage_content_scroll_down_clamps_at_max() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    load_content_shown(&mut state, "line1\nline2\nline3");
    assert!(lineage_focus_content(&mut state));

    // 3 lines → max scroll is 2 (zero-indexed).
    lineage_content_scroll_down(&mut state, 1);
    lineage_content_scroll_down(&mut state, 1);
    lineage_content_scroll_down(&mut state, 5); // clamps
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Content { scroll: 2 });
}

#[test]
fn lineage_content_scroll_up_saturates_at_zero() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    load_content_shown(&mut state, "a\nb\nc\nd\ne");
    assert!(lineage_focus_content(&mut state));
    lineage_content_scroll_down(&mut state, 3);
    lineage_content_scroll_up(&mut state, 10); // saturates
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Content { scroll: 0 });
}

#[test]
fn lineage_content_scroll_home_end_goto_bounds() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    load_content_shown(&mut state, "a\nb\nc\nd\ne\nf");
    assert!(lineage_focus_content(&mut state));
    lineage_content_scroll_end(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Content { scroll: 5 });

    lineage_content_scroll_home(&mut state);
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Content { scroll: 0 });
}

#[test]
fn new_content_payload_resets_scroll_when_focused() {
    use crate::client::tracer::{ContentRender, ContentSide};

    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    load_detail_with_attrs(&mut state, vec![]);
    load_content_shown(&mut state, "a\nb\nc\nd\ne");
    assert!(lineage_focus_content(&mut state));
    lineage_content_scroll_down(&mut state, 3);

    // New payload arrives (e.g., user pressed `o` to load output).
    let new_body = "x\ny\nz";
    apply_payload(
        &mut state,
        TracerPayload::Content(crate::client::tracer::ContentSnapshot {
            event_id: 1,
            side: ContentSide::Output,
            render: ContentRender::Text {
                text: new_body.to_string(),
                pretty_printed: false,
            },
            bytes_fetched: new_body.len(),
            truncated: false,
        }),
    );
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(
        view.focus,
        LineageFocus::Content { scroll: 0 },
        "scroll must reset on new payload"
    );
}

#[test]
fn event_detail_failed_resets_focus_to_timeline() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);
    // Transition through Loading then failed.
    lineage_mark_detail_loading(&mut state);
    apply_payload(
        &mut state,
        TracerPayload::EventDetailFailed {
            event_id: 1,
            error: "boom".to_string(),
        },
    );
    let TracerMode::Lineage(ref view) = state.mode else {
        panic!("expected Lineage mode");
    };
    assert_eq!(view.focus, LineageFocus::Timeline);
    assert!(matches!(view.event_detail, EventDetail::Failed(_)));
}

#[test]
fn diff_mode_toggle_flips_all_and_changed() {
    let mut state = TracerState::new();
    seed_lineage(&mut state, &[1]);

    // Default is All
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.diff_mode, AttributeDiffMode::All);
    }

    lineage_toggle_diff_mode(&mut state);
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.diff_mode, AttributeDiffMode::Changed);
    }

    lineage_toggle_diff_mode(&mut state);
    {
        let TracerMode::Lineage(ref view) = state.mode else {
            panic!("expected Lineage mode");
        };
        assert_eq!(view.diff_mode, AttributeDiffMode::All);
    }
}

#[test]
fn selected_component_label_returns_latest_events_label() {
    let view = LatestEventsView {
        component_id: "cid".into(),
        component_label: "MyProcessor".into(),
        events: Vec::new(),
        selected: 0,
        fetched_at: SystemTime::UNIX_EPOCH,
        loading: false,
    };
    let ts = TracerState {
        mode: TracerMode::LatestEvents(view),
        last_error: None,
        content_modal: None,
    };
    assert_eq!(ts.selected_component_label(), Some("MyProcessor".into()));
}

#[test]
fn selected_component_label_none_in_entry_mode() {
    let ts = TracerState::new();
    assert_eq!(ts.selected_component_label(), None);
}
