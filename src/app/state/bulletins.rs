//! Bulletins tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, PendingIntent, UpdateResult, ViewKeyHandler};
use crate::intent::CrossLink;

/// Zero-sized dispatch struct for the Bulletins tab.
pub(crate) struct BulletinsHandler;

impl ViewKeyHandler for BulletinsHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Text-input mode captures character-level keys and edit keys (Esc,
        // Enter, Backspace). Keys with CONTROL modifiers (Ctrl+C, etc.) skip this
        // block so they reach the global handlers. Tab and other unmodified keys
        // are still suppressed to keep focus on text input. Printable characters
        // including capitals and brackets are captured by handle_text_input; to
        // use them as app-wide commands the user must press Esc to exit text
        // input mode first.
        if state.bulletins.text_input.is_some()
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            return handle_text_input(state, key);
        }

        // View-local keys take priority over global `e`. Accept
        // NONE or SHIFT modifiers so `G` and `T` (typed as Shift+g / Shift+t)
        // reach the handler — crossterm delivers them as
        // `KeyCode::Char('G')` with `KeyModifiers::SHIFT`.
        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }

        match key.code {
            KeyCode::Char('e') => {
                state.bulletins.toggle_error();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('w') => {
                state.bulletins.toggle_warning();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('i') => {
                state.bulletins.toggle_info();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('T') => {
                state.bulletins.cycle_component_type();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('c') => {
                state.bulletins.clear_filters();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('p') => {
                state.bulletins.toggle_pause();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('/') => {
                state.bulletins.enter_text_input_mode();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('g') => {
                state.bulletins.cycle_group_mode();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Home => {
                state.bulletins.jump_to_oldest();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('G') | KeyCode::End => {
                state.bulletins.jump_to_newest();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Up => {
                state.bulletins.move_selection_up();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Down => {
                state.bulletins.move_selection_down();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Enter => {
                if let Some(idx) = state.bulletins.selected_ring_index() {
                    let b = &state.bulletins.ring[idx];
                    let link = CrossLink::OpenInBrowser {
                        component_id: b.source_id.clone(),
                        group_id: b.group_id.clone(),
                    };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                        tracer_followup: None,
                    });
                }
                Some(UpdateResult::default())
            }
            KeyCode::Char('m') => {
                state.bulletins.mute_selected_source();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('t') => {
                if let Some(idx) = state.bulletins.selected_ring_index() {
                    let b = &state.bulletins.ring[idx];
                    let link = CrossLink::JumpToEvents {
                        component_id: b.source_id.clone(),
                    };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                        tracer_followup: None,
                    });
                }
                Some(UpdateResult::default())
            }
            _ => None,
        }
    }

    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
        use crate::widget::hint_bar::HintSpan;

        if state.bulletins.text_input.is_some() {
            return vec![
                HintSpan {
                    key: "type",
                    action: "filter",
                },
                HintSpan {
                    key: "Enter",
                    action: "apply",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ];
        }

        let group_action: &'static str = match state.bulletins.group_mode {
            crate::view::bulletins::state::GroupMode::SourceAndMessage => "group: source+msg",
            crate::view::bulletins::state::GroupMode::Source => "group: source",
            crate::view::bulletins::state::GroupMode::Off => "group: off",
        };

        vec![
            HintSpan {
                key: "j/k",
                action: "nav",
            },
            HintSpan {
                key: "Enter",
                action: "browser",
            },
            HintSpan {
                key: "t",
                action: "events",
            },
            HintSpan {
                key: "/",
                action: "filter",
            },
            HintSpan {
                key: "Space",
                action: "pause",
            },
            HintSpan {
                key: "g",
                action: group_action,
            },
            HintSpan {
                key: "m",
                action: "mute",
            },
        ]
    }
}

/// Handles keypresses while the Bulletins text-input mode is active.
fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    match key.code {
        KeyCode::Esc => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.cancel_text_input(prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Enter => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.commit_text_input(prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Backspace => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.pop_text_input(prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Char(ch) => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.push_text_input(ch, prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        _ => Some(UpdateResult::default()),
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::{PendingIntent, ViewId};
    use crate::client::BulletinSnapshot;
    use crate::event::{AppEvent, BulletinsPayload, ViewPayload};
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    #[test]
    fn bulletins_data_event_seeds_ring() {
        let mut s = fresh_state();
        let c = tiny_config();
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "m".into(),
                source_id: "a".into(),
                source_name: "A".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            }],
            fetched_at: SystemTime::now(),
        };
        let r = update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        assert!(r.redraw);
        assert_eq!(s.bulletins.ring.len(), 1);
    }

    #[test]
    fn on_bulletins_tab_e_toggles_error_chip() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert!(s.bulletins.filters.show_error);
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(!s.bulletins.filters.show_error);
    }

    #[test]
    fn on_bulletins_tab_slash_enters_text_input_mode() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_some());
    }

    #[test]
    fn bulletins_text_input_mode_consumes_chars_and_global_keys_are_suppressed() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input.as_deref(), Some("foo"));
        // Tab should NOT cycle tabs while typing.
        update(&mut s, key(KeyCode::Tab, KeyModifiers::NONE), &c);
        assert_eq!(s.current_tab, ViewId::Bulletins);
        // Enter commits.
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_none());
        assert_eq!(s.bulletins.filters.text, "foo");
    }

    #[test]
    fn on_bulletins_tab_enter_emits_jump_to_browser_intent() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed one bulletin so there's a selection.
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "m".into(),
                source_id: "proc-1".into(),
                source_name: "A".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            }],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(crate::intent::CrossLink::OpenInBrowser {
                component_id,
                group_id,
            })) => {
                assert_eq!(component_id, "proc-1");
                assert_eq!(group_id, "root");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
    }

    #[test]
    fn text_input_mode_does_not_swallow_ctrl_c_quit() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Enter text-input mode.
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.text_input.is_some());
        // Type a character to verify normal input still works.
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input.as_deref(), Some("f"));
        // Ctrl+C should quit, NOT push 'c' into the buffer.
        let r = update(&mut s, key(KeyCode::Char('c'), KeyModifiers::CONTROL), &c);
        assert!(s.should_quit, "Ctrl+C should trigger quit");
        assert!(matches!(r.intent, Some(PendingIntent::Quit)));
        // The text buffer must not have been modified by the Ctrl+C keystroke.
        assert_eq!(
            s.bulletins.text_input.as_deref(),
            Some("f"),
            "Ctrl+C must not append 'c' to the filter buffer"
        );
    }

    #[test]
    fn text_input_mode_captures_capital_k_as_text() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        // Shift+K is a printable character — it should be captured into the
        // filter buffer, not escape to the global handler. The user must Esc
        // out of text-input mode first to use K as an app-wide command.
        update(&mut s, key(KeyCode::Char('K'), KeyModifiers::SHIFT), &c);
        assert!(
            s.modal.is_none(),
            "Shift+K must not open the context switcher while in text-input mode"
        );
        assert_eq!(
            s.bulletins.text_input.as_deref(),
            Some("fK"),
            "Shift+K should be appended to the filter buffer as a literal K"
        );
    }

    #[test]
    fn on_bulletins_tab_g_cycles_group_mode() {
        use crate::view::bulletins::state::GroupMode;
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert_eq!(s.bulletins.group_mode, GroupMode::SourceAndMessage);
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::Source);
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::Off);
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::SourceAndMessage);
    }

    #[test]
    fn on_bulletins_tab_m_mutes_selected_source() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "P[id=a] boom".into(),
                source_id: "src-muted".into(),
                source_name: "P".into(),
                source_type: "PROCESSOR".into(),
                group_id: "g".into(),
                timestamp_iso: "2026-04-11T10:14:22Z".into(),
                timestamp_human: String::new(),
            }],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        assert!(s.bulletins.mutes.is_empty());
        update(&mut s, key(KeyCode::Char('m'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.mutes.contains("src-muted"));
    }

    #[test]
    fn on_bulletins_tab_shift_b_is_now_unbound() {
        // Regression guard: the old consecutive-group toggle is gone.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        let r = update(&mut s, key(KeyCode::Char('B'), KeyModifiers::SHIFT), &c);
        // `B` should be a no-op inside Bulletins now (the global handler
        // has no meaning for it either).
        assert!(!r.redraw);
    }

    #[test]
    fn on_bulletins_tab_lowercase_g_no_longer_jumps_home() {
        // Regression guard: `g` is now cycle-group-mode, not jump-oldest.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed >1 bulletin so a jump would be observable.
        let payload = BulletinsPayload {
            bulletins: vec![
                BulletinSnapshot {
                    id: 1,
                    level: "ERROR".into(),
                    message: "P[id=a] one".into(),
                    source_id: "s1".into(),
                    source_name: "P".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "g".into(),
                    timestamp_iso: "2026-04-11T10:14:22Z".into(),
                    timestamp_human: String::new(),
                },
                BulletinSnapshot {
                    id: 2,
                    level: "ERROR".into(),
                    message: "Q[id=b] two".into(),
                    source_id: "s2".into(),
                    source_name: "Q".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "g".into(),
                    timestamp_iso: "2026-04-11T10:14:23Z".into(),
                    timestamp_human: String::new(),
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        let before_selected = s.bulletins.selected;
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        // `g` cycles mode; selection is preserved via reconcile_selection.
        // It MUST NOT have been reset to 0 unless that happens to be the
        // reconciled position. Assert that mode changed — that's the
        // definitive test that `g` no longer jumps home.
        assert_ne!(
            s.bulletins.group_mode,
            crate::view::bulletins::state::GroupMode::SourceAndMessage
        );
        let _ = before_selected;
    }

    #[test]
    fn on_bulletins_tab_home_still_jumps_oldest() {
        // Regression guard: Home key still works as an alternative.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed two and move selection off the oldest.
        let payload = BulletinsPayload {
            bulletins: vec![
                BulletinSnapshot {
                    id: 1,
                    level: "INFO".into(),
                    message: "a".into(),
                    source_id: "s1".into(),
                    source_name: "A".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "g".into(),
                    timestamp_iso: "2026-04-11T10:14:22Z".into(),
                    timestamp_human: String::new(),
                },
                BulletinSnapshot {
                    id: 2,
                    level: "INFO".into(),
                    message: "b".into(),
                    source_id: "s2".into(),
                    source_name: "B".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "g".into(),
                    timestamp_iso: "2026-04-11T10:14:23Z".into(),
                    timestamp_human: String::new(),
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        s.bulletins.auto_scroll = false;
        s.bulletins.selected = 1;
        update(&mut s, key(KeyCode::Home, KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.selected, 0);
    }

    #[test]
    fn bulletins_hints_show_group_mode_label_for_g_key() {
        use super::super::ViewKeyHandler;
        use super::BulletinsHandler;
        use crate::view::bulletins::state::GroupMode;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        s.bulletins.group_mode = GroupMode::SourceAndMessage;
        let spans = BulletinsHandler::hints(&s);
        assert!(
            spans
                .iter()
                .any(|h| h.key == "g" && h.action.contains("source+msg")),
            "g hint should show current mode `source+msg`; got {spans:?}"
        );
        s.bulletins.group_mode = GroupMode::Off;
        let spans = BulletinsHandler::hints(&s);
        assert!(
            spans
                .iter()
                .any(|h| h.key == "g" && h.action.contains("off"))
        );
    }

    #[test]
    fn bulletins_hints_include_m_mute() {
        use super::super::ViewKeyHandler;
        use super::BulletinsHandler;
        let s = fresh_state();
        let spans = BulletinsHandler::hints(&s);
        assert!(spans.iter().any(|h| h.key == "m" && h.action == "mute"));
    }

    #[test]
    fn bulletins_hints_exclude_b_and_bundle_action() {
        use super::super::ViewKeyHandler;
        use super::BulletinsHandler;
        let s = fresh_state();
        let spans = BulletinsHandler::hints(&s);
        assert!(!spans.iter().any(|h| h.key == "B"));
        assert!(!spans.iter().any(|h| h.action.contains("bundle")));
    }

    #[test]
    fn bulletin_list_nav_uses_arrows_only_no_jk() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed two bulletins so row nav has room to move.
        let payload = BulletinsPayload {
            bulletins: vec![
                BulletinSnapshot {
                    id: 1,
                    level: "INFO".into(),
                    message: "first".into(),
                    source_id: "a".into(),
                    source_name: "A".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "root".into(),
                    timestamp_iso: "2026-04-11T10:14:22Z".into(),
                    timestamp_human: String::new(),
                },
                BulletinSnapshot {
                    id: 2,
                    level: "INFO".into(),
                    message: "second".into(),
                    source_id: "b".into(),
                    source_name: "B".into(),
                    source_type: "PROCESSOR".into(),
                    group_id: "root".into(),
                    timestamp_iso: "2026-04-11T10:14:23Z".into(),
                    timestamp_human: String::new(),
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(&mut s, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        s.bulletins.auto_scroll = false;
        s.bulletins.selected = 0;

        // j is a no-op.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.selected, 0, "j dropped");

        // Down still works.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert!(s.bulletins.selected > 0, "Down still works");

        let before = s.bulletins.selected;
        // k is a no-op.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.selected, before, "k dropped");

        // Up still works.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        assert!(s.bulletins.selected < before, "Up still works");
    }
}
