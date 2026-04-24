use super::super::tests::{fresh_state, key, tiny_config};
use super::super::update;
use crate::app::state::{ViewId, ViewKeyHandler};
use crate::client::tracer::ProvenanceEventDetail;
use crate::view::tracer::state::{self as ts, ContentPane, EventDetail, LineageView, TracerMode};
use crossterm::event::{KeyCode, KeyModifiers};
use std::time::SystemTime;

// Note: 'i'/'o' are no longer bound in Lineage mode (use ←/→ tab cycling instead).
// The old tests that verified 'i'/'o' banner behavior have been removed.
// io_letters_no_longer_bound (in Task 15 tests) verifies they are true no-ops.

#[test]
fn tracer_lineage_row_nav_uses_arrows_only_no_jk() {
    use crate::client::LineageSnapshot;
    use crate::client::tracer::ProvenanceEventSummary;

    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Tracer;
    let make_summary = |id: i64| ProvenanceEventSummary {
        event_id: id,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CREATE".to_string(),
        component_id: "comp-1".to_string(),
        component_name: "Gen".to_string(),
        component_type: "GenerateFlowFile".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: "ff-1".to_string(),
        relationship: None,
        details: None,
    };
    s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
        uuid: "ff-1".to_string(),
        snapshot: LineageSnapshot {
            events: vec![make_summary(1), make_summary(2)],
            percent_completed: 100,
            finished: true,
        },
        selected_event: 0,
        event_detail: EventDetail::NotLoaded,
        loaded_details: std::collections::HashMap::new(),
        diff_mode: ts::AttributeDiffMode::default(),
        fetched_at: SystemTime::now(),
        focus: ts::LineageFocus::default(),
        active_detail_tab: ts::DetailTab::default(),
    }));

    // j is a no-op (returns None, global handler no-ops).
    update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
    let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.selected_event
    } else {
        panic!("expected Lineage mode")
    };
    assert_eq!(sel, 0, "j dropped");

    // Down moves selection forward.
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.selected_event
    } else {
        panic!("expected Lineage mode")
    };
    assert!(sel > 0, "Down still works");

    let before = sel;
    // k is a no-op.
    update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
    let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.selected_event
    } else {
        panic!("expected Lineage mode")
    };
    assert_eq!(sel, before, "k dropped");

    // Up moves selection back.
    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.selected_event
    } else {
        panic!("expected Lineage mode")
    };
    assert!(sel < before, "Up still works");
}

#[test]
fn tracer_latest_events_row_nav_uses_arrows_only_no_jk() {
    use crate::client::LatestEventsSnapshot;
    use crate::client::tracer::ProvenanceEventSummary;
    use crate::view::tracer::state::LatestEventsView;

    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Tracer;
    let make_summary = |id: i64| ProvenanceEventSummary {
        event_id: id,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CREATE".to_string(),
        component_id: "comp-1".to_string(),
        component_name: "Gen".to_string(),
        component_type: "GenerateFlowFile".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: "ff-1".to_string(),
        relationship: None,
        details: None,
    };
    let snap = LatestEventsSnapshot {
        component_id: "comp-1".to_string(),
        component_label: "Gen".to_string(),
        events: vec![make_summary(1), make_summary(2)],
        fetched_at: SystemTime::now(),
    };
    s.tracer.mode = TracerMode::LatestEvents(LatestEventsView::from_snapshot(snap));

    // j is a no-op.
    update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
    let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
        v.selected
    } else {
        panic!("expected LatestEvents mode")
    };
    assert_eq!(sel, 0, "j dropped");

    // Down moves selection forward.
    update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
    let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
        v.selected
    } else {
        panic!("expected LatestEvents mode")
    };
    assert!(sel > 0, "Down still works");

    let before = sel;
    // k is a no-op.
    update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
    let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
        v.selected
    } else {
        panic!("expected LatestEvents mode")
    };
    assert_eq!(sel, before, "k dropped");

    // Up moves selection back.
    update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
    let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
        v.selected
    } else {
        panic!("expected LatestEvents mode")
    };
    assert!(sel < before, "Up still works");
}

// ── Task 15 helpers ───────────────────────────────────────────────────────

/// Seeds a Lineage state with a loaded event detail that has BOTH input
/// and output available. The detail pane focus is on Attributes.
fn seed_tracer_with_loaded_detail(s: &mut crate::app::state::AppState) {
    use crate::client::LineageSnapshot;
    use crate::client::tracer::ProvenanceEventSummary;

    let summary = ProvenanceEventSummary {
        event_id: 99,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CONTENT_MODIFIED".to_string(),
        component_id: "comp-99".to_string(),
        component_name: "UpdateAttribute".to_string(),
        component_type: "UpdateAttribute".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: "ff-99".to_string(),
        relationship: None,
        details: None,
    };
    let detail = ProvenanceEventDetail {
        summary: summary.clone(),
        attributes: vec![],
        transit_uri: None,
        input_available: true,
        output_available: true,
        input_size: None,
        output_size: None,
    };
    s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
        uuid: "ff-99".to_string(),
        snapshot: LineageSnapshot {
            events: vec![summary],
            percent_completed: 100,
            finished: true,
        },
        selected_event: 0,
        event_detail: EventDetail::Loaded {
            event: Box::new(detail),
            content: ContentPane::default(),
        },
        loaded_details: std::collections::HashMap::new(),
        diff_mode: ts::AttributeDiffMode::default(),
        fetched_at: SystemTime::now(),
        focus: ts::LineageFocus::Attributes { row: 0 },
        active_detail_tab: ts::DetailTab::Attributes,
    }));
}

/// Seeds a Lineage state with output only (no input available).
fn seed_tracer_with_output_only(s: &mut crate::app::state::AppState) {
    use crate::client::LineageSnapshot;
    use crate::client::tracer::ProvenanceEventSummary;

    let summary = ProvenanceEventSummary {
        event_id: 88,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CREATE".to_string(),
        component_id: "comp-88".to_string(),
        component_name: "GenerateFlowFile".to_string(),
        component_type: "GenerateFlowFile".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: "ff-88".to_string(),
        relationship: None,
        details: None,
    };
    let detail = ProvenanceEventDetail {
        summary: summary.clone(),
        attributes: vec![],
        transit_uri: None,
        input_available: false,
        output_available: true,
        input_size: None,
        output_size: None,
    };
    s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
        uuid: "ff-88".to_string(),
        snapshot: LineageSnapshot {
            events: vec![summary],
            percent_completed: 100,
            finished: true,
        },
        selected_event: 0,
        event_detail: EventDetail::Loaded {
            event: Box::new(detail),
            content: ContentPane::default(),
        },
        loaded_details: std::collections::HashMap::new(),
        diff_mode: ts::AttributeDiffMode::default(),
        fetched_at: SystemTime::now(),
        focus: ts::LineageFocus::Attributes { row: 0 },
        active_detail_tab: ts::DetailTab::Attributes,
    }));
}

// ── Task 15 tests ─────────────────────────────────────────────────────────

#[test]
fn right_cycles_detail_tabs() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    // Ensure we're on Attributes tab.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
    }

    super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
    let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.active_detail_tab
    } else {
        panic!("expected Lineage")
    };
    assert_eq!(tab, ts::DetailTab::Input);
}

#[test]
fn right_skips_disabled_tab() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_output_only(&mut s);
    // Ensure we're on Attributes tab.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
    }

    super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
    let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.active_detail_tab
    } else {
        panic!("expected Lineage")
    };
    assert_eq!(tab, ts::DetailTab::Output, "should skip Input (disabled)");
}

#[test]
fn d_toggles_diff() {
    use crate::input::{TracerVerb, ViewVerb};
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    let before = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.diff_mode
    } else {
        panic!("expected Lineage")
    };
    super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::ToggleDiff));
    let after = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.diff_mode
    } else {
        panic!("expected Lineage")
    };
    assert_ne!(after, before, "ToggleDiff should flip diff_mode");
}

#[test]
fn s_saves_only_on_content_tab() {
    use crate::client::tracer::{ContentRender, ContentSide};
    use crate::input::{TracerVerb, ViewVerb};

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);

    // Attributes tab → Save should be a no-op.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
    }
    super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::Save));
    assert!(
        s.modal.is_none(),
        "Save must be a no-op when Attributes tab is active"
    );

    // Input tab with content shown → Save should open modal.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Input;
        v.focus = ts::LineageFocus::Content { scroll: 0 };
        let existing_event = if let EventDetail::Loaded { ref event, .. } = v.event_detail {
            event.clone()
        } else {
            panic!("expected Loaded")
        };
        v.event_detail = EventDetail::Loaded {
            event: existing_event,
            content: ContentPane::Shown {
                side: ContentSide::Input,
                render: ContentRender::Text {
                    text: "data".to_string(),
                    pretty_printed: false,
                },
                bytes_fetched: 4,
                truncated: false,
            },
        };
    }
    super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::Save));
    assert!(
        matches!(s.modal, Some(crate::app::state::Modal::SaveEventContent(_))),
        "Save should open modal when Input tab is active and content is Shown"
    );
}

#[test]
fn io_letters_no_longer_bound() {
    let mut s = fresh_state();
    let c = tiny_config();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
    }

    // 'i' should not switch to Input tab — only Left/Right does that.
    update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
    let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
        v.active_detail_tab
    } else {
        panic!("expected Lineage")
    };
    assert_eq!(
        tab,
        ts::DetailTab::Attributes,
        "'i' must not switch to Input — use Left/Right"
    );
}

#[test]
fn descend_on_timeline_focuses_detail_pane() {
    // Regression: pressing Enter on a Timeline selection must move
    // focus into the Detail pane. Before the fix, handle_focus only
    // set `active_detail_tab` and `lineage_mark_detail_loading`
    // reset `focus` back to Timeline, so users couldn't reach the
    // detail pane to check attribute values or cycle tabs.
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    // Start with focus on the timeline, matching the state after a
    // lineage query completes.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.focus = ts::LineageFocus::Timeline;
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Descend)
        .expect("Descend on Timeline must be consumed");
    assert!(r.redraw);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert!(
        matches!(v.focus, ts::LineageFocus::Attributes { .. }),
        "Descend must move focus into the Attributes tab, got {:?}",
        v.focus
    );
    assert_eq!(v.active_detail_tab, ts::DetailTab::Attributes);
}

#[test]
fn right_from_timeline_descent_cycles_to_input_tab() {
    // After Descend lands focus on Attributes, pressing Right must
    // cycle to the Input tab (and set focus to Content). This
    // verifies the full Timeline → Detail → tab-cycle flow.
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.focus = ts::LineageFocus::Timeline;
    }

    super::TracerHandler::handle_focus(&mut s, FocusAction::Descend);
    super::TracerHandler::handle_focus(&mut s, FocusAction::Right);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert_eq!(v.active_detail_tab, ts::DetailTab::Input);
    assert!(matches!(v.focus, ts::LineageFocus::Content { .. }));
}

#[test]
fn cycling_to_input_tab_dispatches_fetch_event_content() {
    // Regression: cycling into the Input tab must trigger a
    // FetchEventContent intent and mark the content pane as
    // LoadingInput. Before the fix, the cycle just updated
    // `active_detail_tab` and left the content pane Collapsed, so
    // users saw an empty tab forever.
    use crate::app::state::PendingIntent;
    use crate::input::FocusAction;
    use crate::intent::{ContentSide as IntentSide, Intent};
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    // Reset to Attributes so Right cycles into Input.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
        v.focus = ts::LineageFocus::Attributes { row: 0 };
        if let EventDetail::Loaded {
            ref mut content, ..
        } = v.event_detail
        {
            *content = ContentPane::Collapsed;
        }
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Right)
        .expect("Right on Attributes tab must be consumed");
    assert!(r.redraw);
    assert!(
        matches!(
            r.intent,
            Some(PendingIntent::Dispatch(Intent::FetchEventContent {
                side: IntentSide::Input,
                ..
            }))
        ),
        "expected FetchEventContent(Input), got {:?}",
        r.intent
    );
    // Content pane should now be in LoadingInput state.
    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    let EventDetail::Loaded { ref content, .. } = v.event_detail else {
        panic!("expected Loaded detail");
    };
    assert!(matches!(content, ContentPane::LoadingInput));
}

#[test]
fn cycling_to_input_tab_is_noop_when_already_loaded() {
    // When content for the target side is already Shown, cycling
    // must not re-dispatch the fetch. Users don't want a network
    // round-trip every time they flip between tabs.
    use crate::client::ContentRender;
    use crate::client::ContentSide as ClientSide;
    use crate::input::FocusAction;

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
        v.focus = ts::LineageFocus::Attributes { row: 0 };
        if let EventDetail::Loaded {
            ref mut content, ..
        } = v.event_detail
        {
            *content = ContentPane::Shown {
                side: ClientSide::Input,
                render: ContentRender::Text {
                    text: "hi".into(),
                    pretty_printed: false,
                },
                bytes_fetched: 2,
                truncated: false,
            };
        }
    }

    // Cycle to Input (which is already loaded).
    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Right).expect("Right consumed");
    assert!(
        r.intent.is_none(),
        "no fetch should dispatch when side is already Shown"
    );
}

#[test]
fn next_pane_in_timeline_enters_attributes() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.focus = ts::LineageFocus::Timeline;
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::NextPane)
        .expect("NextPane on Timeline must be consumed");
    assert!(r.redraw);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert!(
        matches!(v.focus, ts::LineageFocus::Attributes { .. }),
        "NextPane from Timeline must move focus to Attributes, got {:?}",
        v.focus
    );
    assert_eq!(v.active_detail_tab, ts::DetailTab::Attributes);
}

#[test]
fn prev_pane_in_timeline_is_noop() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.focus = ts::LineageFocus::Timeline;
    }

    let _r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
        .expect("PrevPane on Timeline must be consumed (no-op)");

    // Focus must remain on Timeline.
    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert!(
        matches!(v.focus, ts::LineageFocus::Timeline),
        "PrevPane from Timeline must not change focus, got {:?}",
        v.focus
    );
}

#[test]
fn next_pane_in_attributes_cycles_tab_right() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
        v.focus = ts::LineageFocus::Attributes { row: 0 };
        if let EventDetail::Loaded {
            ref mut content, ..
        } = v.event_detail
        {
            *content = ContentPane::Collapsed;
        }
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::NextPane)
        .expect("NextPane in Attributes must be consumed");
    assert!(r.redraw);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert_eq!(
        v.active_detail_tab,
        ts::DetailTab::Input,
        "NextPane from Attributes must cycle to Input tab"
    );
}

#[test]
fn prev_pane_in_attributes_returns_to_timeline() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
        v.focus = ts::LineageFocus::Attributes { row: 0 };
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
        .expect("PrevPane in Attributes must be consumed");
    assert!(r.redraw);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert!(
        matches!(v.focus, ts::LineageFocus::Timeline),
        "PrevPane from Attributes must return focus to Timeline, got {:?}",
        v.focus
    );
}

#[test]
fn prev_pane_in_content_returns_to_timeline() {
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Input;
        v.focus = ts::LineageFocus::Content { scroll: 0 };
    }

    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
        .expect("PrevPane in Content must be consumed");
    assert!(r.redraw);

    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert!(
        matches!(v.focus, ts::LineageFocus::Timeline),
        "PrevPane from Content must return focus to Timeline, got {:?}",
        v.focus
    );
}

#[test]
fn left_right_in_detail_still_cycle_subtab() {
    // Left/Right must still cycle sub-tabs in the Attributes/Content focus,
    // unchanged by the NextPane/PrevPane additions.
    use crate::input::FocusAction;
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        v.active_detail_tab = ts::DetailTab::Attributes;
        v.focus = ts::LineageFocus::Attributes { row: 0 };
    }

    // Right should cycle to Input.
    super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert_eq!(
        v.active_detail_tab,
        ts::DetailTab::Input,
        "Right cycles to Input"
    );

    // Left should cycle back to Attributes.
    super::TracerHandler::handle_focus(&mut s, FocusAction::Left);
    let TracerMode::Lineage(ref v) = s.tracer.mode else {
        panic!("expected Lineage");
    };
    assert_eq!(
        v.active_detail_tab,
        ts::DetailTab::Attributes,
        "Left cycles back to Attributes"
    );
}

#[test]
fn build_pending_save_returns_event_id_and_side() {
    use crate::app::state::{PendingIntent, PendingSave};
    use crate::client::tracer::ProvenanceEventDetail;
    use crate::client::{ContentRender, ContentSide};

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    seed_tracer_with_loaded_detail(&mut s);
    // Override the loaded detail: event_id 77, output side content shown.
    if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
        let summary = crate::client::tracer::ProvenanceEventSummary {
            event_id: 77,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: "comp-77".to_string(),
            component_name: "Gen".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-77".to_string(),
            relationship: None,
            details: None,
        };
        let detail = ProvenanceEventDetail {
            summary,
            attributes: vec![],
            transit_uri: None,
            input_available: false,
            output_available: true,
            input_size: None,
            output_size: None,
        };
        v.event_detail = EventDetail::Loaded {
            event: Box::new(detail),
            content: ContentPane::Shown {
                side: ContentSide::Output,
                render: ContentRender::Text {
                    text: "hi".to_string(),
                    pretty_printed: false,
                },
                bytes_fetched: 2,
                truncated: false,
            },
        };
    } else {
        panic!("expected lineage mode");
    }

    let path = std::path::PathBuf::from("/tmp/out.bin");
    let pending = super::build_pending_save(&s, path.clone()).expect("should build pending save");
    match pending {
        PendingIntent::SaveEventContent(PendingSave {
            path: p,
            event_id,
            side,
        }) => {
            assert_eq!(p, path);
            assert_eq!(event_id, 77);
            assert_eq!(side, ContentSide::Output);
        }
        _ => panic!("expected SaveEventContent"),
    }
}

#[test]
fn collect_hints_save_label_shows_full_size_when_content_truncated() {
    use crate::client::tracer::ProvenanceEventSummary;
    use crate::client::{ContentRender, ContentSide, LineageSnapshot};

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;

    // 512 MiB output; content pane shows a truncated preview.
    let total_output_bytes: u64 = 512 * crate::bytes::MIB;
    let summary = ProvenanceEventSummary {
        event_id: 42,
        event_time_iso: "2026-01-01T00:00:00Z".to_string(),
        event_type: "CONTENT_MODIFIED".to_string(),
        component_id: "comp-42".to_string(),
        component_name: "SomeProcessor".to_string(),
        component_type: "SomeProcessor".to_string(),
        group_id: "root".to_string(),
        flow_file_uuid: "ff-42".to_string(),
        relationship: None,
        details: None,
    };
    let detail = ProvenanceEventDetail {
        summary: summary.clone(),
        attributes: vec![],
        transit_uri: None,
        input_available: false,
        output_available: true,
        input_size: None,
        output_size: Some(total_output_bytes),
    };
    s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
        uuid: "ff-42".to_string(),
        snapshot: LineageSnapshot {
            events: vec![summary],
            percent_completed: 100,
            finished: true,
        },
        selected_event: 0,
        event_detail: EventDetail::Loaded {
            event: Box::new(detail),
            content: ContentPane::Shown {
                side: ContentSide::Output,
                render: ContentRender::Text {
                    text: "preview...".to_string(),
                    pretty_printed: false,
                },
                bytes_fetched: 1024,
                truncated: true,
            },
        },
        loaded_details: std::collections::HashMap::new(),
        diff_mode: ts::AttributeDiffMode::default(),
        fetched_at: std::time::SystemTime::now(),
        focus: ts::LineageFocus::default(),
        active_detail_tab: ts::DetailTab::default(),
    }));

    let hints = super::super::collect_hints(&s);
    let save_hint = hints
        .iter()
        .find(|h| h.action.as_ref().contains("fetches full"))
        .expect("save hint should contain 'fetches full' when content is truncated");
    assert!(
        save_hint.action.as_ref().contains("512.0 MiB"),
        "save hint action should include the human-readable total size; got: {}",
        save_hint.action
    );
}

// ── Content modal scroll via handle_focus ─────────────────────────────────

/// Build a state with a content modal open and 100 fully-loaded lines on
/// the Input tab.
fn state_with_open_modal() -> crate::app::state::AppState {
    use crate::client::tracer::ContentRender;
    use crate::view::tracer::state::{
        ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
    };

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    let text = "row\n".repeat(100);
    let modal = ContentModalState {
        event_id: 1,
        header: ContentModalHeader {
            event_type: "DROP".into(),
            event_timestamp_iso: "".into(),
            component_name: "x".into(),
            pg_path: "pg".into(),
            input_size: Some(400),
            output_size: Some(400),
            input_mime: None,
            output_mime: None,
            input_available: true,
            output_available: true,
        },
        active_tab: ContentModalTab::Input,
        last_nondiff_tab: ContentModalTab::Input,
        diffable: Diffable::Pending,
        input: SideBuffer {
            loaded: text.clone().into_bytes(),
            decoded: ContentRender::Text {
                text,
                pretty_printed: false,
            },
            in_flight: false,
            fully_loaded: true,
            ceiling_hit: false,
            last_error: None,
            effective_ceiling: None,
            in_flight_decode: false,
        },
        output: SideBuffer::default(),
        diff_cache: None,
        scroll: crate::widget::scroll::BidirectionalScrollState {
            vertical: crate::widget::scroll::VerticalScrollState {
                offset: 50,
                last_viewport_rows: 20,
            },
            horizontal_offset: 0,
            last_viewport_body_cols: 0,
        },
        search: None,
    };
    s.tracer.content_modal = Some(modal);
    s
}

#[test]
fn handle_focus_up_scrolls_modal_when_open() {
    use crate::input::FocusAction;
    let mut s = state_with_open_modal();
    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Up)
        .expect("Up must be consumed when modal is open");
    assert!(r.redraw);
    assert_eq!(
        s.tracer
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .vertical
            .offset,
        49,
        "Up should decrement scroll offset by 1"
    );
}

#[test]
fn handle_focus_down_scrolls_modal_when_open() {
    use crate::input::FocusAction;
    let mut s = state_with_open_modal();
    let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Down)
        .expect("Down must be consumed when modal is open");
    assert!(r.redraw);
    assert_eq!(
        s.tracer
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .vertical
            .offset,
        51,
        "Down should increment scroll offset by 1"
    );
}

#[test]
fn handle_focus_first_jumps_to_top_when_modal_open() {
    use crate::input::FocusAction;
    let mut s = state_with_open_modal();
    super::TracerHandler::handle_focus(&mut s, FocusAction::First);
    assert_eq!(
        s.tracer
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .vertical
            .offset,
        0,
        "First/Home should jump to offset 0"
    );
}

#[test]
fn handle_focus_last_jumps_to_tail_when_modal_open() {
    use crate::input::FocusAction;
    let mut s = state_with_open_modal();
    super::TracerHandler::handle_focus(&mut s, FocusAction::Last);
    // 100 lines fully loaded, last valid offset = 99
    assert_eq!(
        s.tracer
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .vertical
            .offset,
        99,
        "Last/End should jump to last line"
    );
}

#[test]
fn handle_focus_page_down_advances_by_viewport_when_modal_open() {
    use crate::input::FocusAction;
    let mut s = state_with_open_modal();
    // scroll_offset = 50, last_viewport_rows = 20 → new offset = 70
    super::TracerHandler::handle_focus(&mut s, FocusAction::PageDown);
    assert_eq!(
        s.tracer
            .content_modal
            .as_ref()
            .unwrap()
            .scroll
            .vertical
            .offset,
        70,
        "PageDown should advance by viewport rows"
    );
}

// ── Modal search text input (C2) ──────────────────────────────────────────

fn state_with_searchable_modal(body: &str) -> crate::app::state::AppState {
    use crate::client::tracer::ContentRender;
    use crate::view::tracer::state::{
        ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
    };
    use crate::widget::search::SearchState;

    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;
    let text = body.to_owned();
    let modal = ContentModalState {
        event_id: 2,
        header: ContentModalHeader {
            event_type: "CONTENT_MODIFIED".into(),
            event_timestamp_iso: "".into(),
            component_name: "x".into(),
            pg_path: "pg".into(),
            input_size: None,
            output_size: None,
            input_mime: None,
            output_mime: None,
            input_available: true,
            output_available: false,
        },
        active_tab: ContentModalTab::Input,
        last_nondiff_tab: ContentModalTab::Input,
        diffable: Diffable::Pending,
        input: SideBuffer {
            loaded: text.clone().into_bytes(),
            decoded: ContentRender::Text {
                text,
                pretty_printed: false,
            },
            fully_loaded: true,
            in_flight: false,
            ceiling_hit: false,
            last_error: None,
            effective_ceiling: None,
            in_flight_decode: false,
        },
        output: SideBuffer::default(),
        diff_cache: None,
        scroll: crate::widget::scroll::BidirectionalScrollState {
            vertical: crate::widget::scroll::VerticalScrollState {
                offset: 0,
                last_viewport_rows: 10,
            },
            horizontal_offset: 0,
            last_viewport_body_cols: 0,
        },
        search: Some(SearchState {
            input_active: true,
            ..Default::default()
        }),
    };
    s.tracer.content_modal = Some(modal);
    s
}

#[test]
fn modal_search_typing_appends_to_query_and_recomputes_matches() {
    let body = "error line\nok line\nerror again";
    let mut s = state_with_searchable_modal(body);
    let c = tiny_config();

    // Simulate typing 'e', 'r', 'r' while search input is active.
    update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);

    let modal = s.tracer.content_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert_eq!(search.query, "err");
    assert_eq!(search.matches.len(), 2, "2 lines contain 'err'");
}

#[test]
fn modal_search_enter_commits_and_picks_first_match() {
    let body = "alpha\nbeta\nalpha again";
    let mut s = state_with_searchable_modal(body);
    let c = tiny_config();

    update(&mut s, key(KeyCode::Char('a'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

    let modal = s.tracer.content_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert!(
        !search.input_active,
        "Enter should commit (deactivate input)"
    );
    assert!(search.committed, "Enter should set committed = true");
    assert_eq!(search.current, Some(0), "first match selected");
}

#[test]
fn modal_search_esc_cancels() {
    let body = "some text";
    let mut s = state_with_searchable_modal(body);
    let c = tiny_config();

    update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);

    let modal = s.tracer.content_modal.as_ref().unwrap();
    assert!(
        modal.search.is_none(),
        "Esc should clear search state entirely"
    );
}

#[test]
fn modal_search_backspace_removes_char() {
    let body = "hello world";
    let mut s = state_with_searchable_modal(body);
    let c = tiny_config();

    update(&mut s, key(KeyCode::Char('w'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
    update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);

    {
        let modal = s.tracer.content_modal.as_ref().unwrap();
        let search = modal.search.as_ref().unwrap();
        assert_eq!(search.query, "wor");
    }

    update(&mut s, key(KeyCode::Backspace, KeyModifiers::NONE), &c);

    let modal = s.tracer.content_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert_eq!(search.query, "wo");
}

// ── SearchNext scroll-to-match (C3) ───────────────────────────────────────

#[test]
fn modal_search_next_scrolls_offset_into_viewport() {
    use crate::client::tracer::ContentRender;
    use crate::input::ContentModalVerb;
    use crate::view::tracer::state::{
        ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
    };
    use crate::widget::search::{MatchSpan, SearchState};

    // Build a state where search is committed with matches on lines 0 and 60,
    // current match at 0, viewport 10 rows at offset 0. SearchNext should
    // advance current to Some(1) and scroll to line 60.
    let mut s = fresh_state();
    s.current_tab = ViewId::Tracer;

    let text = "row\n".repeat(100);
    let modal = ContentModalState {
        event_id: 3,
        header: ContentModalHeader {
            event_type: "DROP".into(),
            event_timestamp_iso: "".into(),
            component_name: "x".into(),
            pg_path: "pg".into(),
            input_size: None,
            output_size: None,
            input_mime: None,
            output_mime: None,
            input_available: true,
            output_available: false,
        },
        active_tab: ContentModalTab::Input,
        last_nondiff_tab: ContentModalTab::Input,
        diffable: Diffable::Pending,
        input: SideBuffer {
            loaded: text.clone().into_bytes(),
            decoded: ContentRender::Text {
                text,
                pretty_printed: false,
            },
            fully_loaded: true,
            in_flight: false,
            ceiling_hit: false,
            last_error: None,
            effective_ceiling: None,
            in_flight_decode: false,
        },
        output: SideBuffer::default(),
        diff_cache: None,
        scroll: crate::widget::scroll::BidirectionalScrollState {
            vertical: crate::widget::scroll::VerticalScrollState {
                offset: 0,
                last_viewport_rows: 10,
            },
            horizontal_offset: 0,
            last_viewport_body_cols: 0,
        },
        search: Some(SearchState {
            query: "row".to_owned(),
            input_active: false,
            committed: true,
            matches: vec![
                MatchSpan {
                    line_idx: 0,
                    byte_start: 0,
                    byte_end: 3,
                },
                MatchSpan {
                    line_idx: 60,
                    byte_start: 0,
                    byte_end: 3,
                },
            ],
            current: Some(0),
        }),
    };
    s.tracer.content_modal = Some(modal);

    // Dispatch SearchNext via handle_verb.
    let r = super::TracerHandler::handle_verb(
        &mut s,
        crate::input::ViewVerb::ContentModal(ContentModalVerb::SearchNext),
    )
    .expect("SearchNext should be consumed");
    assert!(r.redraw);

    let modal = s.tracer.content_modal.as_ref().unwrap();
    let search = modal.search.as_ref().unwrap();
    assert_eq!(search.current, Some(1), "SearchNext advances to match 1");
    assert_eq!(
        modal.scroll.vertical.offset, 60,
        "scroll must jump to match line 60"
    );
}
