//! Bulletins tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, PendingIntent, UpdateResult, ViewKeyHandler};
use crate::intent::CrossLink;

/// Zero-sized dispatch struct for the Bulletins tab.
pub(crate) struct BulletinsHandler;

impl ViewKeyHandler for BulletinsHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Text-input mode captures character-level keys and edit keys (Esc,
        // Enter, Backspace). Keys with CONTROL modifiers (Ctrl+C, Ctrl+K, etc.)
        // skip this block so they reach the global handlers. Tab and other
        // unmodified keys are still suppressed to keep focus on text input.
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
            KeyCode::Char('g') | KeyCode::Home => {
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
            KeyCode::Up | KeyCode::Char('k') => {
                state.bulletins.move_selection_up();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Down | KeyCode::Char('j') => {
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
            KeyCode::Char('t') => {
                if let Some(idx) = state.bulletins.selected_ring_index() {
                    let b = &state.bulletins.ring[idx];
                    let link = CrossLink::TraceComponent {
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
    use crate::app::state::{Modal, PendingIntent, ViewId};
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
    fn text_input_mode_does_not_swallow_ctrl_k_context_switcher() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
        // Ctrl+K should open the context switcher modal.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::CONTROL), &c);
        assert!(
            matches!(s.modal, Some(Modal::ContextSwitcher(_))),
            "Ctrl+K should open the context switcher"
        );
        assert_eq!(
            s.bulletins.text_input.as_deref(),
            Some("f"),
            "Ctrl+K must not append 'k' to the filter buffer"
        );
    }
}
