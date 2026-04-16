//! Browser tab key handler.

use crossterm::event::KeyEvent;

use super::{AppState, Modal, UpdateResult, ViewKeyHandler};
use crate::input::{FocusAction, ViewVerb};
use crate::view::browser::state::{
    DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail,
};

/// Zero-sized dispatch struct for the Browser tab.
pub(crate) struct BrowserHandler;

impl ViewKeyHandler for BrowserHandler {
    fn handle_verb(state: &mut AppState, verb: ViewVerb) -> Option<UpdateResult> {
        use crate::input::BrowserVerb;
        let bv = match verb {
            ViewVerb::Browser(v) => v,
            _ => return None,
        };
        match bv {
            BrowserVerb::Refresh => {
                // Consume the force-tick oneshot. The worker wakes and fetches
                // immediately. Clearing the sender prevents a second press from
                // panicking.
                if let Some(tx) = state.browser.force_tick_tx.take() {
                    let _ = tx.send(());
                }
            }
            BrowserVerb::Copy => {
                // Copy depends on where focus is: row value in detail focus,
                // node id in tree focus.
                if matches!(state.browser.detail_focus, DetailFocus::Section { .. }) {
                    let Some(value) = state.browser.focused_row_copy_value(&state.bulletins.ring)
                    else {
                        return Some(UpdateResult::default());
                    };
                    let preview: String = value.chars().take(40).collect();
                    match state.copy_to_clipboard(value) {
                        Ok(()) => {
                            state.post_info(format!("copied: {preview}"));
                        }
                        Err(err) => {
                            state.post_warning(format!("clipboard: {err}"));
                        }
                    }
                } else {
                    // Tree focus: copy the selected node's id.
                    let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                        return Some(UpdateResult::default());
                    };
                    let id = state.browser.nodes[arena_idx].id.clone();
                    match state.copy_to_clipboard(id.clone()) {
                        Ok(()) => {
                            state.post_info(format!("copied id: {id}"));
                        }
                        Err(err) => {
                            state.post_warning(format!("clipboard: {err}"));
                        }
                    }
                }
            }
            BrowserVerb::OpenProperties => {
                // Open Properties modal only for Processor / CS with detail loaded.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let node_kind = state.browser.nodes[arena_idx].kind;
                let has_detail = state.browser.details.contains_key(&arena_idx);
                use crate::client::NodeKind as NK;
                if matches!(node_kind, NK::Processor | NK::ControllerService) && has_detail {
                    state.modal = Some(Modal::Properties(
                        crate::view::browser::state::PropertiesModalState::new(arena_idx),
                    ));
                } else {
                    return Some(UpdateResult::default());
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
        // Branch on whether we're in detail-section focus or tree focus.
        if let DetailFocus::Section {
            idx,
            rows,
            x_offsets,
        } = state.browser.detail_focus.clone()
        {
            let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                return Some(UpdateResult::default());
            };
            let kind = state.browser.nodes[arena_idx].kind;
            let has_validation = match state.browser.details.get(&arena_idx) {
                Some(NodeDetail::Processor(p)) => !p.validation_errors.is_empty(),
                Some(NodeDetail::ControllerService(cs)) => !cs.validation_errors.is_empty(),
                _ => false,
            };
            let sections = DetailSections::for_node_detail(kind, has_validation);
            return match action {
                FocusAction::Ascend => {
                    // Return to tree focus.
                    state.browser.detail_focus = DetailFocus::Tree;
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Left => {
                    let mut new_x = x_offsets;
                    new_x[idx] = new_x[idx].saturating_sub(1);
                    state.browser.detail_focus = DetailFocus::Section {
                        idx,
                        rows,
                        x_offsets: new_x,
                    };
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Right => {
                    let mut new_x = x_offsets;
                    new_x[idx] += 1;
                    state.browser.detail_focus = DetailFocus::Section {
                        idx,
                        rows,
                        x_offsets: new_x,
                    };
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Up => {
                    let mut new_rows = rows;
                    new_rows[idx] = new_rows[idx].saturating_sub(1);
                    state.browser.detail_focus = DetailFocus::Section {
                        idx,
                        rows: new_rows,
                        x_offsets,
                    };
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Down => {
                    // Capture section_len BEFORE mutating detail_focus to
                    // avoid a double-borrow of state.browser.
                    let current_section = sections.0[idx];
                    let max = state
                        .browser
                        .section_len(current_section, &state.bulletins.ring);
                    let mut new_rows = rows;
                    if max > 0 {
                        new_rows[idx] = (new_rows[idx] + 1).min(max - 1);
                    }
                    state.browser.detail_focus = DetailFocus::Section {
                        idx,
                        rows: new_rows,
                        x_offsets,
                    };
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Descend => {
                    // On the ChildGroups section, drill into the selected child PG.
                    if sections.0.get(idx) == Some(&DetailSection::ChildGroups) {
                        let pg_id = match state.browser.details.get(&arena_idx) {
                            Some(NodeDetail::ProcessGroup(d)) => d.id.clone(),
                            _ => return Some(UpdateResult::default()),
                        };
                        let kids = state.browser.child_process_groups(&pg_id);
                        let row = rows[idx];
                        let Some(target) = kids.get(row) else {
                            return Some(UpdateResult::default());
                        };
                        let target_id = target.id.clone();
                        if state.browser.drill_into_group(&target_id) {
                            state.browser.emit_detail_request_for_current_selection();
                        }
                        return Some(UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        });
                    }
                    // Other sections: no local descent.
                    None
                }
                FocusAction::NextPane => {
                    // Advance to the next section, wrapping back to Tree.
                    let section_count = sections.len();
                    if section_count == 0 {
                        return Some(UpdateResult::default());
                    }
                    let new_idx = idx + 1;
                    if new_idx >= section_count {
                        state.browser.detail_focus = DetailFocus::Tree;
                    } else {
                        state.browser.detail_focus = DetailFocus::Section {
                            idx: new_idx,
                            rows,
                            x_offsets,
                        };
                    }
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::PrevPane => {
                    // Go back to the previous section, wrapping to Tree at idx 0.
                    if sections.is_empty() {
                        return Some(UpdateResult::default());
                    }
                    if idx == 0 {
                        state.browser.detail_focus = DetailFocus::Tree;
                    } else {
                        state.browser.detail_focus = DetailFocus::Section {
                            idx: idx - 1,
                            rows,
                            x_offsets,
                        };
                    }
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::PageUp
                | FocusAction::PageDown
                | FocusAction::First
                | FocusAction::Last => None,
            };
        }

        // Tree focus.
        match action {
            FocusAction::Up => {
                state.browser.move_up();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Down => {
                state.browser.move_down();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::PageUp => {
                state.browser.page_up(10);
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::PageDown => {
                state.browser.page_down(10);
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::First => {
                state.browser.goto_first();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Last => {
                state.browser.goto_last();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Right => {
                // Expand the selected node / move to first child (same as the
                // old Enter/Right behavior).
                state.browser.enter_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Left => {
                // Collapse the current node or goto its parent.
                state.browser.backspace_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Descend => {
                // For a ProcessGroup: expand it (same as Right). For a leaf
                // Processor/ControllerService with focusable sections: enter
                // detail focus. For other node kinds: no-op (return None so
                // the default_cross_link fallback in the dispatcher can fire
                // if applicable).
                let &arena_idx = state.browser.visible.get(state.browser.selected)?;
                let kind = state.browser.nodes[arena_idx].kind;
                let sections = DetailSections::for_node(kind);
                use crate::client::NodeKind as NK;
                if matches!(kind, NK::ProcessGroup) {
                    state.browser.enter_selection();
                    state.browser.emit_detail_request_for_current_selection();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                } else if !sections.is_empty() {
                    // Leaf node with focusable sections — enter detail focus.
                    state.browser.detail_focus = DetailFocus::Section {
                        idx: 0,
                        rows: [0; MAX_DETAIL_SECTIONS],
                        x_offsets: [0; MAX_DETAIL_SECTIONS],
                    };
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                } else {
                    None
                }
            }
            FocusAction::Ascend => {
                // In tree focus: Ascend collapses the current node if expanded,
                // or gotos to the parent. Delegates to backspace_selection().
                state.browser.backspace_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::NextPane => {
                // From Tree: enter Section{0} if the selected node has sections.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let kind = state.browser.nodes[arena_idx].kind;
                let sections = DetailSections::for_node(kind);
                if sections.is_empty() {
                    return Some(UpdateResult::default());
                }
                state.browser.detail_focus = DetailFocus::Section {
                    idx: 0,
                    rows: [0; MAX_DETAIL_SECTIONS],
                    x_offsets: [0; MAX_DETAIL_SECTIONS],
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::PrevPane => {
                // From Tree: enter Section{last} if the selected node has sections.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let kind = state.browser.nodes[arena_idx].kind;
                let sections = DetailSections::for_node(kind);
                if sections.is_empty() {
                    return Some(UpdateResult::default());
                }
                state.browser.detail_focus = DetailFocus::Section {
                    idx: sections.len() - 1,
                    rows: [0; MAX_DETAIL_SECTIONS],
                    x_offsets: [0; MAX_DETAIL_SECTIONS],
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
        }
    }

    /// Browser always handles descent locally (expand PG or enter detail focus).
    fn default_cross_link(_state: &AppState) -> Option<crate::input::GoTarget> {
        None
    }

    fn is_text_input_focused(_state: &AppState) -> bool {
        false
    }

    fn handle_text_input(_state: &mut AppState, _key: KeyEvent) -> Option<UpdateResult> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, seeded_browser_state, tiny_config};
    use super::super::update;
    use super::BrowserHandler;
    use crate::app::state::{
        AppState, BannerSeverity, Modal, PendingIntent, ViewId, ViewKeyHandler,
    };
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use crate::config::Config;
    use crate::event::{AppEvent, BrowserPayload, ViewPayload};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{
        FlowIndex, FlowIndexEntry, MAX_DETAIL_SECTIONS, PropertiesModalState,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    #[test]
    fn browser_tree_payload_populates_browser_state_and_flow_index() {
        let mut s = fresh_state();
        let c = tiny_config();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "NiFi".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 1,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "root".into(),
                    name: "Gen".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        let r = update(
            &mut s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        assert!(r.redraw);
        assert_eq!(s.browser.nodes.len(), 2);
        assert_eq!(s.browser.visible.len(), 2); // root expanded -> 1 child visible
        let idx = s.flow_index.as_ref().expect("FlowIndex built");
        assert_eq!(idx.entries.len(), 2);
    }

    #[test]
    fn open_in_browser_target_switches_tab_and_expands_ancestors() {
        let mut s = fresh_state();
        let c = tiny_config();
        // Seed a small tree: root → ingest → upd.
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ProcessGroup,
                    id: "ingest".into(),
                    group_id: "root".into(),
                    name: "ingest".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(1),
                    kind: NodeKind::Processor,
                    id: "upd".into(),
                    group_id: "ingest".into(),
                    name: "UpdateAttribute".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );

        // Jump to "upd".
        let outcome = Ok(crate::event::IntentOutcome::OpenInBrowserTarget {
            component_id: "upd".into(),
            group_id: "ingest".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        let arena = s.browser.nodes.iter().position(|n| n.id == "upd").unwrap();
        let visible = s.browser.visible.iter().position(|&i| i == arena).unwrap();
        assert_eq!(s.browser.selected, visible);
        // Ancestor expanded: "ingest" (arena 1) ∈ expanded.
        assert!(s.browser.expanded.contains(&1));
    }

    #[test]
    fn open_in_browser_target_warns_when_id_not_in_arena() {
        let mut s = fresh_state();
        let c = tiny_config();
        let outcome = Ok(crate::event::IntentOutcome::OpenInBrowserTarget {
            component_id: "ghost".into(),
            group_id: "root".into(),
        });
        update(&mut s, AppEvent::IntentOutcome(outcome), &c);
        assert_eq!(s.current_tab, ViewId::Browser);
        let banner = s.status.banner.as_ref().unwrap();
        assert_eq!(banner.severity, BannerSeverity::Warning);
        assert!(banner.message.contains("ghost"));
    }

    #[test]
    fn on_browser_tab_down_moves_selection_down() {
        let (mut s, c) = seeded_browser_state();
        assert_eq!(s.browser.selected, 0);
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.selected, 1);
    }

    #[test]
    fn on_browser_tab_enter_on_collapsed_pg_drills_in() {
        let (mut s, c) = seeded_browser_state();
        // Move selection to "ingest" (visible row 2 in a seeded tree with
        // root expanded and "gen" as first child).
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(s.browser.expanded.contains(&2));
    }

    #[test]
    fn on_browser_tab_left_on_expanded_pg_collapses() {
        let (mut s, c) = seeded_browser_state();
        s.browser.expanded.insert(2);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert!(!s.browser.expanded.contains(&2));
    }

    #[test]
    fn on_browser_tab_r_fires_force_tick() {
        let (mut s, c) = seeded_browser_state();
        let (tx, _rx) = tokio::sync::oneshot::channel::<()>();
        s.browser.force_tick_tx = Some(tx);
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
        // Sender consumed; force_tick_tx is cleared.
        assert!(s.browser.force_tick_tx.is_none());
    }

    #[test]
    fn f_with_no_index_shows_warning_banner_and_does_not_open_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
        assert!(s.modal.is_none());
        assert!(
            s.status
                .banner
                .as_ref()
                .map(|b| b.severity == BannerSeverity::Warning)
                .unwrap_or(false)
        );
    }

    #[test]
    fn f_with_index_opens_fuzzy_find_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "p".into(),
                group_id: "root".into(),
                kind: NodeKind::Processor,
                name: "P".into(),
                group_path: "root".into(),
                state: crate::view::browser::state::StateBadge::Processor {
                    glyph: '\u{25CF}',
                    style: crate::theme::success(),
                },
                haystack: "p   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
        assert!(matches!(s.modal, Some(Modal::FuzzyFind(_))));
    }

    #[test]
    fn fuzzy_find_modal_enter_emits_open_in_browser_intent() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "target".into(),
                group_id: "g".into(),
                kind: NodeKind::Processor,
                name: "PutKafka".into(),
                group_path: "root".into(),
                state: crate::view::browser::state::StateBadge::Processor {
                    glyph: '\u{25CF}',
                    style: crate::theme::success(),
                },
                haystack: "putkafka   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
        update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::Goto(CrossLink::OpenInBrowser { component_id, .. })) => {
                assert_eq!(component_id, "target");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
        assert!(s.modal.is_none());
    }

    #[test]
    fn fuzzy_find_modal_esc_closes_without_goto() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "x".into(),
                group_id: "g".into(),
                kind: NodeKind::Processor,
                name: "X".into(),
                group_path: "root".into(),
                state: crate::view::browser::state::StateBadge::Processor {
                    glyph: '\u{25CF}',
                    style: crate::theme::success(),
                },
                haystack: "x   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('F'), KeyModifiers::SHIFT), &c);
        let r = update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
        assert!(r.intent.is_none());
    }

    #[test]
    fn p_on_processor_with_detail_opens_properties_modal() {
        use crate::client::ProcessorDetail;
        use crate::view::browser::state::NodeDetail;

        let (mut s, c) = seeded_browser_state();
        // Seed detail for "gen" (arena 1).
        s.browser.details.insert(
            1,
            NodeDetail::Processor(ProcessorDetail {
                id: "gen".into(),
                name: "Gen".into(),
                type_name: "x".into(),
                bundle: String::new(),
                run_status: "Running".into(),
                scheduling_strategy: String::new(),
                scheduling_period: String::new(),
                concurrent_tasks: 1,
                run_duration_ms: 0,
                penalty_duration: String::new(),
                yield_duration: String::new(),
                bulletin_level: String::new(),
                properties: vec![("k".into(), "v".into())],
                validation_errors: vec![],
            }),
        );
        s.browser.selected = 1; // visible row for arena 1
        update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::Properties(_))));
    }

    #[test]
    fn e_no_longer_opens_properties_modal() {
        // `e` used to open properties; now it is a no-op — use `p` instead.
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1;
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn p_on_processor_without_detail_is_noop() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1;
        update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn e_on_pg_is_noop() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 0; // root PG
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn esc_closes_properties_modal() {
        let (mut s, c) = seeded_browser_state();
        s.modal = Some(Modal::Properties(PropertiesModalState::new(1)));
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
    }

    #[test]
    fn t_is_no_longer_a_goto_events_shortcut() {
        // `t` used to emit GotoEvents; that shortcut is retired.
        // Users now navigate via `g` which opens the AppAction::Goto modal.
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1; // "gen" processor
        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        assert!(
            r.intent.is_none(),
            "t must no longer emit GotoEvents; got {r:?}"
        );
    }

    /// Build a 3-level tree: Root (PG) > Pipeline (PG) > Generate (Processor).
    /// Root and Pipeline are expanded so all three are visible.
    /// Returns (state, config) with `current_tab` set to Browser.
    fn three_level_browser_state() -> (AppState, Config) {
        let mut s = fresh_state();
        let c = tiny_config();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "Root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ProcessGroup,
                    id: "pipeline".into(),
                    group_id: "root".into(),
                    name: "Pipeline".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(1),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "pipeline".into(),
                    name: "Generate".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(
            &mut s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        // Expand Pipeline so Generate is visible.
        s.browser.expanded.insert(1);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.current_tab = ViewId::Browser;
        (s, c)
    }

    #[test]
    fn b_is_no_longer_breadcrumb_activation() {
        // `b` used to enter breadcrumb mode; the interactive breadcrumb mode
        // has been removed entirely. Pressing `b` must be a no-op.
        let (mut s, c) = three_level_browser_state();
        s.browser.selected = 2;
        let before_selected = s.browser.selected;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.selected, before_selected, "b must be a no-op");
    }

    #[test]
    fn b_at_root_is_noop() {
        // `b` is a no-op on both leaf and root nodes.
        let (mut s, c) = three_level_browser_state();
        s.browser.selected = 0;
        let before_selected = s.browser.selected;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.selected, before_selected,
            "b must be a no-op at root"
        );
    }

    #[test]
    fn left_key_collapses_expanded_pg_in_tree_focus() {
        // Left collapses an expanded PG (replaces old backspace/h behavior).
        let (mut s, c) = three_level_browser_state();
        // Pipeline (arena 1) is expanded. Select it.
        let pipeline_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| n.id == "pipeline")
            .unwrap();
        let pipeline_vis = s
            .browser
            .visible
            .iter()
            .position(|&i| i == pipeline_arena)
            .unwrap();
        s.browser.selected = pipeline_vis;
        assert!(s.browser.expanded.contains(&pipeline_arena));
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert!(
            !s.browser.expanded.contains(&pipeline_arena),
            "Left on expanded PG should collapse it"
        );
    }

    #[test]
    fn right_key_expands_collapsed_pg_in_tree_focus() {
        // Right expands a collapsed PG (replaces old Enter/Right behavior).
        let (mut s, c) = three_level_browser_state();
        // Collapse Pipeline first.
        s.browser.expanded.remove(&1);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        let pipeline_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| n.id == "pipeline")
            .unwrap();
        let pipeline_vis = s
            .browser
            .visible
            .iter()
            .position(|&i| i == pipeline_arena)
            .unwrap();
        s.browser.selected = pipeline_vis;
        let before = s.browser.visible.clone();
        update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
        assert_ne!(s.browser.visible, before, "Right should expand the PG");
    }

    #[test]
    fn enter_on_collapsed_pg_expands_and_moves_to_child() {
        // Enter (Descend) on a collapsed PG expands and selects the first child.
        let (mut s, c) = three_level_browser_state();
        // Collapse Pipeline so Enter on it will expand.
        s.browser.expanded.remove(&1);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        let pipeline_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| n.id == "pipeline")
            .unwrap();
        let pipeline_vis = s
            .browser
            .visible
            .iter()
            .position(|&i| i == pipeline_arena)
            .unwrap();
        s.browser.selected = pipeline_vis;
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(
            s.browser.expanded.contains(&pipeline_arena),
            "Enter should expand the PG"
        );
    }

    #[test]
    fn esc_on_expanded_pg_in_tree_collapses_it() {
        // Ascend (Esc) in tree focus on an expanded PG collapses it.
        let (mut s, c) = three_level_browser_state();
        // Pipeline (arena 1) is expanded. Select it.
        let pipeline_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| n.id == "pipeline")
            .unwrap();
        let pipeline_vis = s
            .browser
            .visible
            .iter()
            .position(|&i| i == pipeline_arena)
            .unwrap();
        s.browser.selected = pipeline_vis;
        assert!(s.browser.expanded.contains(&pipeline_arena));
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(
            !s.browser.expanded.contains(&pipeline_arena),
            "Esc (Ascend) on expanded PG should collapse it"
        );
    }

    #[test]
    fn tree_nav_uses_arrows_only_no_jk() {
        let (mut s, c) = seeded_browser_state();

        // j is dropped — firing it leaves selection unchanged.
        let before = s.browser.selected;
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.selected, before,
            "j should no longer move the cursor"
        );

        // Down still works.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert!(s.browser.selected > before, "Down should move the cursor");

        // k is dropped — firing it leaves selection unchanged.
        let before = s.browser.selected;
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.selected, before,
            "k should no longer move the cursor"
        );

        // Up still works.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        assert!(s.browser.selected < before, "Up should move the cursor");
    }

    // -----------------------------------------------------------------------
    // Helpers for Task 11-14 tests
    // -----------------------------------------------------------------------

    /// AppState with current_tab = Browser, selection on the "gen" Processor
    /// (visible row 1 in seeded_browser_state).
    fn fresh_browser_on_processor() -> (AppState, crate::config::Config) {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1; // "gen" Processor
        (s, c)
    }

    // -----------------------------------------------------------------------
    // Task 11: Focus cycle — Enter=descend, Right/Left=section, Esc=ascend
    // -----------------------------------------------------------------------

    #[test]
    fn enter_on_processor_enters_detail_focus_at_section_zero() {
        // Enter (Descend) on a leaf Processor enters detail focus at section 0.
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { idx, .. } => {
                assert_eq!(*idx, 0)
            }
            crate::view::browser::state::DetailFocus::Tree => {
                panic!("expected Section focus, got Tree")
            }
        }
    }

    #[test]
    fn right_is_noop_in_section_focus() {
        // Right is unmapped in section focus — use NextPane (Tab) to cycle sections.
        let (mut s, c) = fresh_browser_on_processor();
        // Enter detail focus (Section{0}), then press Right twice — idx must stay 0.
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { idx, .. } => {
                assert_eq!(*idx, 0, "Right must be a no-op in section focus")
            }
            _ => panic!("expected Section focus"),
        }
    }

    #[test]
    fn esc_returns_to_tree_focus_from_detail() {
        // Esc (Ascend) in Section focus returns to Tree focus.
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree
        );
    }

    #[test]
    fn moving_tree_selection_resets_detail_focus() {
        // Moving the tree cursor while in detail focus resets focus to Tree.
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(matches!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Section { .. }
        ));
        // Return to tree focus (Esc), then move down.
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree
        );
    }

    #[test]
    fn enter_on_pg_does_not_enter_section_focus() {
        // Enter (Descend) on a ProcessGroup expands it — it does NOT enter
        // detail section focus.
        let (mut s, c) = seeded_browser_state();
        // Confirm we're on a PG (root, selected=0), and it's already expanded.
        // Collapse it first so Enter will expand.
        s.browser.expanded.remove(&0);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.browser.selected = 0;
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        // Must still be in Tree focus — not Section focus.
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree,
            "Enter on PG should expand, not enter section focus"
        );
        // And the PG should now be expanded.
        assert!(
            s.browser.expanded.contains(&0),
            "PG should be expanded after Enter"
        );
    }

    // -----------------------------------------------------------------------
    // Task 12: Arrow-key row nav inside focused sections
    // -----------------------------------------------------------------------

    /// AppState with selection on the "gen" Processor (arena 1, visible 1)
    /// and a NodeDetail::Processor seeded with 3 properties.
    fn fresh_browser_on_processor_with_properties() -> (AppState, crate::config::Config) {
        use crate::client::ProcessorDetail;
        use crate::view::browser::state::NodeDetail;

        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1;
        s.browser.details.insert(
            1,
            NodeDetail::Processor(ProcessorDetail {
                id: "gen".into(),
                name: "Gen".into(),
                type_name: "x".into(),
                bundle: String::new(),
                run_status: "Running".into(),
                scheduling_strategy: String::new(),
                scheduling_period: String::new(),
                concurrent_tasks: 1,
                run_duration_ms: 0,
                penalty_duration: String::new(),
                yield_duration: String::new(),
                bulletin_level: String::new(),
                properties: vec![
                    ("alpha".into(), "one".into()),
                    ("beta".into(), "two".into()),
                    ("gamma".into(), "three".into()),
                ],
                validation_errors: vec![],
            }),
        );
        (s, c)
    }

    #[test]
    fn down_inside_focused_properties_advances_row() {
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        // Enter detail focus on Properties (section 0) via Descend (Enter).
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { idx, rows, .. } => {
                assert_eq!(*idx, 0);
                assert_eq!(rows[0], 1);
            }
            _ => panic!("expected Section focus"),
        }
    }

    #[test]
    fn up_inside_focused_properties_clamps_at_zero() {
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { rows, .. } => {
                assert_eq!(rows[0], 0, "clamped at 0")
            }
            _ => panic!("expected Section focus"),
        }
    }

    #[test]
    fn down_inside_focused_properties_clamps_at_max() {
        use crate::view::browser::state::DetailSection;

        let (mut s, c) = fresh_browser_on_processor_with_properties();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        for _ in 0..100 {
            update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        }
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { rows, .. } => {
                let max = s
                    .browser
                    .section_len(DetailSection::Properties, &s.bulletins.ring);
                assert_eq!(rows[0], max.saturating_sub(1), "clamped at max-1");
            }
            _ => panic!("expected Section focus"),
        }
    }

    // -----------------------------------------------------------------------
    // Task 13: c copy in focused sections
    // -----------------------------------------------------------------------

    #[test]
    fn c_in_focused_properties_copies_value_and_emits_banner() {
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        // Enter detail focus on Properties (section 0) via Descend (Enter).
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);

        // The banner must start with "copied" (success) or "clipboard" (failure).
        // We can't assert the clipboard was set in a headless test, but we can
        // assert the reducer produced an Info or Warning banner.
        let banner = s.status.banner.as_ref().expect("banner set after c");
        assert!(
            banner.message.starts_with("copied") || banner.message.starts_with("clipboard"),
            "banner = {}",
            banner.message
        );
    }

    /// Regression test for the arboard X11 `Drop` teardown bug that
    /// corrupted the ratatui alt-screen grid on every `c` keypress.
    /// Verifies that `AppState::clipboard` starts as `None` (lazy
    /// init), the reducer sets a banner on `c`, and the second `c`
    /// press does not panic — the handle is reused if the first
    /// succeeded, and re-attempted if the first failed (e.g. headless
    /// CI with no X display).
    #[test]
    fn c_lazily_initializes_and_reuses_persistent_clipboard_handle() {
        let (mut s, c) = seeded_browser_state();

        // Before any c press, the clipboard handle is None.
        assert!(
            s.clipboard.is_none(),
            "clipboard should be lazily initialized, not eager",
        );

        // First c press: a banner should appear (Info on success or
        // Warning on failure). In headless CI with no X display the
        // arboard call may fail — that's fine, we're testing lazy
        // init and banner emission, not clipboard success.
        update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);
        assert!(
            s.status.banner.is_some(),
            "banner should be set after first c press",
        );

        // Second c press: the reducer reuses the handle if the first
        // succeeded, retries lazy init if the first failed. Either
        // path must not panic and must still produce a banner.
        update(&mut s, key(KeyCode::Char('c'), KeyModifiers::NONE), &c);
        assert!(
            s.status.banner.is_some(),
            "banner should still be set after second c press",
        );
    }

    // -----------------------------------------------------------------------
    // Task 14: t cross-link on focused bulletin rows
    // -----------------------------------------------------------------------

    /// AppState with selection on "gen" Processor, a populated NodeDetail, and
    /// one matching bulletin in the ring.
    fn fresh_browser_on_processor_with_bulletins() -> (AppState, crate::config::Config) {
        use crate::client::BulletinSnapshot;

        let (mut s, c) = fresh_browser_on_processor_with_properties();
        s.bulletins.ring.push_back(BulletinSnapshot {
            id: 1,
            message: "test bulletin".into(),
            source_id: "gen".into(),
            source_name: "Gen".into(),
            group_id: "root".into(),
            source_type: "PROCESSOR".into(),
            level: "WARNING".into(),
            timestamp_iso: String::new(),
            timestamp_human: "00:00:00".into(),
        });
        (s, c)
    }

    #[test]
    fn t_is_noop_in_focused_recent_bulletins() {
        // `t` is retired; it is now a no-op in both tree and detail focus.
        // Users use `g e` (GoTarget::Events) for cross-tab gotos instead.
        let (mut s, c) = fresh_browser_on_processor_with_bulletins();
        // Enter detail focus on Properties (section 0), then cycle to
        // RecentBulletins (section 1) via Right.
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);

        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        assert!(
            r.intent.is_none(),
            "t must be a no-op in detail focus; got {r:?}"
        );
    }

    #[test]
    fn t_is_noop_in_focused_properties_section() {
        // `t` is retired; no intent emitted from Properties section focus either.
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        assert!(
            r.intent.is_none(),
            "t must be a no-op in section focus; got {r:?}"
        );
    }

    #[test]
    fn tree_drill_uses_enter_or_right_only_no_l_alias() {
        // Use three_level_browser_state: root(0) expanded, pipeline(1) expanded,
        // gen(2). Collapse pipeline so Enter on it will expand and change visible.
        let (mut s, c) = three_level_browser_state();
        s.browser.expanded.remove(&1);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        // Visible is now [0, 1] (root expanded, pipeline collapsed).
        s.browser.selected = 1; // pipeline (collapsed PG)
        let before_visible = s.browser.visible.clone();

        // `l` no longer drills in.
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.visible, before_visible,
            "l should no longer drill into a PG"
        );

        // Enter still drills in (expands pipeline, adds gen to visible).
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert_ne!(
            s.browser.visible, before_visible,
            "Enter should still drill in"
        );
    }

    #[test]
    fn t_on_focused_pg_recent_bulletins_emits_crosslink_for_row_source() {
        use crate::client::{BulletinSnapshot, ProcessGroupDetail};
        use crate::view::browser::state::{DetailFocus, NodeDetail};

        let (mut s, c) = seeded_browser_state();
        // Put the tree cursor on `root` (arena idx 0).
        s.browser.selected = s
            .browser
            .visible
            .iter()
            .position(|&i| i == 0)
            .expect("root visible");
        // Inject a PG detail for root so focused_row_source_id resolves.
        s.browser.details.insert(
            0,
            NodeDetail::ProcessGroup(ProcessGroupDetail {
                id: "root".into(),
                name: "root".into(),
                parent_group_id: None,
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                queued_display: "".into(),
                controller_services: vec![],
            }),
        );
        // Focus PG's RecentBulletins section (idx 2 per for_node(PG)).
        s.browser.detail_focus = DetailFocus::Section {
            idx: 2,
            rows: [0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };

        // Ring: newest at the back. Newest-first iteration → row 0 = p2.
        s.bulletins.ring.push_back(BulletinSnapshot {
            id: 1,
            level: "WARN".into(),
            message: "old".into(),
            source_id: "p1".into(),
            source_name: "p1".into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "".into(),
            timestamp_human: "".into(),
        });
        s.bulletins.ring.push_back(BulletinSnapshot {
            id: 2,
            level: "WARN".into(),
            message: "new".into(),
            source_id: "p2".into(),
            source_name: "p2".into(),
            source_type: "PROCESSOR".into(),
            group_id: "root".into(),
            timestamp_iso: "".into(),
            timestamp_human: "".into(),
        });

        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        // `t` is now a no-op; cross-tab goto is via `g e`.
        assert!(r.intent.is_none(), "t must be a no-op; got {r:?}");
    }

    #[test]
    fn tree_drill_out_uses_left_only_no_h_alias() {
        // `h` and Backspace are retired; only Left (Ascend) collapses a PG.
        let (mut s, c) = three_level_browser_state();
        let pipeline_arena = s
            .browser
            .nodes
            .iter()
            .position(|n| n.id == "pipeline")
            .unwrap();
        let pipeline_visible = s
            .browser
            .visible
            .iter()
            .position(|&i| i == pipeline_arena)
            .unwrap();
        s.browser.selected = pipeline_visible;
        let before_visible = s.browser.visible.clone();

        // `h` is a no-op.
        update(&mut s, key(KeyCode::Char('h'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.visible, before_visible,
            "h must no longer collapse a PG"
        );

        // Left collapses the expanded pipeline PG.
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert_ne!(
            s.browser.visible, before_visible,
            "Left should collapse the PG"
        );
    }

    #[test]
    fn enter_on_focused_pg_child_groups_drills_in() {
        use crate::client::ProcessGroupDetail;
        use crate::view::browser::state::{DetailFocus, NodeDetail};

        let (mut s, c) = seeded_browser_state();
        // Put the tree cursor on `root` (arena idx 0).
        s.browser.selected = s
            .browser
            .visible
            .iter()
            .position(|&i| i == 0)
            .expect("root visible");
        // Inject a PG detail for root so the handler can read `d.id`.
        s.browser.details.insert(
            0,
            NodeDetail::ProcessGroup(ProcessGroupDetail {
                id: "root".into(),
                name: "root".into(),
                parent_group_id: None,
                running: 0,
                stopped: 0,
                invalid: 0,
                disabled: 0,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
                queued_display: "".into(),
                controller_services: vec![],
            }),
        );
        // Focus PG's ChildGroups section, row 0 → `ingest`.
        s.browser.detail_focus = DetailFocus::Section {
            idx: 1,
            rows: [0, 0, 0, 0],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };

        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(r.redraw);
        // Tree cursor now on ingest (arena idx 2), not gen (arena idx 1).
        assert_eq!(s.browser.visible[s.browser.selected], 2);
        assert_eq!(s.browser.detail_focus, DetailFocus::Tree);
    }

    // -----------------------------------------------------------------------
    // Task 14: New typed-verb / typed-focus tests
    // -----------------------------------------------------------------------

    /// Seed a browser with a single Processor (gen, arena 1) and set
    /// `current_tab = Browser`. Mirrors `seeded_browser_state` but exposes
    /// the state at arena index 1 as a Processor.
    fn seed_browser_with_processor(s: &mut AppState) {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::event::{BrowserPayload, ViewPayload};
        use std::time::SystemTime;

        let c = tiny_config();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::Processor,
                    id: "gen".into(),
                    group_id: "root".into(),
                    name: "Gen".into(),
                    status_summary: NodeStatusSummary::Processor {
                        run_status: "Running".into(),
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(
            s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        s.current_tab = ViewId::Browser;
        s.browser.selected = 1; // "gen" Processor
    }

    /// Seed a browser with a ProcessGroup child. The root (arena 0) has
    /// one child PG named "ingest" (arena 1).
    fn seed_browser_with_child_pg(s: &mut AppState) {
        use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
        use crate::event::{BrowserPayload, ViewPayload};
        use std::time::SystemTime;

        let c = tiny_config();
        let snap = RecursiveSnapshot {
            nodes: vec![
                RawNode {
                    parent_idx: None,
                    kind: NodeKind::ProcessGroup,
                    id: "root".into(),
                    group_id: "root".into(),
                    name: "root".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
                RawNode {
                    parent_idx: Some(0),
                    kind: NodeKind::ProcessGroup,
                    id: "ingest".into(),
                    group_id: "root".into(),
                    name: "ingest".into(),
                    status_summary: NodeStatusSummary::ProcessGroup {
                        running: 0,
                        stopped: 0,
                        invalid: 0,
                        disabled: 0,
                    },
                },
            ],
            fetched_at: SystemTime::now(),
        };
        update(
            s,
            AppEvent::Data(ViewPayload::Browser(BrowserPayload::Tree(snap))),
            &c,
        );
        s.current_tab = ViewId::Browser;
        // Select root (arena 0) — the parent of the child PG.
        s.browser.selected = 0;
    }

    #[test]
    fn p_opens_properties_modal() {
        use crate::client::ProcessorDetail;
        use crate::input::{BrowserVerb, ViewVerb};
        use crate::view::browser::state::NodeDetail;

        let mut s = fresh_state();
        s.current_tab = ViewId::Browser;
        seed_browser_with_processor(&mut s);
        // Seed detail so the OpenProperties path has data.
        s.browser.details.insert(
            1,
            NodeDetail::Processor(ProcessorDetail {
                id: "gen".into(),
                name: "Gen".into(),
                type_name: "x".into(),
                bundle: String::new(),
                run_status: "Running".into(),
                scheduling_strategy: String::new(),
                scheduling_period: String::new(),
                concurrent_tasks: 1,
                run_duration_ms: 0,
                penalty_duration: String::new(),
                yield_duration: String::new(),
                bulletin_level: String::new(),
                properties: vec![],
                validation_errors: vec![],
            }),
        );

        BrowserHandler::handle_verb(&mut s, ViewVerb::Browser(BrowserVerb::OpenProperties));
        assert!(matches!(s.modal, Some(Modal::Properties(_))));
    }

    #[test]
    fn e_no_longer_opens_properties() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Browser;
        seed_browser_with_processor(&mut s);
        update(
            &mut s,
            AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
                KeyCode::Char('e'),
                KeyModifiers::NONE,
            ))),
            &c,
        );
        assert!(
            !matches!(s.modal, Some(Modal::Properties(_))),
            "e must no longer open Properties; p does"
        );
    }

    #[test]
    fn descend_on_process_group_expands() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Browser;
        seed_browser_with_child_pg(&mut s);
        // Root (arena 0) is already expanded by seed; collapse it first.
        s.browser.expanded.remove(&0);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.browser.selected = 0;
        let before = s.browser.expanded.len();
        BrowserHandler::handle_focus(&mut s, FocusAction::Descend);
        assert!(
            s.browser.expanded.len() > before,
            "Descend on PG must expand it"
        );
    }

    #[test]
    fn breadcrumb_mode_no_longer_triggered_by_b() {
        use crossterm::event::{KeyEvent, KeyModifiers};
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Browser;
        seed_browser_with_processor(&mut s);
        let before_selected = s.browser.selected;
        update(
            &mut s,
            AppEvent::Input(crossterm::event::Event::Key(KeyEvent::new(
                KeyCode::Char('b'),
                KeyModifiers::NONE,
            ))),
            &c,
        );
        // Interactive breadcrumb mode has been removed; `b` must be a no-op.
        assert_eq!(s.browser.selected, before_selected, "b must be a no-op");
    }

    // -----------------------------------------------------------------------
    // Task 7: NextPane/PrevPane cycle Tree → Section{0..n} → Tree
    // -----------------------------------------------------------------------

    #[test]
    fn next_pane_in_tree_enters_first_section_for_processor() {
        use crate::input::FocusAction;
        use crate::view::browser::state::DetailFocus;
        let (mut s, _c) = fresh_browser_on_processor();
        // Selection is on the "gen" Processor which has sections.
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.is_some(), "NextPane in Tree focus should return Some");
        match &s.browser.detail_focus {
            DetailFocus::Section { idx, .. } => {
                assert_eq!(*idx, 0, "NextPane from Tree should enter Section{{0}}")
            }
            DetailFocus::Tree => panic!("expected Section focus, got Tree"),
        }
    }

    #[test]
    fn prev_pane_in_tree_enters_last_section_for_processor() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, DetailSections};
        let (mut s, _c) = fresh_browser_on_processor();
        let arena_idx = s.browser.visible[s.browser.selected];
        let kind = s.browser.nodes[arena_idx].kind;
        let section_count = DetailSections::for_node(kind).len();
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert!(r.is_some(), "PrevPane in Tree focus should return Some");
        match &s.browser.detail_focus {
            DetailFocus::Section { idx, .. } => {
                assert_eq!(
                    *idx,
                    section_count - 1,
                    "PrevPane from Tree should enter Section{{last}}"
                )
            }
            DetailFocus::Tree => panic!("expected Section focus, got Tree"),
        }
    }

    #[test]
    fn next_pane_in_tree_enters_first_section_for_pg() {
        use crate::input::FocusAction;
        use crate::view::browser::state::DetailFocus;
        let (mut s, _c) = seeded_browser_state();
        // seeded_browser_state puts the root PG at selected=0 (arena 0).
        // ProcessGroup has 3 focusable sections (ControllerServices, ChildGroups, RecentBulletins).
        s.browser.selected = 0;
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.is_some(), "NextPane on PG should return Some");
        match &s.browser.detail_focus {
            DetailFocus::Section { idx, .. } => {
                assert_eq!(
                    *idx, 0,
                    "NextPane from Tree on PG should enter Section{{0}}"
                )
            }
            DetailFocus::Tree => panic!("expected Section focus, got Tree"),
        }
    }

    #[test]
    fn next_pane_in_section_advances_to_next_section() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
        let (mut s, _c) = fresh_browser_on_processor();
        // Enter section 0 manually.
        s.browser.detail_focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.is_some(), "NextPane in Section focus should return Some");
        match &s.browser.detail_focus {
            DetailFocus::Section { idx, .. } => {
                assert_eq!(
                    *idx, 1,
                    "NextPane from Section{{0}} should go to Section{{1}}"
                )
            }
            DetailFocus::Tree => panic!("expected Section focus, got Tree"),
        }
    }

    #[test]
    fn next_pane_from_last_section_wraps_to_tree() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, DetailSections, MAX_DETAIL_SECTIONS};
        let (mut s, _c) = fresh_browser_on_processor();
        let arena_idx = s.browser.visible[s.browser.selected];
        let kind = s.browser.nodes[arena_idx].kind;
        let last_idx = DetailSections::for_node(kind).len() - 1;
        s.browser.detail_focus = DetailFocus::Section {
            idx: last_idx,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::NextPane);
        assert!(r.is_some(), "NextPane from last section should return Some");
        assert_eq!(
            s.browser.detail_focus,
            DetailFocus::Tree,
            "NextPane from last section must wrap back to Tree"
        );
    }

    #[test]
    fn prev_pane_from_section_zero_wraps_to_tree() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
        let (mut s, _c) = fresh_browser_on_processor();
        s.browser.detail_focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert!(r.is_some(), "PrevPane from Section{{0}} should return Some");
        assert_eq!(
            s.browser.detail_focus,
            DetailFocus::Tree,
            "PrevPane from Section{{0}} must wrap to Tree"
        );
    }

    #[test]
    fn prev_pane_in_section_goes_to_previous_section() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
        let (mut s, _c) = fresh_browser_on_processor();
        s.browser.detail_focus = DetailFocus::Section {
            idx: 1,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::PrevPane);
        assert!(r.is_some(), "PrevPane from Section{{1}} should return Some");
        match &s.browser.detail_focus {
            DetailFocus::Section { idx, .. } => {
                assert_eq!(
                    *idx, 0,
                    "PrevPane from Section{{1}} should go to Section{{0}}"
                )
            }
            DetailFocus::Tree => panic!("expected Section focus, got Tree"),
        }
    }

    #[test]
    fn left_right_scroll_in_section_focus() {
        use crate::input::FocusAction;
        use crate::view::browser::state::{DetailFocus, MAX_DETAIL_SECTIONS};
        let (mut s, _c) = fresh_browser_on_processor();
        s.browser.detail_focus = DetailFocus::Section {
            idx: 0,
            rows: [0; MAX_DETAIL_SECTIONS],
            x_offsets: [0; MAX_DETAIL_SECTIONS],
        };

        // Right increments x_offsets[idx].
        let r = BrowserHandler::handle_focus(&mut s, FocusAction::Right);
        assert!(r.is_some(), "Right must return Some in Section focus");
        assert!(
            matches!(
                s.browser.detail_focus,
                DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 1
            ),
            "Right must increment x_offsets[0]"
        );

        // Left decrements back to 0.
        BrowserHandler::handle_focus(&mut s, FocusAction::Left);
        assert!(
            matches!(
                s.browser.detail_focus,
                DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 0
            ),
            "Left must decrement x_offsets[0]"
        );

        // Left at 0 stays at 0 (saturating).
        BrowserHandler::handle_focus(&mut s, FocusAction::Left);
        assert!(
            matches!(
                s.browser.detail_focus,
                DetailFocus::Section { x_offsets, .. } if x_offsets[0] == 0
            ),
            "Left at 0 must not underflow"
        );
    }
}
