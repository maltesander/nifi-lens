//! Events tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, PendingIntent, UpdateResult, ViewKeyHandler};
use crate::intent::CrossLink;
use crate::view::events::state::FilterField;

/// Zero-sized dispatch struct for the Events tab.
pub(crate) struct EventsHandler;

impl ViewKeyHandler for EventsHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Filter-edit mode captures character-level keys into the buffer.
        // Keys with CONTROL modifiers (Ctrl+C, etc.) skip this block so
        // they reach the global handlers. Printable characters including
        // capitals and brackets are captured here; to use them as app-wide
        // commands the user must Esc out of the edit first.
        if state.events.filter_edit.is_some()
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            return handle_filter_edit(state, key);
        }

        // Non-edit modes: gate on modifiers so app-wide chords still
        // reach the global handlers.
        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }

        // Mode B (row navigation) takes priority when a row is selected.
        if state.events.selected_row.is_some() {
            return handle_row_nav(state, key);
        }

        // Mode A (filter-bar navigation).
        handle_filter_nav(state, key)
    }

    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
        use crate::widget::hint_bar::HintSpan;

        if state.events.filter_edit.is_some() {
            return vec![
                HintSpan {
                    key: "type",
                    action: "edit",
                },
                HintSpan {
                    key: "Enter",
                    action: "commit",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ];
        }

        if state.events.selected_row.is_some() {
            return vec![
                HintSpan {
                    key: "j/k",
                    action: "nav",
                },
                HintSpan {
                    key: "t",
                    action: "trace uuid",
                },
                HintSpan {
                    key: "g",
                    action: "browser",
                },
                HintSpan {
                    key: "Esc",
                    action: "filters",
                },
            ];
        }

        vec![
            HintSpan {
                key: "t/T/s/u/a",
                action: "edit filter",
            },
            HintSpan {
                key: "Enter",
                action: "run",
            },
            HintSpan {
                key: "n",
                action: "new",
            },
            HintSpan {
                key: "r",
                action: "reset",
            },
            HintSpan {
                key: "L",
                action: "raise cap",
            },
            HintSpan {
                key: "j/k",
                action: "results",
            },
        ]
    }
}

fn redraw() -> Option<UpdateResult> {
    Some(UpdateResult {
        redraw: true,
        intent: None,
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
        KeyCode::Char(ch) => {
            state.events.push_filter_char(ch);
            redraw()
        }
        _ => Some(UpdateResult::default()),
    }
}

fn handle_filter_nav(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    match key.code {
        KeyCode::Char('t') => {
            state.events.enter_filter_edit(FilterField::Time);
            redraw()
        }
        KeyCode::Char('T') => {
            state.events.enter_filter_edit(FilterField::Types);
            redraw()
        }
        KeyCode::Char('s') => {
            state.events.enter_filter_edit(FilterField::Source);
            redraw()
        }
        KeyCode::Char('u') => {
            state.events.enter_filter_edit(FilterField::Uuid);
            redraw()
        }
        KeyCode::Char('a') => {
            state.events.enter_filter_edit(FilterField::Attr);
            redraw()
        }
        KeyCode::Char('n') => {
            state.events.new_query();
            redraw()
        }
        KeyCode::Char('r') => {
            state.events.reset_filters();
            redraw()
        }
        KeyCode::Char('L') => {
            state.events.raise_cap();
            redraw()
        }
        KeyCode::Enter => {
            // Task 14 will attach the RunProvenanceQuery intent here.
            // Phase 6 Task 9 leaves Enter as a redraw-only no-op.
            redraw()
        }
        KeyCode::Down | KeyCode::Char('j') | KeyCode::Up | KeyCode::Char('k') => {
            state.events.enter_row_nav();
            redraw()
        }
        _ => None,
    }
}

fn handle_row_nav(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    match key.code {
        KeyCode::Esc => {
            state.events.leave_row_nav();
            redraw()
        }
        KeyCode::Down | KeyCode::Char('j') => {
            state.events.move_selection_down();
            redraw()
        }
        KeyCode::Up | KeyCode::Char('k') => {
            state.events.move_selection_up();
            redraw()
        }
        KeyCode::Char('t') => {
            if let Some(e) = state.events.selected_event() {
                let link = CrossLink::TraceByUuid {
                    uuid: e.flow_file_uuid.clone(),
                };
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::JumpTo(link)),
                    tracer_followup: None,
                });
            }
            Some(UpdateResult::default())
        }
        KeyCode::Char('g') => {
            if let Some(e) = state.events.selected_event() {
                let link = CrossLink::OpenInBrowser {
                    component_id: e.component_id.clone(),
                    group_id: e.group_id.clone(),
                };
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::JumpTo(link)),
                    tracer_followup: None,
                });
            }
            Some(UpdateResult::default())
        }
        _ => {
            // Fall through to Mode A: leave row nav and dispatch as
            // a filter-bar key. This lets users press `t`, `n`, `r`,
            // etc. from row-nav and have them mean what they do on
            // the filter bar (after first leaving row nav).
            state.events.leave_row_nav();
            handle_filter_nav(state, key)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::ViewId;
    use crate::view::events::state::FilterField;
    use crossterm::event::{KeyCode, KeyModifiers};

    #[test]
    fn t_on_filter_bar_enters_time_edit() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
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
        update(&mut s, key(KeyCode::Char('s'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('X'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.events.filters.source, "oldX");
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.events.filter_edit.is_none());
        assert_eq!(s.events.filters.source, "old");
    }

    #[test]
    fn r_on_filter_bar_resets_filters() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Events;
        s.events.filters.source = "proc-1".into();
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
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
        use crate::client::ProvenanceEventSummary;
        use crate::event::{AppEvent, EventsPayload, ViewPayload};
        use std::time::SystemTime;
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
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, Some(0));
    }

    #[test]
    fn row_nav_esc_returns_to_filter_bar() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::{AppEvent, EventsPayload, ViewPayload};
        use std::time::SystemTime;
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
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert_eq!(s.events.selected_row, None);
    }

    #[test]
    fn row_t_emits_trace_by_uuid_cross_link() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::{AppEvent, EventsPayload, ViewPayload};
        use crate::intent::CrossLink;
        use std::time::SystemTime;
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
        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(crate::app::state::PendingIntent::JumpTo(CrossLink::TraceByUuid { uuid })) => {
                assert_eq!(uuid, "ffuuid-42");
            }
            other => panic!("expected TraceByUuid, got {other:?}"),
        }
    }

    #[test]
    fn row_g_emits_open_in_browser_cross_link() {
        use crate::client::ProvenanceEventSummary;
        use crate::event::{AppEvent, EventsPayload, ViewPayload};
        use crate::intent::CrossLink;
        use std::time::SystemTime;
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
        let r = update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(crate::app::state::PendingIntent::JumpTo(CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "proc-42");
                assert_eq!(group_id, "pg-9");
            }
            other => panic!("expected OpenInBrowser, got {other:?}"),
        }
    }
}
