//! Bulletins tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Banner, BannerSeverity, UpdateResult, ViewKeyHandler};
use crate::input::{BulletinsVerb, FocusAction, GoTarget, Severity, ViewVerb};

/// Zero-sized dispatch struct for the Bulletins tab.
pub(crate) struct BulletinsHandler;

impl ViewKeyHandler for BulletinsHandler {
    fn handle_verb(state: &mut AppState, verb: ViewVerb) -> Option<UpdateResult> {
        let bv = match verb {
            ViewVerb::Bulletins(v) => v,
            _ => return None,
        };
        match bv {
            BulletinsVerb::ToggleSeverity(Severity::Error) => state.bulletins.toggle_error(),
            BulletinsVerb::ToggleSeverity(Severity::Warning) => state.bulletins.toggle_warning(),
            BulletinsVerb::ToggleSeverity(Severity::Info) => state.bulletins.toggle_info(),
            BulletinsVerb::CycleTypeFilter => state.bulletins.cycle_component_type(),
            BulletinsVerb::CycleGroupBy => state.bulletins.cycle_group_mode(),
            BulletinsVerb::TogglePause => state.bulletins.toggle_pause(),
            BulletinsVerb::MuteSource => state.bulletins.mute_selected_source(),
            BulletinsVerb::CopyMessage => {
                let raw = state
                    .bulletins
                    .group_details()
                    .map(|d| d.raw_message.clone());
                let Some(msg) = raw else {
                    return Some(UpdateResult::default());
                };
                let preview: String = msg.chars().take(40).collect();
                match state.copy_to_clipboard(msg) {
                    Ok(()) => {
                        state.status.banner = Some(Banner {
                            severity: BannerSeverity::Info,
                            message: format!("copied: {preview}"),
                            detail: None,
                        });
                    }
                    Err(err) => {
                        state.status.banner = Some(Banner {
                            severity: BannerSeverity::Warning,
                            message: format!("clipboard: {err}"),
                            detail: None,
                        });
                    }
                }
            }
            BulletinsVerb::ClearFilters => state.bulletins.clear_filters(),
            BulletinsVerb::OpenSearch => state.bulletins.enter_text_input_mode(),
            // Bulletins auto-refreshes; verb kept for parity but no state mutation.
            BulletinsVerb::Refresh => {}
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }

    fn handle_focus(state: &mut AppState, action: FocusAction) -> Option<UpdateResult> {
        match action {
            FocusAction::Up => state.bulletins.move_selection_up(),
            FocusAction::Down => state.bulletins.move_selection_down(),
            FocusAction::First => state.bulletins.jump_to_oldest(),
            FocusAction::Last => state.bulletins.jump_to_newest(),
            // Descend: return None so the central dispatcher applies Rule 1a
            // (Enter-fallback to default_cross_link → Browser).
            FocusAction::Descend => return None,
            // Ascend: no-op at root level.
            FocusAction::Ascend => return None,
            // Left / Right / PageUp / PageDown / NextPane / PrevPane not bound for Bulletins.
            FocusAction::Left
            | FocusAction::Right
            | FocusAction::PageUp
            | FocusAction::PageDown
            | FocusAction::NextPane
            | FocusAction::PrevPane => return None,
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }

    fn default_cross_link(state: &AppState) -> Option<GoTarget> {
        if state.bulletins.selected_ring_index().is_some() {
            Some(GoTarget::Browser)
        } else {
            None
        }
    }

    fn is_text_input_focused(state: &AppState) -> bool {
        state.bulletins.text_input.is_some()
    }

    fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        if state.bulletins.text_input.is_some()
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            handle_text_input(state, key)
        } else {
            None
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
        KeyCode::Char('v') if key.modifiers == KeyModifiers::NONE => {
            match state.get_from_clipboard() {
                Ok(text) => {
                    let prev = state.bulletins.selected_ring_index();
                    for ch in text.chars() {
                        state.bulletins.push_text_input(ch, prev);
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
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Char('x') if key.modifiers == KeyModifiers::NONE => {
            let text = state
                .bulletins
                .text_input_value()
                .unwrap_or_default()
                .to_owned();
            if !text.is_empty() {
                let _ = state.copy_to_clipboard(text);
            }
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.cancel_text_input(prev);
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
    use crate::app::state::{AppState, PendingIntent, ViewId};
    use crate::client::BulletinSnapshot;
    use crate::event::{AppEvent, BulletinsPayload, ViewPayload};
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    /// Push one ERROR bulletin into `state.bulletins.ring` directly.
    fn seed_one_bulletin(state: &mut AppState) {
        let c = tiny_config();
        let payload = BulletinsPayload {
            bulletins: vec![BulletinSnapshot {
                id: 1,
                level: "ERROR".into(),
                message: "seed[id=a] boom".into(),
                source_id: "seed-src".into(),
                source_name: "Seed".into(),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: "2026-04-14T00:00:01Z".into(),
                timestamp_human: String::new(),
            }],
            fetched_at: SystemTime::now(),
        };
        update(state, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
    }

    /// Push `n` INFO bulletins with distinct IDs into `state.bulletins.ring`.
    fn seed_multiple_bulletins(state: &mut AppState, n: usize) {
        let c = tiny_config();
        let bulletins = (1..=(n as i64))
            .map(|i| BulletinSnapshot {
                id: i,
                level: "INFO".into(),
                message: format!("msg-{i}"),
                source_id: format!("src-{i}"),
                source_name: format!("S{i}"),
                source_type: "PROCESSOR".into(),
                group_id: "root".into(),
                timestamp_iso: format!("2026-04-14T00:00:{:02}Z", i),
                timestamp_human: String::new(),
            })
            .collect();
        let payload = BulletinsPayload {
            bulletins,
            fetched_at: SystemTime::now(),
        };
        update(state, AppEvent::Data(ViewPayload::Bulletins(payload)), &c);
        state.bulletins.auto_scroll = false;
        state.bulletins.selected = 0;
    }

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
    fn on_bulletins_tab_1_toggles_error_chip() {
        // After the keybind redesign, severity filters are on 1/2/3 (not e/w/i).
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert!(s.bulletins.filters.show_error);
        update(&mut s, key(KeyCode::Char('1'), KeyModifiers::NONE), &c);
        assert!(!s.bulletins.filters.show_error);
    }

    #[test]
    fn on_bulletins_tab_e_is_now_noop() {
        // Regression guard: `e` no longer toggles error filter in Bulletins.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert!(s.bulletins.filters.show_error);
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(
            s.bulletins.filters.show_error,
            "`e` must not toggle error filter after keybind redesign"
        );
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
    fn on_bulletins_tab_shift_g_cycles_group_mode() {
        // After the keybind redesign, cycle-group-by is on G (Shift+g).
        use crate::view::bulletins::state::GroupMode;
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        assert_eq!(s.bulletins.group_mode, GroupMode::SourceAndMessage);
        update(&mut s, key(KeyCode::Char('G'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::Source);
        update(&mut s, key(KeyCode::Char('G'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::Off);
        update(&mut s, key(KeyCode::Char('G'), KeyModifiers::SHIFT), &c);
        assert_eq!(s.bulletins.group_mode, GroupMode::SourceAndMessage);
    }

    #[test]
    fn on_bulletins_tab_shift_m_mutes_selected_source() {
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
        update(&mut s, key(KeyCode::Char('M'), KeyModifiers::SHIFT), &c);
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
    fn bare_g_produces_app_jump_not_group_cycle() {
        // After the keybind redesign, `g` maps to AppAction::Jump
        // and no longer cycles group-by mode (which moved to Y).
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        let before_mode = s.bulletins.group_mode;
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        // Group mode MUST NOT have changed — `g` now maps to AppAction::Jump.
        assert_eq!(
            s.bulletins.group_mode, before_mode,
            "`g` must not cycle group mode after keybind redesign"
        );
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
    fn bulletins_hints_show_group_by_hint_for_shift_g() {
        // After the keybind redesign, group-by is on Shift+G; collect_hints
        // derives the hint from BulletinsVerb::CycleGroupBy::hint() == "group".
        use crate::app::state::collect_hints;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let spans = collect_hints(&s);
        assert!(
            spans
                .iter()
                .any(|h| h.key == "Shift+G" && h.action == "group"),
            "Shift+G hint should show `group`; got {spans:?}"
        );
    }

    #[test]
    fn bulletins_hints_include_shift_m_mute() {
        use crate::app::state::collect_hints;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let spans = collect_hints(&s);
        assert!(
            spans
                .iter()
                .any(|h| h.key == "Shift+M" && h.action == "mute")
        );
    }

    #[test]
    fn bulletins_hints_exclude_b_and_bundle_action() {
        use crate::app::state::collect_hints;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let spans = collect_hints(&s);
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

    // ---- New tests for typed handle_verb / handle_focus / Rule 1a ----

    #[test]
    fn number_keys_toggle_severity_filters() {
        use crate::app::state::ViewKeyHandler;
        use crate::input::{BulletinsVerb, Severity, ViewVerb};
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let before = s.bulletins.filters.show_error;

        let r = super::BulletinsHandler::handle_verb(
            &mut s,
            ViewVerb::Bulletins(BulletinsVerb::ToggleSeverity(Severity::Error)),
        )
        .expect("verb consumed");
        assert!(r.redraw);
        assert_ne!(s.bulletins.filters.show_error, before);
    }

    #[test]
    fn shift_y_cycles_group_by() {
        use crate::app::state::ViewKeyHandler;
        use crate::input::{BulletinsVerb, ViewVerb};
        use crate::view::bulletins::state::GroupMode;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        s.bulletins.group_mode = GroupMode::SourceAndMessage;
        super::BulletinsHandler::handle_verb(
            &mut s,
            ViewVerb::Bulletins(BulletinsVerb::CycleGroupBy),
        );
        assert_eq!(s.bulletins.group_mode, GroupMode::Source);
    }

    #[test]
    fn enter_fallback_produces_browser_crosslink() {
        // Rule 1a: Bulletins has no local descent target, so Enter
        // (FocusAction::Descend) returns None from handle_focus; the
        // central dispatcher then falls back to the default cross-link.
        use crate::app::state::ViewKeyHandler;
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        let consumed = super::BulletinsHandler::handle_focus(&mut s, FocusAction::Descend);
        assert!(
            consumed.is_none(),
            "Bulletins Descend must not consume at root"
        );
    }

    #[test]
    fn default_cross_link_is_browser() {
        use crate::app::state::ViewKeyHandler;
        use crate::input::GoTarget;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        assert_eq!(
            super::BulletinsHandler::default_cross_link(&s),
            Some(GoTarget::Browser)
        );
    }

    #[test]
    fn arrow_keys_via_focus_move_selection() {
        use crate::app::state::ViewKeyHandler;
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        seed_multiple_bulletins(&mut s, 5);
        let before = s.bulletins.selected;
        super::BulletinsHandler::handle_focus(&mut s, FocusAction::Down);
        assert_eq!(s.bulletins.selected, before + 1);
    }

    #[test]
    fn paste_inserts_clipboard_text_into_search() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Enter search mode.
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        assert!(
            s.bulletins.text_input.is_some(),
            "must be in text-input mode"
        );
        // Copy a string to clipboard; skip test if clipboard is unavailable (CI).
        if s.copy_to_clipboard("hello".into()).is_err() {
            return;
        }
        // Verify the round-trip actually works on this system before asserting.
        // On some headless environments the write may not stick; skip if so.
        match s.get_from_clipboard() {
            Ok(ref v) if v == "hello" => {}
            _ => return,
        }
        // Press 'v' to paste.
        update(&mut s, key(KeyCode::Char('v'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input_value(), Some("hello"));
    }

    #[test]
    fn cut_cancels_text_input_mode() {
        // Verifies the structural behaviour of 'x' (cancel text-input) without
        // relying on clipboard availability.  A separate integration scenario
        // would cover the copy-to-clipboard half.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Enter search mode and type a query.
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('a'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.text_input_value(), Some("ab"));
        // Press 'x' to cut.
        update(&mut s, key(KeyCode::Char('x'), KeyModifiers::NONE), &c);
        // text_input mode is cancelled (cut calls cancel_text_input).
        assert!(
            s.bulletins.text_input.is_none(),
            "text_input should be cancelled after cut"
        );
    }
}
