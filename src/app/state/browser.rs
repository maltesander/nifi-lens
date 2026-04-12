//! Browser tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Banner, BannerSeverity, Modal, PendingIntent, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Browser tab.
pub(crate) struct BrowserHandler;

impl ViewKeyHandler for BrowserHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }

        match key.code {
            KeyCode::Up | KeyCode::Char('k') => {
                state.browser.move_up();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Down | KeyCode::Char('j') => {
                state.browser.move_down();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::PageDown => {
                state.browser.page_down(10);
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::PageUp => {
                state.browser.page_up(10);
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Home => {
                state.browser.jump_home();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::End => {
                state.browser.jump_end();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Enter | KeyCode::Right | KeyCode::Char('l') => {
                state.browser.enter_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Backspace | KeyCode::Left | KeyCode::Char('h') => {
                state.browser.backspace_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('r') => {
                // Consume the force-tick oneshot. The worker is listening
                // and will fire an immediate tree fetch. Clearing the
                // sender prevents a second press from panicking.
                if let Some(tx) = state.browser.force_tick_tx.take() {
                    let _ = tx.send(());
                }
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('e') => {
                // Open Properties modal only for Processor / CS with
                // detail loaded. No-op otherwise.
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
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                Some(UpdateResult::default())
            }
            KeyCode::Char('c') => {
                // Copy selected node's id to clipboard.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let id = state.browser.nodes[arena_idx].id.clone();
                match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(id.clone())) {
                    Ok(()) => {
                        state.status.banner = Some(Banner {
                            severity: BannerSeverity::Info,
                            message: format!("copied id: {id}"),
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
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('t') => {
                // Emit the Phase 4 TraceComponent cross-link for Processors only.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let node = &state.browser.nodes[arena_idx];
                if !matches!(node.kind, crate::client::NodeKind::Processor) {
                    return Some(UpdateResult::default());
                }
                let link = crate::intent::CrossLink::TraceComponent {
                    component_id: node.id.clone(),
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::JumpTo(link)),
                    tracer_followup: None,
                })
            }
            _ => None,
        }
    }

    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
        use crate::widget::hint_bar::HintSpan;

        if state.browser.breadcrumb_focus.is_some() {
            return vec![
                HintSpan {
                    key: "h/l",
                    action: "nav",
                },
                HintSpan {
                    key: "Enter",
                    action: "jump",
                },
                HintSpan {
                    key: "Esc",
                    action: "cancel",
                },
            ];
        }

        vec![
            HintSpan {
                key: "j/k",
                action: "nav",
            },
            HintSpan {
                key: "Enter",
                action: "expand",
            },
            HintSpan {
                key: "h",
                action: "back",
            },
            HintSpan {
                key: "e",
                action: "props",
            },
            HintSpan {
                key: "c",
                action: "copy",
            },
            HintSpan {
                key: "b",
                action: "crumb",
            },
            HintSpan {
                key: "t",
                action: "trace",
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, seeded_browser_state, tiny_config};
    use super::super::update;
    use crate::app::state::{BannerSeverity, Modal, PendingIntent, ViewId};
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use crate::event::{AppEvent, BrowserPayload, ViewPayload};
    use crate::intent::CrossLink;
    use crate::view::browser::state::{FlowIndex, FlowIndexEntry, PropertiesModalState};
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
    fn on_browser_tab_j_moves_selection_down() {
        let (mut s, c) = seeded_browser_state();
        assert_eq!(s.browser.selected, 0);
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
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
    fn on_browser_tab_backspace_on_expanded_pg_collapses() {
        let (mut s, c) = seeded_browser_state();
        s.browser.expanded.insert(2);
        crate::view::browser::state::rebuild_visible(&mut s.browser);
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Backspace, KeyModifiers::NONE), &c);
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
    fn ctrl_f_with_no_index_shows_warning_banner_and_does_not_open_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
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
    fn ctrl_f_with_index_opens_fuzzy_find_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "p".into(),
                group_id: "root".into(),
                kind: NodeKind::Processor,
                display: "P   Processor   root".into(),
                haystack: "p   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
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
                display: "PutKafka   Processor   root".into(),
                haystack: "putkafka   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        update(&mut s, key(KeyCode::Char('p'), KeyModifiers::NONE), &c);
        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::OpenInBrowser { component_id, .. })) => {
                assert_eq!(component_id, "target");
            }
            other => panic!("expected JumpTo(OpenInBrowser), got {other:?}"),
        }
        assert!(s.modal.is_none());
    }

    #[test]
    fn fuzzy_find_modal_esc_closes_without_jumping() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.flow_index = Some(FlowIndex {
            entries: vec![FlowIndexEntry {
                id: "x".into(),
                group_id: "g".into(),
                kind: NodeKind::Processor,
                display: "X   Processor   root".into(),
                haystack: "x   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::CONTROL), &c);
        let r = update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert!(s.modal.is_none());
        assert!(r.intent.is_none());
    }

    #[test]
    fn e_on_processor_with_detail_opens_properties_modal() {
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
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        assert!(matches!(s.modal, Some(Modal::Properties(_))));
    }

    #[test]
    fn e_on_processor_without_detail_is_noop() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1;
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
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
    fn t_on_processor_emits_trace_component_crosslink() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1; // "gen" processor
        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::TraceComponent { component_id, .. })) => {
                assert_eq!(component_id, "gen");
            }
            other => panic!("expected TraceComponent, got {other:?}"),
        }
    }
}
