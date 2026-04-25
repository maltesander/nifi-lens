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
        // VersionControlModal verbs take priority when the modal is open.
        if let ViewVerb::VersionControlModal(v) = verb {
            return Some(handle_version_control_modal_verb(state, v));
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
            BrowserVerb::ShowVersionControl => {
                if !state.browser_selection_is_versioned_pg() {
                    // Defensive — the verb's `enabled` predicate should
                    // already gray it out. Surface a banner if we got
                    // here anyway (e.g. the snapshot raced ahead).
                    state.post_warning("not under version control".to_string());
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
                    // row's value resolves to a known arena node.
                    if sections.0.get(idx) == Some(&DetailSection::Properties) {
                        let value = match state.browser.details.get(&arena_idx) {
                            Some(NodeDetail::Processor(p)) => {
                                p.properties.get(rows[idx]).map(|(_, v)| v.clone())
                            }
                            Some(NodeDetail::ControllerService(cs)) => {
                                cs.properties.get(rows[idx]).map(|(_, v)| v.clone())
                            }
                            _ => None,
                        };
                        if let Some(v) = value
                            && let Some(r) = state.browser.resolve_id(&v)
                        {
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
                let sections = DetailSections::for_node(kind);
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
