//! Browser tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

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
        // VersionControlModal verbs take priority when the modal is open.
        if let ViewVerb::VersionControlModal(v) = verb {
            return Some(handle_version_control_modal_verb(state, v));
        }
        // ParameterContextModal verbs take priority when the modal is open.
        if let ViewVerb::ParameterContextModal(v) = verb {
            return Some(handle_parameter_context_modal_verb(state, v));
        }
        let bv = match verb {
            ViewVerb::Browser(v) => v,
            _ => return None,
        };
        match bv {
            BrowserVerb::Refresh => {
                // Task 6: Browser's arena is rebuilt from the cluster
                // snapshot, so the old per-worker force-tick oneshot is
                // gone. Force-refresh now nudges every endpoint the
                // arena depends on — each per-endpoint `Notify` wakes
                // the sleeping fetch loop without jitter. A fresh
                // `ClusterUpdate` → `ClusterChanged` round trip rebuilds
                // the arena.
                use crate::cluster::ClusterEndpoint;
                state.cluster.force(ClusterEndpoint::RootPgStatus);
                state.cluster.force(ClusterEndpoint::ControllerServices);
                state.cluster.force(ClusterEndpoint::ConnectionsByPg);
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
            BrowserVerb::OpenParameterContext => {
                // The enabled() predicate gates this verb to PG rows that have
                // a bound parameter context. Add a defensive guard (belt-and-
                // suspenders against future refactors that bypass the enabled
                // check) mirroring the ShowVersionControl pattern.
                if !state.browser_selection_pg_has_parameter_context_binding() {
                    return Some(UpdateResult::default());
                }
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let pg_id = state.browser.nodes[arena_idx].id.clone();
                let pg_path = state.browser.nodes[arena_idx].name.clone();
                let bound_context_id = state
                    .browser
                    .parameter_context_ref_for(&pg_id)
                    .map(|r| r.id.clone());
                state
                    .browser
                    .open_parameter_context_modal(pg_id.clone(), pg_path, None);
                if let Some(bound_context_id) = bound_context_id {
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(super::PendingIntent::SpawnParameterContextModalFetch {
                            pg_id,
                            bound_context_id,
                        }),
                        tracer_followup: None,
                    });
                }
                // This branch is unreachable when the enabled() predicate is
                // respected; kept for defensive completeness.
                return Some(UpdateResult::default());
            }
            BrowserVerb::ShowVersionControl => {
                if !state.browser_selection_is_versioned_pg() {
                    // Defensive: `enabled()` prevents dispatch for non-versioned
                    // PG rows, so this branch is unreachable under normal
                    // operation. Keep the guard as belt-and-suspenders against
                    // future refactors that might bypass the enabled check.
                    return Some(UpdateResult::default());
                }
                state
                    .browser
                    .open_version_control_modal(&state.cluster.snapshot);
                let Some(modal) = state.browser.version_modal.as_ref() else {
                    return Some(UpdateResult::default());
                };
                let pg_id = modal.pg_id.clone();
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(super::PendingIntent::SpawnVersionControlModalFetch { pg_id }),
                    tracer_followup: None,
                });
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
            let has_validation = match state.browser.details.get(&arena_idx) {
                Some(NodeDetail::Processor(p)) => !p.validation_errors.is_empty(),
                Some(NodeDetail::ControllerService(cs)) => !cs.validation_errors.is_empty(),
                _ => false,
            };
            let sections =
                sections_for_arena_with_detail(&state.browser, arena_idx, has_validation);
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
                    // Endpoints (Connection detail): row 0 = FROM, row 1 = TO.
                    if sections.0.get(idx) == Some(&DetailSection::Endpoints) {
                        let Some(NodeDetail::Connection(c)) = state.browser.details.get(&arena_idx)
                        else {
                            return Some(UpdateResult::default());
                        };
                        let (component_id, group_id) = match rows[idx] {
                            0 => (c.source_id.clone(), c.source_group_id.clone()),
                            1 => (c.destination_id.clone(), c.destination_group_id.clone()),
                            _ => return Some(UpdateResult::default()),
                        };
                        let intent =
                            super::PendingIntent::Goto(crate::intent::CrossLink::OpenInBrowser {
                                component_id,
                                group_id,
                            });
                        return Some(UpdateResult {
                            redraw: true,
                            intent: Some(intent),
                            tracer_followup: None,
                        });
                    }
                    // Properties (Processor or CS): jump when the selected
                    // row's value resolves to a known arena node (UUID
                    // cross-link) or contains a #{name} parameter
                    // reference whose owning PG has a bound context.
                    if sections.0.get(idx) == Some(&DetailSection::Properties) {
                        let (value, owning_pg_id) = match state.browser.details.get(&arena_idx) {
                            Some(NodeDetail::Processor(p)) => {
                                let v = p.properties.get(rows[idx]).map(|(_, v)| v.clone());
                                let pg = state
                                    .browser
                                    .nodes
                                    .get(arena_idx)
                                    .map(|n| n.group_id.clone())
                                    .unwrap_or_default();
                                (v, pg)
                            }
                            Some(NodeDetail::ControllerService(cs)) => {
                                let v = cs.properties.get(rows[idx]).map(|(_, v)| v.clone());
                                let pg = cs.parent_group_id.clone().unwrap_or_default();
                                (v, pg)
                            }
                            _ => (None, String::new()),
                        };
                        if let Some(v) = value {
                            // UUID cross-link takes priority.
                            if let Some(r) = state.browser.resolve_id(&v) {
                                let intent = super::PendingIntent::Goto(
                                    crate::intent::CrossLink::OpenInBrowser {
                                        component_id: v.trim().to_string(),
                                        group_id: r.group_id,
                                    },
                                );
                                return Some(UpdateResult {
                                    redraw: true,
                                    intent: Some(intent),
                                    tracer_followup: None,
                                });
                            }
                            // Parameter reference cross-link.
                            if state
                                .browser
                                .parameter_context_ref_for(&owning_pg_id)
                                .is_some()
                            {
                                use crate::view::browser::render::{ParamRefScan, scan_param_refs};
                                let preselect = match scan_param_refs(&v) {
                                    ParamRefScan::Single { name } => Some(name),
                                    ParamRefScan::Multiple => None,
                                    ParamRefScan::None => {
                                        return Some(UpdateResult::default());
                                    }
                                };
                                let intent = super::PendingIntent::Goto(
                                    crate::intent::CrossLink::OpenParameterContextModal {
                                        pg_id: owning_pg_id,
                                        preselect,
                                    },
                                );
                                return Some(UpdateResult {
                                    redraw: true,
                                    intent: Some(intent),
                                    tracer_followup: None,
                                });
                            }
                        }
                        return Some(UpdateResult::default());
                    }
                    // On the ReferencingComponents section of a CS, emit a
                    // Goto intent so the Browser jumps to the referenced
                    // component (processor / other CS / etc.).
                    if sections.0.get(idx) == Some(&DetailSection::ReferencingComponents) {
                        let Some(NodeDetail::ControllerService(cs)) =
                            state.browser.details.get(&arena_idx)
                        else {
                            return Some(UpdateResult::default());
                        };
                        let Some(target) = cs.referencing_components.get(rows[idx]) else {
                            return Some(UpdateResult::default());
                        };
                        let intent =
                            super::PendingIntent::Goto(crate::intent::CrossLink::OpenInBrowser {
                                component_id: target.id.clone(),
                                group_id: target.group_id.clone(),
                            });
                        return Some(UpdateResult {
                            redraw: true,
                            intent: Some(intent),
                            tracer_followup: None,
                        });
                    }
                    // Connections (Processor detail): jump to opposite endpoint.
                    if sections.0.get(idx) == Some(&DetailSection::Connections) {
                        let proc_id = match state.browser.details.get(&arena_idx) {
                            Some(NodeDetail::Processor(p)) => p.id.clone(),
                            _ => return Some(UpdateResult::default()),
                        };
                        let edges = state.browser.connections_for_processor(&proc_id);
                        let Some(edge) = edges.get(rows[idx]) else {
                            return Some(UpdateResult::default());
                        };
                        let intent =
                            super::PendingIntent::Goto(crate::intent::CrossLink::OpenInBrowser {
                                component_id: edge.opposite_id.clone(),
                                group_id: edge.opposite_group_id.clone(),
                            });
                        return Some(UpdateResult {
                            redraw: true,
                            intent: Some(intent),
                            tracer_followup: None,
                        });
                    }
                    // On the ControllerServices section of a PG, emit a
                    // Goto intent so the Browser jumps to the selected CS.
                    // CSes in a PG detail are always children of that PG in
                    // the arena, so we use the PG's id as the group_id.
                    if sections.0.get(idx) == Some(&DetailSection::ControllerServices) {
                        let Some(NodeDetail::ProcessGroup(d)) =
                            state.browser.details.get(&arena_idx)
                        else {
                            return Some(UpdateResult::default());
                        };
                        let Some(cs) = d.controller_services.get(rows[idx]) else {
                            return Some(UpdateResult::default());
                        };
                        let intent =
                            super::PendingIntent::Goto(crate::intent::CrossLink::OpenInBrowser {
                                component_id: cs.id.clone(),
                                group_id: d.id.clone(),
                            });
                        return Some(UpdateResult {
                            redraw: true,
                            intent: Some(intent),
                            tracer_followup: None,
                        });
                    }
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
                    // ParameterContext section: open the parameter-context modal.
                    if sections.0.get(idx) == Some(&DetailSection::ParameterContext) {
                        let Some(NodeDetail::ProcessGroup(d)) =
                            state.browser.details.get(&arena_idx)
                        else {
                            return Some(UpdateResult::default());
                        };
                        let pg_id = d.id.clone();
                        let intent = super::PendingIntent::Goto(
                            crate::intent::CrossLink::OpenParameterContextModal {
                                pg_id,
                                preselect: None,
                            },
                        );
                        return Some(UpdateResult {
                            redraw: true,
                            intent: Some(intent),
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
                // On a folder row: expand it (no descent into sections).
                // Otherwise: expand the selected node / move to first child
                // (same as the old Enter/Right behavior).
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                if matches!(
                    state.browser.nodes[arena_idx].kind,
                    crate::client::NodeKind::Folder(_)
                ) {
                    state.browser.expanded.insert(arena_idx);
                    crate::view::browser::state::rebuild_visible(&mut state.browser);
                } else {
                    state.browser.enter_selection();
                    state.browser.emit_detail_request_for_current_selection();
                }
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            FocusAction::Left => {
                // On a folder row: collapse it. Otherwise: collapse the
                // current node or goto its parent.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                if matches!(
                    state.browser.nodes[arena_idx].kind,
                    crate::client::NodeKind::Folder(_)
                ) {
                    state.browser.expanded.remove(&arena_idx);
                    crate::view::browser::state::rebuild_visible(&mut state.browser);
                } else {
                    state.browser.backspace_selection();
                    state.browser.emit_detail_request_for_current_selection();
                }
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
                let sections = sections_for_arena(&state.browser, arena_idx);
                use crate::client::NodeKind as NK;
                if matches!(kind, NK::Folder(_)) {
                    if state.browser.expanded.contains(&arena_idx) {
                        state.browser.expanded.remove(&arena_idx);
                    } else {
                        state.browser.expanded.insert(arena_idx);
                    }
                    crate::view::browser::state::rebuild_visible(&mut state.browser);
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
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
                let sections = sections_for_arena(&state.browser, arena_idx);
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
                let sections = sections_for_arena(&state.browser, arena_idx);
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

    fn is_text_input_focused(state: &AppState) -> bool {
        // Version-control modal's search input.
        let vc_active = state
            .browser
            .version_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if vc_active {
            return true;
        }
        // Parameter-context modal's search input.
        state
            .browser
            .parameter_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false)
    }

    fn blocks_app_shortcuts(state: &AppState) -> bool {
        // While the search bar is capturing keys, F1-F5 / `?` / `:` /
        // Shift+F must NOT escape the modal — the user is mid-typing.
        Self::is_text_input_focused(state)
    }

    fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        // Route raw key events to the version-control modal's search
        // reducer methods (mirrors handle_content_modal_search_input
        // for the Tracer modal).
        let vc_active = state
            .browser
            .version_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if vc_active {
            match key.code {
                KeyCode::Esc => state.browser.version_modal_search_cancel(),
                KeyCode::Enter => state.browser.version_modal_search_commit(),
                KeyCode::Backspace => state.browser.version_modal_search_pop(),
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.browser.version_modal_search_push(ch);
                }
                _ => return None,
            }
            return Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            });
        }

        // Parameter-context modal's search input.
        let pc_active = state
            .browser
            .parameter_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if !pc_active {
            return None;
        }
        match key.code {
            KeyCode::Esc => state.browser.parameter_modal_search_cancel(),
            KeyCode::Enter => state.browser.parameter_modal_search_commit(),
            KeyCode::Backspace => state.browser.parameter_modal_search_pop(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                state.browser.parameter_modal_search_push(ch);
            }
            _ => return None,
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }
}

/// Return the `DetailSections` for the node at `arena_idx`, taking the
/// PG parameter-context binding into account. For ProcessGroup nodes this
/// calls `DetailSections::for_pg_node(has_param_ctx)` instead of the
/// generic `for_node(kind)`; all other kinds fall back to `for_node`.
fn sections_for_arena(
    browser: &crate::view::browser::state::BrowserState,
    arena_idx: usize,
) -> DetailSections {
    let Some(node) = browser.nodes.get(arena_idx) else {
        return DetailSections(&[]);
    };
    if matches!(node.kind, crate::client::NodeKind::ProcessGroup) {
        let has_param_ctx = node.parameter_context_ref.is_some();
        return DetailSections::for_pg_node(has_param_ctx);
    }
    DetailSections::for_node(node.kind)
}

/// Return the `DetailSections` for the node at `arena_idx`, including
/// `ValidationErrors` when the detail snapshot reports errors. PG nodes
/// use `for_pg_node` to conditionally include the ParameterContext section.
fn sections_for_arena_with_detail(
    browser: &crate::view::browser::state::BrowserState,
    arena_idx: usize,
    has_validation: bool,
) -> DetailSections {
    let Some(node) = browser.nodes.get(arena_idx) else {
        return DetailSections(&[]);
    };
    if matches!(node.kind, crate::client::NodeKind::ProcessGroup) {
        let has_param_ctx = node.parameter_context_ref.is_some();
        return DetailSections::for_pg_node(has_param_ctx);
    }
    DetailSections::for_node_detail(node.kind, has_validation)
}

/// Dispatch a `VersionControlModalVerb` action. Called when the
/// version-control modal is open and the keymap has routed
/// `ViewVerb::VersionControlModal(v)` here.
fn handle_version_control_modal_verb(
    state: &mut AppState,
    v: crate::input::VersionControlModalVerb,
) -> UpdateResult {
    use crate::input::VersionControlModalVerb as V;

    let redraw = || UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
    };

    match v {
        V::Close => {
            // Esc cancels an active search first; only when no search
            // is active does Esc close the modal.
            let has_search = state
                .browser
                .version_modal
                .as_ref()
                .is_some_and(|m| m.search.is_some());
            if has_search {
                state.browser.version_modal_search_cancel();
            } else {
                state.browser.close_version_control_modal();
            }
            redraw()
        }
        V::OpenSearch => {
            state.browser.version_modal_search_open();
            redraw()
        }
        V::SearchNext => {
            state.browser.version_modal_search_cycle_next();
            redraw()
        }
        V::SearchPrev => {
            state.browser.version_modal_search_cycle_prev();
            redraw()
        }
        V::Copy => {
            if let Some(text) = version_control_modal_copy_text(&state.browser) {
                let preview: String = text.chars().take(40).collect();
                match state.copy_to_clipboard(text) {
                    Ok(()) => state.post_info(format!("copied: {preview}")),
                    Err(err) => state.post_warning(format!("clipboard: {err}")),
                }
            }
            redraw()
        }
        V::ToggleEnvironmental => {
            state.browser.toggle_environmental();
            redraw()
        }
        V::Refresh => {
            // Re-spawn the worker. Refresh does NOT close the modal,
            // so we abort the previous handle directly here rather
            // than going through close_version_control_modal().
            let Some(modal) = state.browser.version_modal.as_mut() else {
                return UpdateResult::default();
            };
            modal.differences = crate::view::browser::state::VersionControlDifferenceLoad::Pending;
            let pg_id = modal.pg_id.clone();
            // Dropping the search index too — body is changing.
            modal.search = None;
            if let Some(h) = state.browser.version_modal_handle.take() {
                h.abort();
            }
            UpdateResult {
                redraw: true,
                intent: Some(super::PendingIntent::SpawnVersionControlModalFetch { pg_id }),
                tracer_followup: None,
            }
        }
        V::ScrollUp => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                // Content_rows is unknown here; pass usize::MAX so the
                // widget clamp degrades to "render is the source of
                // truth" — render pass updates `last_viewport_rows`
                // and re-clamps via page_down/jump_bottom on next tick.
                modal.scroll.vertical.scroll_by(-1, usize::MAX);
            }
            redraw()
        }
        V::ScrollDown => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                modal.scroll.vertical.scroll_by(1, usize::MAX);
            }
            redraw()
        }
        V::PageUp => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                modal.scroll.vertical.page_up();
            }
            redraw()
        }
        V::PageDown => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                modal.scroll.vertical.page_down(usize::MAX);
            }
            redraw()
        }
        V::Home => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                modal.scroll.vertical.jump_top();
            }
            redraw()
        }
        V::End => {
            if let Some(modal) = state.browser.version_modal.as_mut() {
                modal.scroll.vertical.jump_bottom(usize::MAX);
            }
            redraw()
        }
    }
}

/// Dispatch a `ParameterContextModalVerb` action. Called when the
/// parameter-context modal is open and the keymap has routed
/// `ViewVerb::ParameterContextModal(v)` here.
fn handle_parameter_context_modal_verb(
    state: &mut AppState,
    v: crate::input::ParameterContextModalVerb,
) -> UpdateResult {
    use crate::input::ParameterContextModalVerb as V;

    let redraw = || UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
    };

    match v {
        V::Close => {
            // Esc cancels an active search first; only when no search
            // is active does Esc close the modal.
            let has_search = state
                .browser
                .parameter_modal
                .as_ref()
                .is_some_and(|m| m.search.is_some());
            if has_search {
                state.browser.parameter_modal_search_cancel();
            } else {
                state.browser.close_parameter_context_modal();
            }
            redraw()
        }
        V::Refresh => {
            // Re-spawn the worker. Refresh does NOT close the modal,
            // so we abort the previous handle directly here rather
            // than going through close_parameter_context_modal().
            let modal = state.browser.parameter_modal.as_mut();
            let Some(modal) = modal else {
                return UpdateResult::default();
            };
            let pg_id = modal.originating_pg_id.clone();
            modal.load =
                crate::view::browser::state::parameter_context_modal::ParameterContextLoad::Loading;
            // Drop the search index too — body is changing.
            modal.search = None;
            if let Some(h) = state.browser.parameter_modal_handle.take() {
                h.abort();
            }
            // Re-look up the bound context id. The binding may have
            // updated since the modal was first opened.
            let bound_context_id = state
                .browser
                .parameter_context_ref_for(&pg_id)
                .map(|r| r.id.clone());
            if let Some(bound_context_id) = bound_context_id {
                UpdateResult {
                    redraw: true,
                    intent: Some(super::PendingIntent::SpawnParameterContextModalFetch {
                        pg_id,
                        bound_context_id,
                    }),
                    tracer_followup: None,
                }
            } else {
                state.browser.apply_parameter_context_modal_failed(
                    pg_id,
                    "no bound parameter context found".into(),
                );
                redraw()
            }
        }
        V::ToggleByContext => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.by_context_mode = !modal.by_context_mode;
                modal.scroll.jump_top();
            }
            redraw()
        }
        V::ToggleShadowed => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.show_shadowed = !modal.show_shadowed;
            }
            redraw()
        }
        V::ToggleUsedBy => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.show_used_by = !modal.show_used_by;
                modal.scroll.jump_top();
            }
            redraw()
        }
        V::RowUp => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.scroll_by(-1, usize::MAX);
            }
            redraw()
        }
        V::RowDown => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.scroll_by(1, usize::MAX);
            }
            redraw()
        }
        V::PageUp => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.page_up();
            }
            redraw()
        }
        V::PageDown => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.page_down(usize::MAX);
            }
            redraw()
        }
        V::JumpTop => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.jump_top();
            }
            redraw()
        }
        V::JumpBottom => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.scroll.jump_bottom(usize::MAX);
            }
            redraw()
        }
        V::ChainFocusLeft => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.sidebar_index = modal.sidebar_index.saturating_sub(1);
            }
            redraw()
        }
        V::ChainFocusRight => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                let chain_len = match &modal.load {
                    crate::view::browser::state::parameter_context_modal::ParameterContextLoad::Loaded {
                        chain,
                    } => chain.len(),
                    _ => 0,
                };
                if chain_len > 0 {
                    modal.sidebar_index =
                        (modal.sidebar_index + 1).min(chain_len.saturating_sub(1));
                }
            }
            redraw()
        }
        V::ChainEnter => {
            // Selecting a chain entry switches to by-context mode
            // scoped to the current sidebar_index.
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                modal.by_context_mode = true;
                modal.scroll.jump_top();
            }
            redraw()
        }
        V::Search => {
            state.browser.parameter_modal_search_open();
            redraw()
        }
        V::SearchNext => {
            state.browser.parameter_modal_search_cycle_next();
            redraw()
        }
        V::SearchPrev => {
            state.browser.parameter_modal_search_cycle_prev();
            redraw()
        }
        V::Copy => {
            if let Some(text) = parameter_context_modal_copy_text(&state.browser) {
                let preview: String = text.chars().take(40).collect();
                match state.copy_to_clipboard(text) {
                    Ok(()) => state.post_info(format!("copied: {preview}")),
                    Err(err) => state.post_warning(format!("clipboard: {err}")),
                }
            }
            redraw()
        }
    }
}

/// Build a plain-text rendering of the resolved parameter list for
/// clipboard copy. Returns `None` when the modal is not loaded.
fn parameter_context_modal_copy_text(
    browser: &crate::view::browser::state::BrowserState,
) -> Option<String> {
    use crate::view::browser::state::parameter_context_modal::{ParameterContextLoad, resolve};
    let modal = browser.parameter_modal.as_ref()?;
    let chain = match &modal.load {
        ParameterContextLoad::Loaded { chain } => chain,
        _ => return None,
    };
    let resolved = resolve(chain, modal.preselect.as_deref());
    if resolved.is_empty() {
        return None;
    }
    let mut out = String::new();
    for row in &resolved {
        let value = if row.winner.sensitive {
            "(sensitive)".to_string()
        } else {
            row.winner.value.clone().unwrap_or_default()
        };
        out.push_str(&format!("{}={}\n", row.winner.name, value));
    }
    Some(out)
}

/// Build a plain-text rendering of the modal's identity + diff body
/// for clipboard copy. Returns `None` when there is no content to
/// copy (modal closed, or modal open but identity absent and diff
/// still pending / failed).
fn version_control_modal_copy_text(
    browser: &crate::view::browser::state::BrowserState,
) -> Option<String> {
    let modal = browser.version_modal.as_ref()?;
    let mut out = String::new();
    if let Some(id) = &modal.identity {
        if let Some(name) = &id.flow_name {
            out.push_str(&format!("flow: {name}\n"));
        }
        if let Some(v) = &id.version {
            out.push_str(&format!("version: {v}\n"));
        }
        out.push_str(&format!("state: {:?}\n\n", id.state));
    }
    if let crate::view::browser::state::VersionControlDifferenceLoad::Loaded(sections) =
        &modal.differences
    {
        for s in sections {
            out.push_str(&format!(
                "{} · {} · {}\n",
                s.component_type, s.component_name, s.component_id
            ));
            for d in &s.differences {
                if !modal.show_environmental && d.environmental {
                    continue;
                }
                out.push_str(&format!("  {}: {}\n", d.kind, d.description));
            }
            out.push('\n');
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

#[cfg(test)]
mod tests;
