//! Bulletins tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, UpdateResult, ViewKeyHandler};
use crate::input::{BulletinsVerb, FocusAction, GoTarget, Severity, ViewVerb};

/// Zero-sized dispatch struct for the Bulletins tab.
pub(crate) struct BulletinsHandler;

impl ViewKeyHandler for BulletinsHandler {
    fn handle_verb(state: &mut AppState, verb: ViewVerb) -> Option<UpdateResult> {
        if state.bulletins.detail_modal.is_some() {
            return handle_modal_verb(state, verb);
        }
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
                let msg = state.bulletins.modal_copy_message().or_else(|| {
                    state
                        .bulletins
                        .group_details()
                        .map(|d| d.raw_message.clone())
                });
                let Some(msg) = msg else {
                    return Some(UpdateResult::default());
                };
                let preview: String = msg.chars().take(40).collect();
                match state.copy_to_clipboard(msg) {
                    Ok(()) => state.post_info(format!("copied: {preview}")),
                    Err(err) => state.post_warning(format!("clipboard: {err}")),
                }
            }
            BulletinsVerb::ClearFilters => state.bulletins.clear_filters(),
            BulletinsVerb::OpenSearch => state.bulletins.enter_text_input_mode(),
            BulletinsVerb::OpenDetail => {
                state.bulletins.open_detail_modal();
            }
            // Bulletins auto-refreshes; verb kept for parity but no state mutation.
            BulletinsVerb::Refresh => {}
            // SearchNext/SearchPrev are only active when the modal is open; they
            // are routed through handle_modal_verb and never reach this path.
            BulletinsVerb::SearchNext | BulletinsVerb::SearchPrev => {}
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
            sparkline_followup: None,
            queue_listing_followup: None,
        })
    }

    fn handle_focus(state: &mut AppState, action: FocusAction) -> Option<UpdateResult> {
        if state.bulletins.detail_modal.is_some() {
            return handle_modal_focus(state, action);
        }
        match action {
            FocusAction::Up => state.bulletins.move_selection_up(),
            FocusAction::Down => state.bulletins.move_selection_down(),
            FocusAction::First => state.bulletins.goto_oldest(),
            FocusAction::Last => state.bulletins.goto_newest(),
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
            sparkline_followup: None,
            queue_listing_followup: None,
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
            || state
                .bulletins
                .detail_modal
                .as_ref()
                .and_then(|m| m.search.as_ref())
                .map(|s| s.input_active)
                .unwrap_or(false)
    }

    fn blocks_app_shortcuts(state: &AppState) -> bool {
        Self::is_text_input_focused(state)
    }

    fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Modal search takes priority when its input is active.
        let modal_search_active = state
            .bulletins
            .detail_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if modal_search_active {
            return handle_modal_search_input(state, key);
        }
        // Existing list-search path (unchanged):
        if state.bulletins.text_input.is_some()
            && matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT)
        {
            handle_text_input(state, key)
        } else {
            None
        }
    }
}

/// Handles focus actions while the detail modal is open.
fn handle_modal_focus(state: &mut AppState, action: FocusAction) -> Option<UpdateResult> {
    match action {
        FocusAction::Up => state.bulletins.modal_scroll_by(-1),
        FocusAction::Down => state.bulletins.modal_scroll_by(1),
        FocusAction::PageUp => state.bulletins.modal_page_up(),
        FocusAction::PageDown => state.bulletins.modal_page_down(),
        FocusAction::First => state.bulletins.modal_jump_top(),
        FocusAction::Last => state.bulletins.modal_jump_bottom(),
        // Enter is intentionally a no-op inside the modal: double-pressing
        // Enter while using `/` search used to commit the search and then
        // jump to Browser, which was a surprising navigation. Use `g` on
        // the Bulletins tab to jump to the source in Browser.
        FocusAction::Descend => return Some(UpdateResult::default()),
        FocusAction::Ascend => state.bulletins.close_detail_modal(),
        FocusAction::Left | FocusAction::Right | FocusAction::NextPane | FocusAction::PrevPane => {
            return Some(UpdateResult::default());
        }
    }
    Some(UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
        sparkline_followup: None,
        queue_listing_followup: None,
    })
}

/// Handles verb dispatch while the detail modal is open.
fn handle_modal_verb(state: &mut AppState, verb: ViewVerb) -> Option<UpdateResult> {
    let bv = match verb {
        ViewVerb::Bulletins(v) => v,
        _ => return Some(UpdateResult::default()),
    };
    match bv {
        BulletinsVerb::OpenSearch => state.bulletins.modal_search_open(),
        BulletinsVerb::SearchNext => state.bulletins.modal_search_cycle_next(),
        BulletinsVerb::SearchPrev => state.bulletins.modal_search_cycle_prev(),
        BulletinsVerb::CopyMessage => {
            if let Some(msg) = state.bulletins.modal_copy_message() {
                let preview: String = msg.chars().take(40).collect();
                match state.copy_to_clipboard(msg) {
                    Ok(()) => state.post_info(format!("copied: {preview}")),
                    Err(err) => state.post_warning(format!("clipboard: {err}")),
                }
            }
        }
        // Everything else is swallowed while the modal is open — no
        // filter toggles, no pause, no group cycle. OpenDetail is a
        // no-op (already open). Refresh is a no-op.
        _ => {}
    }
    Some(UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
        sparkline_followup: None,
        queue_listing_followup: None,
    })
}

/// Handles text-input keypresses while modal search input is active.
fn handle_modal_search_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    match key.code {
        KeyCode::Esc => state.bulletins.modal_search_cancel(),
        KeyCode::Enter => state.bulletins.modal_search_commit(),
        KeyCode::Backspace => state.bulletins.modal_search_pop(),
        KeyCode::Char(ch) => state.bulletins.modal_search_push(ch),
        _ => return Some(UpdateResult::default()),
    }
    Some(UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
        sparkline_followup: None,
        queue_listing_followup: None,
    })
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
                sparkline_followup: None,
                queue_listing_followup: None,
            })
        }
        KeyCode::Enter => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.commit_text_input(prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            })
        }
        KeyCode::Backspace => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.pop_text_input(prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
                    state.post_warning(format!("clipboard paste: {err}"));
                }
            }
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
                sparkline_followup: None,
                queue_listing_followup: None,
            })
        }
        KeyCode::Char(ch) => {
            let prev = state.bulletins.selected_ring_index();
            state.bulletins.push_text_input(ch, prev);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
    use crossterm::event::{KeyCode, KeyModifiers};

    /// Merge the given bulletins into the cluster-owned ring and mirror
    /// them into `state.bulletins` via `redraw_bulletins` — the Task 7
    /// equivalent of the old `BulletinsPayload` data-event path.
    fn apply_bulletins(state: &mut AppState, bulletins: Vec<BulletinSnapshot>) {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::{Duration, Instant};
        state.cluster.snapshot.bulletins.merge(bulletins);
        state.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: Duration::from_secs(5),
        });
        crate::view::bulletins::state::redraw_bulletins(state);
    }

    /// Push one ERROR bulletin into `state.bulletins.ring` directly.
    fn seed_one_bulletin(state: &mut AppState) {
        apply_bulletins(
            state,
            vec![BulletinSnapshot {
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
        );
    }

    /// Push `n` INFO bulletins with distinct IDs into `state.bulletins.ring`.
    fn seed_multiple_bulletins(state: &mut AppState, n: usize) {
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
        apply_bulletins(state, bulletins);
        state.bulletins.auto_scroll = false;
        state.bulletins.selected = 0;
    }

    #[test]
    fn bulletins_data_event_seeds_ring() {
        let mut s = fresh_state();
        apply_bulletins(
            &mut s,
            vec![BulletinSnapshot {
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
        );
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
    fn on_bulletins_tab_enter_emits_goto_browser_intent() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed one bulletin so there's a selection.
        apply_bulletins(
            &mut s,
            vec![BulletinSnapshot {
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
        );
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::Goto(crate::intent::CrossLink::OpenInBrowser {
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
        apply_bulletins(
            &mut s,
            vec![BulletinSnapshot {
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
        );
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
    fn bare_g_produces_app_goto_not_group_cycle() {
        // After the keybind redesign, `g` maps to AppAction::Goto
        // and no longer cycles group-by mode (which moved to Y).
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        let before_mode = s.bulletins.group_mode;
        update(&mut s, key(KeyCode::Char('g'), KeyModifiers::NONE), &c);
        // Group mode MUST NOT have changed — `g` now maps to AppAction::Goto.
        assert_eq!(
            s.bulletins.group_mode, before_mode,
            "`g` must not cycle group mode after keybind redesign"
        );
    }

    #[test]
    fn on_bulletins_tab_home_still_gotos_oldest() {
        // Regression guard: Home key still works as an alternative.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed two and move selection off the oldest.
        apply_bulletins(
            &mut s,
            vec![
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
        );
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
            spans.iter().any(|h| h.key == "G" && h.action == "group"),
            "G hint should show `group`; got {spans:?}"
        );
    }

    #[test]
    fn bulletins_hints_include_shift_m_mute() {
        use crate::app::state::collect_hints;
        let mut s = fresh_state();
        s.current_tab = ViewId::Bulletins;
        let spans = collect_hints(&s);
        assert!(spans.iter().any(|h| h.key == "M" && h.action == "mute"));
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
        apply_bulletins(
            &mut s,
            vec![
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
        );
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

    #[test]
    fn i_opens_detail_modal_when_a_bulletin_is_selected() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        assert!(s.bulletins.detail_modal.is_none());
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.detail_modal.is_some());
    }

    #[test]
    fn i_is_noop_on_empty_bulletins_list() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.detail_modal.is_none());
    }

    #[test]
    fn modal_open_arrow_keys_scroll_body_not_list() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        // Down arrow must now move the modal's scroll_offset, not the list selection.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.bulletins.detail_modal.as_ref().unwrap().scroll.offset, 1);
        assert_eq!(s.bulletins.selected, 0);
    }

    #[test]
    fn modal_open_esc_closes_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        assert!(s.bulletins.detail_modal.is_some());
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.bulletins.detail_modal.is_none());
    }

    #[test]
    fn modal_open_enter_is_noop_does_not_jump_or_close() {
        // Regression: double-Enter while using `/`-search inside the modal
        // used to commit the search and then jump to Browser. Enter inside
        // the modal is now a no-op; `g` on the Bulletins tab is the sole
        // way to jump to the source.
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(
            s.bulletins.detail_modal.is_some(),
            "Enter in modal must not close it"
        );
        assert!(
            r.intent.is_none(),
            "Enter in modal must not emit a jump intent; got {:?}",
            r.intent
        );
    }

    #[test]
    fn slash_in_modal_enters_search_input_mode() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        let m = s.bulletins.detail_modal.as_ref().unwrap();
        assert!(m.search.as_ref().unwrap().input_active);
    }

    #[test]
    fn typing_in_modal_search_fills_query_not_scroll() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        let s_state = s
            .bulletins
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap();
        assert_eq!(s_state.query, "bo");
    }

    #[test]
    fn enter_in_search_commits_not_closes_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        let m = s.bulletins.detail_modal.as_ref().unwrap();
        assert!(m.search.as_ref().unwrap().committed);
        assert!(!m.search.as_ref().unwrap().input_active);
    }

    #[test]
    fn esc_in_search_cancels_search_not_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        let m = s.bulletins.detail_modal.as_ref().unwrap();
        assert!(m.search.is_none());
    }

    #[test]
    fn n_and_shift_n_cycle_after_commit() {
        use crate::cluster::snapshot::FetchMeta;
        use std::time::{Duration, Instant};
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        // Seed a bulletin with three "aaa" matches.
        s.cluster.snapshot.bulletins.merge(vec![BulletinSnapshot {
            id: 1,
            level: "ERROR".into(),
            message: "aaa bbb aaa ccc aaa".into(),
            source_id: "s".into(),
            source_name: "S".into(),
            source_type: "PROCESSOR".into(),
            group_id: "g".into(),
            timestamp_iso: "2026-04-20T10:00:00Z".into(),
            timestamp_human: String::new(),
        }]);
        s.cluster.snapshot.bulletins.meta = Some(FetchMeta {
            fetched_at: Instant::now(),
            fetch_duration: crate::test_support::default_fetch_duration(),
            next_interval: Duration::from_secs(5),
        });
        crate::view::bulletins::state::redraw_bulletins(&mut s);
        s.bulletins.auto_scroll = false;
        s.bulletins.selected = 0;

        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('/'), KeyModifiers::NONE), &c);
        for ch in "aaa".chars() {
            update(&mut s, key(KeyCode::Char(ch), KeyModifiers::NONE), &c);
        }
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        update(&mut s, key(KeyCode::Char('n'), KeyModifiers::NONE), &c);
        let cur = s
            .bulletins
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current;
        assert_eq!(cur, Some(1));

        update(&mut s, key(KeyCode::Char('N'), KeyModifiers::SHIFT), &c);
        let cur = s
            .bulletins
            .detail_modal
            .as_ref()
            .unwrap()
            .search
            .as_ref()
            .unwrap()
            .current;
        assert_eq!(cur, Some(0));
    }

    #[test]
    fn modal_open_c_copies_raw_message() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Bulletins;
        seed_one_bulletin(&mut s);
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        // `c` copies. Clipboard-availability varies on CI.
        // Verify via the status banner: a "copied: …" or "clipboard: …" message
        // must appear, proving the CopyMessage arm was reached and ran through
        // the modal path (not the group_details path).
        update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);
        let banner_msg = s
            .status
            .banner
            .as_ref()
            .map(|b| b.message.as_str())
            .unwrap_or("");
        assert!(
            banner_msg.starts_with("copied:") || banner_msg.starts_with("clipboard:"),
            "expected a copy banner, got {banner_msg:?}"
        );
    }
}
