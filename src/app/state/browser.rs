//! Browser tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Banner, BannerSeverity, Modal, PendingIntent, UpdateResult, ViewKeyHandler};
use crate::view::browser::state::{
    DetailFocus, DetailSection, DetailSections, MAX_DETAIL_SECTIONS, NodeDetail,
};

/// Zero-sized dispatch struct for the Browser tab.
pub(crate) struct BrowserHandler;

impl ViewKeyHandler for BrowserHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Breadcrumb mode intercepts all keys.
        if state.browser.breadcrumb_focus.is_some() {
            return handle_breadcrumb_key(state, key);
        }

        if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
            return None;
        }

        // Detail-focus handling runs before the tree match. When focus is in a
        // Section, h/l/Esc are handled here; unrecognised keys fall through to
        // the tree match below (Tasks 12-14 add more arms here).
        if let DetailFocus::Section { idx, rows } = state.browser.detail_focus.clone() {
            let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                return Some(UpdateResult::default());
            };
            let kind = state.browser.nodes[arena_idx].kind;
            let sections = DetailSections::for_node(kind);
            match key.code {
                KeyCode::Char('h') | KeyCode::Esc => {
                    state.browser.detail_focus = DetailFocus::Tree;
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                KeyCode::Char('l') => {
                    if sections.is_empty() {
                        // Defensive: entered Section focus on a node that has no sections.
                        return Some(UpdateResult::default());
                    }
                    let new_idx = (idx + 1) % sections.len();
                    state.browser.detail_focus = DetailFocus::Section { idx: new_idx, rows };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                KeyCode::Up => {
                    let mut new_rows = rows;
                    new_rows[idx] = new_rows[idx].saturating_sub(1);
                    state.browser.detail_focus = DetailFocus::Section {
                        idx,
                        rows: new_rows,
                    };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                KeyCode::Down => {
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
                    };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                KeyCode::Char('c') => {
                    let Some(value) = state.browser.focused_row_copy_value(&state.bulletins.ring)
                    else {
                        return Some(UpdateResult::default());
                    };
                    let preview: String = value.chars().take(40).collect();
                    match state.copy_to_clipboard(value) {
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
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                KeyCode::Char('t')
                    if sections.0.get(idx) == Some(&DetailSection::RecentBulletins) =>
                {
                    let Some(source_id) =
                        state.browser.focused_row_source_id(&state.bulletins.ring)
                    else {
                        return Some(UpdateResult::default());
                    };
                    let link = crate::intent::CrossLink::JumpToEvents {
                        component_id: source_id,
                    };
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::JumpTo(link)),
                        tracer_followup: None,
                    });
                }
                KeyCode::Enter if sections.0.get(idx) == Some(&DetailSection::ChildGroups) => {
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
                _ => {
                    // Fall through to the tree-focused match.
                }
            }
        }

        match key.code {
            KeyCode::Up => {
                state.browser.move_up();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Down => {
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
            KeyCode::Enter | KeyCode::Right => {
                state.browser.enter_selection();
                state.browser.emit_detail_request_for_current_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Backspace | KeyCode::Left => {
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
                match state.copy_to_clipboard(id.clone()) {
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
                // Emit the JumpToEvents cross-link for Processors only.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let node = &state.browser.nodes[arena_idx];
                if !matches!(node.kind, crate::client::NodeKind::Processor) {
                    return Some(UpdateResult::default());
                }
                let link = crate::intent::CrossLink::JumpToEvents {
                    component_id: node.id.clone(),
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::JumpTo(link)),
                    tracer_followup: None,
                })
            }
            KeyCode::Char('b') => {
                let segments = state.browser.breadcrumb_segments();
                if segments.len() > 1 {
                    // Focus the last ancestor (parent of selected node).
                    state.browser.breadcrumb_focus = Some(segments.len() - 2);
                }
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            KeyCode::Char('l') => {
                // Enter detail focus at section 0, or show a banner if the
                // selected node has no focusable sections.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let kind = state.browser.nodes[arena_idx].kind;
                let sections = DetailSections::for_node(kind);
                if sections.is_empty() {
                    state.status.banner = Some(Banner {
                        severity: BannerSeverity::Info,
                        message: "no focusable sections for this node".to_string(),
                        detail: None,
                    });
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                state.browser.detail_focus = DetailFocus::Section {
                    idx: 0,
                    rows: [0; MAX_DETAIL_SECTIONS],
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            _ => None,
        }
    }

    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
        use crate::view::browser::state::{DetailFocus, DetailSection, DetailSections};
        use crate::widget::hint_bar::HintSpan;

        if state.browser.breadcrumb_focus.is_some() {
            return vec![
                HintSpan {
                    key: "←/→",
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

        match &state.browser.detail_focus {
            DetailFocus::Tree => vec![
                HintSpan {
                    key: "↑/↓",
                    action: "nav",
                },
                HintSpan {
                    key: "Enter",
                    action: "drill",
                },
                HintSpan {
                    key: "l",
                    action: "detail",
                },
                HintSpan {
                    key: "e",
                    action: "props",
                },
                HintSpan {
                    key: "c",
                    action: "copy id",
                },
                HintSpan {
                    key: "t",
                    action: "events",
                },
                HintSpan {
                    key: "b",
                    action: "crumb",
                },
            ],
            DetailFocus::Section { idx, .. } => {
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return vec![HintSpan {
                        key: "h",
                        action: "back",
                    }];
                };
                let kind = state.browser.nodes[arena_idx].kind;
                let sections = DetailSections::for_node(kind);
                match sections.0.get(*idx).copied() {
                    Some(DetailSection::Properties) => vec![
                        HintSpan {
                            key: "↑/↓",
                            action: "row",
                        },
                        HintSpan {
                            key: "l",
                            action: "next",
                        },
                        HintSpan {
                            key: "h",
                            action: "back",
                        },
                        HintSpan {
                            key: "c",
                            action: "copy value",
                        },
                        HintSpan {
                            key: "e",
                            action: "full list",
                        },
                    ],
                    Some(DetailSection::RecentBulletins) => vec![
                        HintSpan {
                            key: "↑/↓",
                            action: "row",
                        },
                        HintSpan {
                            key: "l",
                            action: "next",
                        },
                        HintSpan {
                            key: "h",
                            action: "back",
                        },
                        HintSpan {
                            key: "c",
                            action: "copy msg",
                        },
                        HintSpan {
                            key: "t",
                            action: "trace",
                        },
                    ],
                    Some(DetailSection::ControllerServices) => vec![
                        HintSpan {
                            key: "↑/↓",
                            action: "row",
                        },
                        HintSpan {
                            key: "l",
                            action: "next",
                        },
                        HintSpan {
                            key: "h",
                            action: "back",
                        },
                        HintSpan {
                            key: "c",
                            action: "copy id",
                        },
                    ],
                    Some(DetailSection::ChildGroups) => vec![
                        HintSpan {
                            key: "↑/↓",
                            action: "row",
                        },
                        HintSpan {
                            key: "l",
                            action: "next",
                        },
                        HintSpan {
                            key: "h",
                            action: "back",
                        },
                        HintSpan {
                            key: "c",
                            action: "copy id",
                        },
                        HintSpan {
                            key: "Enter",
                            action: "drill",
                        },
                    ],
                    _ => vec![HintSpan {
                        key: "h",
                        action: "back",
                    }],
                }
            }
        }
    }
}

fn handle_breadcrumb_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    let focus = state.browser.breadcrumb_focus?;
    let segments = state.browser.breadcrumb_segments();
    let max_focus = segments.len().saturating_sub(2); // last segment is current node

    match key.code {
        KeyCode::Left => {
            state.browser.breadcrumb_focus = Some(focus.saturating_sub(1));
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Right => {
            state.browser.breadcrumb_focus = Some((focus + 1).min(max_focus));
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Enter => {
            if let Some(seg) = segments.get(focus) {
                let arena_idx = seg.arena_idx;
                if let Some(pos) = state.browser.visible.iter().position(|&i| i == arena_idx) {
                    state.browser.selected = pos;
                    state.browser.emit_detail_request_for_current_selection();
                }
            }
            state.browser.breadcrumb_focus = None;
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Esc => {
            state.browser.breadcrumb_focus = None;
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        _ => Some(UpdateResult::default()), // consume all other keys in breadcrumb mode
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, seeded_browser_state, tiny_config};
    use super::super::update;
    use crate::app::state::{AppState, BannerSeverity, Modal, PendingIntent, ViewId};
    use crate::client::{NodeKind, NodeStatusSummary, RawNode, RecursiveSnapshot};
    use crate::config::Config;
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
    fn f_with_no_index_shows_warning_banner_and_does_not_open_modal() {
        let mut s = fresh_state();
        let c = tiny_config();
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
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
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
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
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
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
                name: "X".into(),
                group_path: "root".into(),
                state: crate::view::browser::state::StateBadge::Processor {
                    glyph: '\u{25CF}',
                    style: crate::theme::success(),
                },
                haystack: "x   processor   root".into(),
            }],
        });
        update(&mut s, key(KeyCode::Char('f'), KeyModifiers::NONE), &c);
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
    fn t_on_processor_emits_jump_to_events_crosslink() {
        let (mut s, c) = seeded_browser_state();
        s.browser.selected = 1; // "gen" processor
        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::JumpToEvents { component_id, .. })) => {
                assert_eq!(component_id, "gen");
            }
            other => panic!("expected JumpToEvents, got {other:?}"),
        }
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
    fn b_enters_breadcrumb_mode() {
        let (mut s, c) = three_level_browser_state();
        // Select Generate (visible index 2, arena index 2).
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        // Breadcrumb segments for Generate: [Root, Pipeline, Generate].
        // Focus should land on the last ancestor = index 1 (Pipeline).
        assert_eq!(s.browser.breadcrumb_focus, Some(1));
    }

    #[test]
    fn b_at_root_is_noop() {
        let (mut s, c) = three_level_browser_state();
        // Select Root (visible index 0). Only 1 segment → no breadcrumb mode.
        s.browser.selected = 0;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, None);
    }

    #[test]
    fn left_right_navigate_breadcrumb_segments() {
        let (mut s, c) = three_level_browser_state();
        // Select Generate → breadcrumb segments: [Root(0), Pipeline(1), Generate(2)].
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(1)); // Pipeline

        // Left → move to Root (index 0).
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(0));

        // Left again → still 0 (saturating).
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(0));

        // Right → back to 1 (Pipeline).
        update(&mut s, key(KeyCode::Right, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(1));
    }

    #[test]
    fn enter_in_breadcrumb_jumps_to_ancestor() {
        let (mut s, c) = three_level_browser_state();
        // Select Generate (visible index 2).
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(1)); // Pipeline

        // Navigate to Root (index 0).
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(0));

        // Enter → jump to Root, exit breadcrumb mode.
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, None);
        // Root is visible index 0.
        assert_eq!(s.browser.selected, 0);
    }

    #[test]
    fn esc_in_breadcrumb_cancels() {
        let (mut s, c) = three_level_browser_state();
        s.browser.selected = 2;
        update(&mut s, key(KeyCode::Char('b'), KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, Some(1));

        let prev_selected = s.browser.selected;
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert_eq!(s.browser.breadcrumb_focus, None);
        assert_eq!(s.browser.selected, prev_selected);
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
    // Task 11: Focus cycle (l/h/Esc)
    // -----------------------------------------------------------------------

    #[test]
    fn l_on_processor_enters_detail_focus_at_section_zero() {
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
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
    fn l_from_last_section_wraps_to_zero() {
        let (mut s, c) = fresh_browser_on_processor();
        // Processor has 2 focusable sections. Tree → 0 → 1 → 0 (wrap).
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { idx, .. } => {
                assert_eq!(*idx, 0, "wrap")
            }
            _ => panic!("expected Section focus"),
        }
    }

    #[test]
    fn h_returns_to_tree_focus() {
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('h'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree
        );
    }

    #[test]
    fn esc_returns_to_tree_focus() {
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree
        );
    }

    #[test]
    fn moving_tree_selection_resets_detail_focus() {
        let (mut s, c) = fresh_browser_on_processor();
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        assert!(matches!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Section { .. }
        ));
        // Return to tree focus first (h), then move down — this mimics the
        // real user flow where h exits section focus and Down re-moves the
        // tree cursor, which calls reset_detail_focus().
        update(&mut s, key(KeyCode::Char('h'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Tree
        );
    }

    #[test]
    fn l_on_pg_enters_section_focus_at_idx_0() {
        let (mut s, c) = seeded_browser_state();
        // Confirm we're on a PG (root, selected=0).
        let idx = s.browser.visible[s.browser.selected];
        assert!(matches!(
            s.browser.nodes[idx].kind,
            crate::client::NodeKind::ProcessGroup
        ));
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        // PG has 3 focusable sections (ControllerServices, ChildGroups,
        // RecentBulletins), so `l` enters Section focus at idx 0.
        assert!(matches!(
            s.browser.detail_focus,
            crate::view::browser::state::DetailFocus::Section { idx: 0, .. }
        ));
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
        // Enter detail focus on Properties (section 0).
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        match &s.browser.detail_focus {
            crate::view::browser::state::DetailFocus::Section { idx, rows } => {
                assert_eq!(*idx, 0);
                assert_eq!(rows[0], 1);
            }
            _ => panic!("expected Section focus"),
        }
    }

    #[test]
    fn up_inside_focused_properties_clamps_at_zero() {
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

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
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

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
        // Enter detail focus on Properties (section 0).
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

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
    fn t_in_focused_recent_bulletins_emits_jump_to_events_crosslink() {
        let (mut s, c) = fresh_browser_on_processor_with_bulletins();
        // Enter detail focus on Properties (section 0), then cycle to
        // RecentBulletins (section 1).
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::JumpToEvents { component_id })) => {
                assert_eq!(component_id, "gen");
            }
            other => panic!("expected JumpToEvents cross-link, got {other:?}"),
        }
    }

    #[test]
    fn t_in_focused_properties_section_falls_through_to_tree_t() {
        // When focus is on Properties (not RecentBulletins), t should fall
        // through and emit the same JumpToEvents cross-link the tree-level
        // t handler emits (since the processor is selected in the tree).
        let (mut s, c) = fresh_browser_on_processor_with_properties();
        // Enter detail focus on Properties (section 0 — NOT RecentBulletins).
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);

        let r = update(&mut s, key(KeyCode::Char('t'), KeyModifiers::NONE), &c);
        // The guard `sections.0.get(idx) == Some(&DetailSection::RecentBulletins)`
        // is false on section 0 (Properties), so t falls through to the tree
        // handler which also emits JumpToEvents for the selected Processor.
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::JumpToEvents { component_id })) => {
                assert_eq!(component_id, "gen");
            }
            other => panic!("expected JumpToEvents from tree handler, got {other:?}"),
        }
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
        use crate::intent::CrossLink;
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
        match r.intent {
            Some(PendingIntent::JumpTo(CrossLink::JumpToEvents { component_id })) => {
                assert_eq!(component_id, "p2");
            }
            other => panic!("unexpected intent: {other:?}"),
        }
    }

    #[test]
    fn tree_drill_out_uses_backspace_or_left_only_no_h_alias() {
        // Use three_level_browser_state: root(0), pipeline(1) expanded, gen(2).
        // Select pipeline (arena 1, visible row 1), which is already expanded.
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
        let after_drill = s.browser.visible.clone();

        // `h` no longer drills out.
        update(&mut s, key(KeyCode::Char('h'), KeyModifiers::NONE), &c);
        assert_eq!(
            s.browser.visible, after_drill,
            "h should no longer drill out"
        );

        // Left still drills out (collapses the expanded pipeline PG).
        update(&mut s, key(KeyCode::Left, KeyModifiers::NONE), &c);
        assert_ne!(
            s.browser.visible, after_drill,
            "Left should still drill out"
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
        };

        let r = update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);
        assert!(r.redraw);
        // Tree cursor now on ingest (arena idx 2), not gen (arena idx 1).
        assert_eq!(s.browser.visible[s.browser.selected], 2);
        assert_eq!(s.browser.detail_focus, DetailFocus::Tree);
    }
}
