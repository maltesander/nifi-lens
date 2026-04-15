//! Events tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Banner, BannerSeverity, PendingIntent, UpdateResult, ViewKeyHandler};
use crate::input::FilterField as InputFilterField;
use crate::input::{EventsVerb, FocusAction, GoTarget, ViewVerb};
use crate::view::events::state::FilterField;

/// Convert `crate::input::FilterField` to `crate::view::events::state::FilterField`.
/// The two enums are structurally identical; this bridges the input layer to state.
fn input_field_to_state(f: InputFilterField) -> FilterField {
    match f {
        InputFilterField::Time => FilterField::Time,
        InputFilterField::Types => FilterField::Types,
        InputFilterField::Source => FilterField::Source,
        InputFilterField::Uuid => FilterField::Uuid,
        InputFilterField::Attr => FilterField::Attr,
    }
}

/// Zero-sized dispatch struct for the Events tab.
pub(crate) struct EventsHandler;

impl ViewKeyHandler for EventsHandler {
    fn handle_verb(state: &mut AppState, verb: ViewVerb) -> Option<UpdateResult> {
        let ev = match verb {
            ViewVerb::Events(v) => v,
            _ => return None,
        };

        match ev {
            EventsVerb::EditField(input_field) => {
                let field = input_field_to_state(input_field);
                // If in row-nav mode, exit it first and switch to filter edit.
                if state.events.selected_row.is_some() {
                    state.events.leave_row_nav();
                }
                state.events.enter_filter_edit(field);
            }
            EventsVerb::NewQuery => {
                // new_query() clears results and resets to Idle — no intent needed.
                // The user can press Enter (Descend) afterwards to actually run.
                state.events.new_query();
            }
            EventsVerb::Reset => {
                state.events.reset_filters();
            }
            EventsVerb::RaiseCap => {
                state.events.raise_cap();
            }
            EventsVerb::Refresh => {
                if let Some(r) = submit_query(state) {
                    return Some(r);
                }
            }
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }

    fn handle_focus(state: &mut AppState, action: FocusAction) -> Option<UpdateResult> {
        // If a field is being edited, Descend commits and Ascend cancels.
        // This path is defensive — normally text-input bypass handles edits.
        if state.events.filter_edit.is_some() {
            return match action {
                FocusAction::Descend => {
                    state.events.commit_filter_edit();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Ascend => {
                    state.events.cancel_filter_edit();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                _ => None,
            };
        }

        // Mode B: row selected.
        if state.events.selected_row.is_some() {
            return match action {
                FocusAction::Up => {
                    // At the top row (index 0), Up exits row-nav back to filter bar.
                    if state.events.selected_row == Some(0) {
                        state.events.leave_row_nav();
                    } else {
                        state.events.move_selection_up();
                    }
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Down => {
                    state.events.move_selection_down();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Ascend => {
                    state.events.leave_row_nav();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::PageUp => {
                    // Page up through results: go back 10 rows.
                    for _ in 0..10 {
                        if state.events.selected_row == Some(0) {
                            break;
                        }
                        state.events.move_selection_up();
                    }
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::PageDown => {
                    // Page down through results: go forward 10 rows.
                    for _ in 0..10 {
                        let max = state.events.events.len().saturating_sub(1);
                        if state.events.selected_row == Some(max) {
                            break;
                        }
                        state.events.move_selection_down();
                    }
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                // Descend on a row: no deeper structure — return None.
                // Rule 1a applies: default_cross_link returns None, so nothing happens.
                FocusAction::Descend => None,
                // Tab/Shift+Tab from row list returns to filter bar.
                FocusAction::NextPane | FocusAction::PrevPane => {
                    state.events.leave_row_nav();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Left | FocusAction::Right | FocusAction::First | FocusAction::Last => {
                    None
                }
            };
        }

        // Mode A: filter bar, no row selected, no field edit.
        match action {
            FocusAction::Down => {
                // Descend into results list.
                state.events.enter_row_nav();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Left | FocusAction::Right => None,
            FocusAction::Descend => {
                // Enter → submit query.
                submit_query(state)
            }
            FocusAction::Ascend => {
                // Esc at filter bar: no-op.
                None
            }
            // Tab from filter bar enters row-nav (selects first row).
            FocusAction::NextPane => {
                state.events.enter_row_nav();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            // Shift+Tab at filter bar: already at top pane, nothing to do.
            FocusAction::PrevPane => None,
            FocusAction::Up
            | FocusAction::PageUp
            | FocusAction::PageDown
            | FocusAction::First
            | FocusAction::Last => None,
        }
    }

    /// Events always has a local descent target (at minimum: submit a query from
    /// the filter bar). Return `None` so Enter never triggers a cross-link goto.
    fn default_cross_link(_state: &AppState) -> Option<GoTarget> {
        None
    }

    fn is_text_input_focused(state: &AppState) -> bool {
        state.events.filter_edit.is_some()
    }

    fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        if state.events.filter_edit.is_some()
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            handle_filter_edit(state, key)
        } else {
            None
        }
    }
}

fn redraw() -> Option<UpdateResult> {
    Some(UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
    })
}

fn submit_query(state: &mut AppState) -> Option<UpdateResult> {
    // Transition to Running immediately so the UI shows "running …"
    // even before the worker's first payload arrives.
    state.events.status = crate::view::events::state::EventsQueryStatus::Running {
        query_id: None,
        submitted_at: std::time::SystemTime::now(),
        percent: 0,
    };
    state.events.events.clear();
    state.events.selected_row = None;
    let query = state.events.build_query();
    Some(UpdateResult {
        redraw: true,
        intent: Some(PendingIntent::RunProvenanceQuery { query }),
        tracer_followup: None,
    })
}

fn handle_filter_edit(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    match key.code {
        KeyCode::Esc => {
            state.events.cancel_filter_edit();
            redraw()
        }
        KeyCode::Enter => {
            state.events.commit_filter_edit();
            redraw()
        }
        KeyCode::Backspace => {
            state.events.pop_filter_char();
            redraw()
        }
        KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
            match state.get_from_clipboard() {
                Ok(text) => {
                    for ch in text.chars() {
                        state.events.push_filter_char(ch);
                    }
                }
                Err(err) => {
                    state.status.banner = Some(Banner {
                        severity: BannerSeverity::Warning,
                        message: format!("clipboard paste: {err}"),
                        detail: None,
                    });
                }
            }
            redraw()
        }
        KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => {
            let text = state
                .events
                .current_filter_value()
                .unwrap_or_default()
                .to_owned();
            if !text.is_empty() {
                let _ = state.copy_to_clipboard(text);
            }
            state.events.cancel_filter_edit();
            redraw()
        }
        KeyCode::Char(ch) => {
            state.events.push_filter_char(ch);
            redraw()
        }
        _ => Some(UpdateResult::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::{PendingIntent, ViewId, ViewKeyHandler};
    use crate::client::ProvenanceEventSummary;
    use crate::event::{AppEvent, EventsPayload, ViewPayload};
    use crate::view::events::state::FilterField;
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    fn make_event(id: i64) -> ProvenanceEventSummary {
        ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: format!("uuid-{id}"),
            relationship: None,
            details: None,
        }
    }

    fn seed_events_with_results(state: &mut crate::app::state::AppState, n: usize) {
        let c = tiny_config();
        let events: Vec<ProvenanceEventSummary> = (1..=(n as i64)).map(make_event).collect();
        update(
            state,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            state,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events,
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
    }

    #[test]
    fn shift_d_on_filter_bar_enters_time_edit() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        update(&mut s, key(KeyCode::Char('D'), KeyModifiers::SHIFT), &c);
        assert_eq!(
            s.events.filter_edit.as_ref().map(|(f, _)| *f),
            Some(FilterField::Time)
        );
    }

    #[test]
    fn capital_t_on_filter_bar_enters_types_edit() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        update(&mut s, key(KeyCode::Char('T'), KeyModifiers::SHIFT), &c);
        assert_eq!(
            s.events.filter_edit.as_ref().map(|(f, _)| *f),
            Some(FilterField::Types)
        );
    }

    #[test]
    fn filter_edit_esc_cancels_and_restores() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        s.events.filters.source = "old".into();
        update(&mut s, key(KeyCode::Char('S'), KeyModifiers::SHIFT), &c);
        update(&mut s, key(KeyCode::Char('X'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.events.filters.source, "oldX");
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.events.filter_edit.is_none());
        assert_eq!(s.events.filters.source, "old");
    }

    #[test]
    fn shift_r_on_filter_bar_resets_filters() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        s.events.filters.source = "proc-1".into();
        update(&mut s, key(KeyCode::Char('R'), KeyModifiers::SHIFT), &c);
        assert!(s.events.filters.source.is_empty());
    }

    #[test]
    fn capital_l_raises_cap() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        assert_eq!(s.events.cap, 500);
        update(&mut s, key(KeyCode::Char('L'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.events.cap, 5000);
    }

    #[test]
    fn j_from_filter_bar_with_results_enters_row_nav() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = make_event(1);
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(0));
    }

    #[test]
    fn row_nav_esc_returns_to_filter_bar() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = make_event(1);
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        s.events.enter_row_nav();
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, None);
    }

    #[test]
    fn g_opens_goto_menu_then_down_enter_emits_trace_by_uuid_cross_link() {
        use crate::app::state::Modal;
        use crate::intent::CrossLink;
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: "ffuuid-42".into(),
            relationship: None,
            details: None,
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        s.events.enter_row_nav();
        // `g` opens the goto menu (Browser + Tracer are both available).
        let r_g = update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert!(r_g.intent.is_none());
        assert!(matches!(s.modal, Some(Modal::GotoMenu(_))));
        // Down selects index 1 = Tracer (Browser is 0, Tracer is 1).
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        // Enter confirms the Tracer target.
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(crate::app::state::PendingIntent::Goto(CrossLink::TraceByUuid { uuid })) => {
                assert_eq!(uuid, "ffuuid-42");
            }
            other => panic!("expected TraceByUuid, got {other:?}"),
        }
    }

    #[test]
    fn shift_d_in_row_nav_enters_time_filter_edit_not_trace() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = make_event(1);
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        s.events.enter_row_nav();
        // `Shift+D` in row-nav mode must enter Time filter edit, not trace.
        let r = update(&mut s, key(KeyCode::Char('D'), KeyModifiers::SHIFT), &c);
        assert!(
            r.intent.is_none(),
            "Shift+D in row-nav must not emit a TraceByUuid intent"
        );
        assert_eq!(
            s.events.filter_edit.as_ref().map(|(f, _)| *f),
            Some(FilterField::Time),
            "Shift+D in row-nav must enter Time filter edit mode"
        );
        // Row-nav must be exited when entering filter edit.
        assert!(
            s.events.selected_row.is_none(),
            "row-nav must be exited when entering filter edit"
        );
    }

    #[test]
    fn events_row_nav_uses_arrows_only_no_jk() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let make_ev = |id: i64| make_event(id);
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![make_ev(1), make_ev(2)],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );

        // j does NOT enter row nav.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, None, "j does not enter row nav");

        // Down enters row nav.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(0), "Down enters row nav");

        // From Mode B: Down moves selection forward.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(1), "Down moves selection down");

        // Up at row 1 moves to row 0.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(0), "Up moves selection up");

        // Up at row 0 exits row nav back to filter bar.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, None, "Up at top row exits row nav");

        // Re-enter Mode B and confirm Up moves selection up after Down.
        s.events.enter_row_nav();
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(1));
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(0), "Up moves selection up");
    }

    #[tokio::test]
    async fn goto_menu_tracer_then_outcome_switches_to_tracer_tab() {
        use crate::app::state::Modal;
        use crate::event::{IntentOutcome, ViewPayload};
        use crate::intent::CrossLink;
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "p".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "g".into(),
            flow_file_uuid: "ffuuid-42".into(),
            relationship: None,
            details: None,
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        s.events.enter_row_nav();

        // `g` opens the goto menu; Down selects Tracer (index 1); Enter confirms.
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::GotoMenu(_))));
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(matches!(
            r.intent,
            Some(crate::app::state::PendingIntent::Goto(
                CrossLink::TraceByUuid { .. }
            ))
        ));
        assert_eq!(s.current_tab, ViewId::Events);

        let join = tokio::spawn(async {});
        let abort = join.abort_handle();
        let outcome = IntentOutcome::TracerLineageStarted {
            uuid: "ffuuid-42".to_string(),
            abort,
        };
        update(&mut s, AppEvent::IntentOutcome(Ok(outcome)), &c);
        assert_eq!(s.current_tab, ViewId::Tracer);
    }

    #[test]
    fn g_goto_menu_enter_emits_open_in_browser_cross_link() {
        use crate::app::state::Modal;
        use crate::intent::CrossLink;
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        let e = ProvenanceEventSummary {
            event_id: 1,
            event_time_iso: "2026-04-13T10:00:00Z".into(),
            event_type: "DROP".into(),
            component_id: "proc-42".into(),
            component_name: "P".into(),
            component_type: "PROCESSOR".into(),
            group_id: "pg-9".into(),
            flow_file_uuid: "u".into(),
            relationship: None,
            details: None,
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryStarted {
                query_id: "q".into(),
            })),
            &c,
        );
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Events(EventsPayload::QueryDone {
                query_id: "q".into(),
                events: vec![e],
                fetched_at: SystemTime::now(),
                truncated: false,
            })),
            &c,
        );
        s.events.enter_row_nav();
        // `g` opens the goto menu; Enter selects index 0 = Browser.
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::GotoMenu(_))));
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(crate::app::state::PendingIntent::Goto(CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "proc-42");
                assert_eq!(group_id, "pg-9");
            }
            other => panic!("expected OpenInBrowser, got {other:?}"),
        }
    }

    // ---- New tests for typed handle_verb / handle_focus ----

    #[test]
    fn filter_field_letters_still_enter_edit() {
        use crate::input::{EventsVerb, FilterField as InputFilterField, ViewVerb};
        use crate::view::events::state::FilterField as StateFilterField;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;

        let _ = super::EventsHandler::handle_verb(
            &mut s,
            ViewVerb::Events(EventsVerb::EditField(InputFilterField::Source)),
        );
        // After the verb, filter_edit should be set for Source.
        assert!(
            super::EventsHandler::is_text_input_focused(&s),
            "is_text_input_focused should be true after EditField verb"
        );
        assert_eq!(
            s.events.filter_edit.as_ref().map(|(f, _)| *f),
            Some(StateFilterField::Source)
        );
    }

    #[test]
    fn enter_on_filter_bar_submits_query() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        // No field edit, no row selected — filter bar root.
        assert!(s.events.filter_edit.is_none());
        assert!(s.events.selected_row.is_none());

        let r = super::EventsHandler::handle_focus(&mut s, FocusAction::Descend)
            .expect("Descend consumed on filter bar");
        assert!(matches!(
            r.intent,
            Some(PendingIntent::RunProvenanceQuery { .. })
        ));
    }

    #[test]
    fn down_from_filter_bar_enters_results() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        seed_events_with_results(&mut s, 3);
        super::EventsHandler::handle_focus(&mut s, FocusAction::Down);
        // After Down from filter bar with results present, selected_row should be Some.
        assert!(
            s.events.selected_row.is_some(),
            "selected_row should be Some after Down from filter bar"
        );
    }

    #[test]
    fn no_enter_fallback_for_events() {
        let s = fresh_state();
        assert!(super::EventsHandler::default_cross_link(&s).is_none());
    }

    #[test]
    fn next_pane_from_filter_bar_enters_row_nav() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        seed_events_with_results(&mut s, 3);
        assert!(s.events.selected_row.is_none());
        let r = super::EventsHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.unwrap().redraw);
        assert!(s.events.selected_row.is_some());
    }

    #[test]
    fn next_pane_from_event_list_returns_to_filter_bar() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        seed_events_with_results(&mut s, 3);
        s.events.enter_row_nav();
        assert!(s.events.selected_row.is_some());
        super::EventsHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(s.events.selected_row.is_none());
    }

    #[test]
    fn left_right_unmapped_in_filter_bar_mode() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Events;
        assert!(s.events.selected_row.is_none());
        assert!(super::EventsHandler::handle_focus(&mut s, FocusAction::Left).is_none());
        assert!(super::EventsHandler::handle_focus(&mut s, FocusAction::Right).is_none());
    }
}
