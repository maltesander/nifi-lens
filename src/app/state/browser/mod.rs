//! Browser tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Modal, UpdateResult, ViewKeyHandler};
use crate::input::{CommonVerb, FocusAction, ViewVerb};
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
        // ActionHistoryModal verbs take priority when the modal is open.
        if let ViewVerb::ActionHistoryModal(v) = verb {
            return Some(handle_action_history_modal_verb(state, v));
        }
        // BrowserQueueVerb chords are dispatched when the listing panel
        // has focus or the chord doesn't conflict with the tree-focus chord
        // set. The shadow gate inside the keymap (T9) ensures these only
        // fire on the Browser tab.
        if let ViewVerb::BrowserQueue(v) = verb {
            return Some(handle_browser_queue_verb(state, v));
        }
        // BrowserPeek verbs take priority when the peek modal is open. The
        // keymap shadow gate (T9) routes to BrowserPeekVerb only when
        // peek_modal_open == true, so reaching here implies the modal is open.
        if let ViewVerb::BrowserPeek(v) = verb {
            return Some(handle_browser_peek_verb(state, v));
        }
        let bv = match verb {
            ViewVerb::Browser(v) => v,
            _ => return None,
        };
        match bv {
            BrowserVerb::Common(CommonVerb::Refresh) => {
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
            BrowserVerb::Common(CommonVerb::Copy) => {
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    });
                }
                // This branch is unreachable when the enabled() predicate is
                // respected; kept for defensive completeness.
                return Some(UpdateResult::default());
            }
            BrowserVerb::OpenActionHistory => {
                // Defensive: enabled() gates this verb to UUID-bearing rows.
                // Belt-and-suspenders mirroring the ShowVersionControl pattern.
                if !state.browser_selection_supports_action_history() {
                    return Some(UpdateResult::default());
                }
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let node = &state.browser.nodes[arena_idx];
                let source_id = node.id.clone();
                let component_label = node.name.clone();
                state
                    .browser
                    .open_action_history_modal(source_id.clone(), component_label);
                let Some(fetch_signal) = state
                    .browser
                    .action_history_modal
                    .as_ref()
                    .map(|m| m.fetch_signal.clone())
                else {
                    return Some(UpdateResult::default());
                };
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(super::PendingIntent::SpawnActionHistoryModalFetch {
                        source_id,
                        fetch_signal,
                    }),
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                });
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
                });
            }
            // OpenSearch / SearchNext / SearchPrev / Close are not bound on
            // the Browser top-level — only inside the modals (handled by their
            // own dispatch above). These arms keep the match exhaustive.
            BrowserVerb::Common(
                CommonVerb::OpenSearch
                | CommonVerb::SearchNext
                | CommonVerb::SearchPrev
                | CommonVerb::Close,
            ) => {}
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
        // Listing-row focus: drives the row cursor inside the queue
        // listing panel. Up/Down step one row; PgUp/PgDn step ten;
        // Home/End jump. The keymap's listing-focus shadow gate passes
        // these FocusAction chords through (everything else dispatches
        // BrowserQueueVerb), so they always land here when listing
        // focus is active.
        if state.browser.listing_focused {
            let Some(listing) = state.browser.queue_listing.as_mut() else {
                return Some(UpdateResult::default());
            };
            let visible_count = listing.visible_indices().len();
            if visible_count == 0 {
                return Some(UpdateResult::default());
            }
            let last = visible_count - 1;
            match action {
                FocusAction::Up => {
                    listing.selected = listing.selected.saturating_sub(1);
                }
                FocusAction::Down => {
                    listing.selected = (listing.selected + 1).min(last);
                }
                FocusAction::PageUp => {
                    listing.selected = listing.selected.saturating_sub(10);
                }
                FocusAction::PageDown => {
                    listing.selected = (listing.selected + 10).min(last);
                }
                FocusAction::First => {
                    listing.selected = 0;
                }
                FocusAction::Last => {
                    listing.selected = last;
                }
                _ => return Some(UpdateResult::default()),
            }
            return Some(UpdateResult {
                redraw: true,
                ..Default::default()
            });
        }

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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
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
                        let (__sparkline_fu, __queue_listing_fu) =
                            if state.browser.drill_into_group(&target_id) {
                                state.browser.emit_detail_request_for_current_selection();
                                (
                                    state.refresh_sparkline_for_selection(),
                                    state.refresh_queue_listing_for_selection(),
                                )
                            } else {
                                (None, None)
                            };
                        return Some(UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: __sparkline_fu,
                            queue_listing_followup: __queue_listing_fu,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        });
                    }
                    // Other sections: no local descent.
                    None
                }
                FocusAction::NextPane => {
                    // Advance to the next section. If at the last
                    // section and a non-empty queue listing is
                    // present, hop into listing focus before wrapping
                    // back to Tree on the next press.
                    let section_count = sections.len();
                    if section_count == 0 {
                        return Some(UpdateResult::default());
                    }
                    let new_idx = idx + 1;
                    if new_idx >= section_count {
                        let listing_has_rows = state
                            .browser
                            .queue_listing
                            .as_ref()
                            .map(|s| !s.rows.is_empty())
                            .unwrap_or(false);
                        if listing_has_rows {
                            state.browser.detail_focus = DetailFocus::Tree;
                            state.browser.listing_focused = true;
                        } else {
                            state.browser.detail_focus = DetailFocus::Tree;
                        }
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::Down => {
                state.browser.move_down();
                state.browser.emit_detail_request_for_current_selection();
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::PageUp => {
                state.browser.page_up(10);
                state.browser.emit_detail_request_for_current_selection();
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::PageDown => {
                state.browser.page_down(10);
                state.browser.emit_detail_request_for_current_selection();
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::First => {
                state.browser.goto_first();
                state.browser.emit_detail_request_for_current_selection();
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::Last => {
                state.browser.goto_last();
                state.browser.emit_detail_request_for_current_selection();
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::Right => {
                // On a folder row: expand it (no descent into sections).
                // Otherwise: expand the selected node / move to first child
                // (same as the old Enter/Right behavior).
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let (__sparkline_fu, __queue_listing_fu) = if matches!(
                    state.browser.nodes[arena_idx].kind,
                    crate::client::NodeKind::Folder(_)
                ) {
                    state.browser.expanded.insert(arena_idx);
                    crate::view::browser::state::rebuild_visible(&mut state.browser);
                    (None, None)
                } else {
                    state.browser.enter_selection();
                    state.browser.emit_detail_request_for_current_selection();
                    (
                        state.refresh_sparkline_for_selection(),
                        state.refresh_queue_listing_for_selection(),
                    )
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
                })
            }
            FocusAction::Left => {
                // On a folder row: collapse it. Otherwise: collapse the
                // current node or goto its parent.
                let Some(&arena_idx) = state.browser.visible.get(state.browser.selected) else {
                    return Some(UpdateResult::default());
                };
                let (__sparkline_fu, __queue_listing_fu) = if matches!(
                    state.browser.nodes[arena_idx].kind,
                    crate::client::NodeKind::Folder(_)
                ) {
                    state.browser.expanded.remove(&arena_idx);
                    crate::view::browser::state::rebuild_visible(&mut state.browser);
                    (None, None)
                } else {
                    state.browser.backspace_selection();
                    state.browser.emit_detail_request_for_current_selection();
                    (
                        state.refresh_sparkline_for_selection(),
                        state.refresh_queue_listing_for_selection(),
                    )
                };
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    });
                }
                if matches!(kind, NK::ProcessGroup) {
                    state.browser.enter_selection();
                    state.browser.emit_detail_request_for_current_selection();
                    let __sparkline_fu = state.refresh_sparkline_for_selection();
                    let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: __sparkline_fu,
                        queue_listing_followup: __queue_listing_fu,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                let __sparkline_fu = state.refresh_sparkline_for_selection();
                let __queue_listing_fu = state.refresh_queue_listing_for_selection();
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: __sparkline_fu,
                    queue_listing_followup: __queue_listing_fu,
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
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
        let pc_active = state
            .browser
            .parameter_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if pc_active {
            return true;
        }
        // Action-history modal's search input.
        let ah_active = state
            .browser
            .action_history_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if ah_active {
            return true;
        }
        // Queue-listing peek-modal search input.
        let peek_search_active = state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|l| l.peek.as_ref())
            .and_then(|p| p.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if peek_search_active {
            return true;
        }
        // Queue-listing filter prompt.
        state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|l| l.filter_prompt.as_ref())
            .is_some()
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
                sparkline_followup: None,
                queue_listing_followup: None,
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
        if pc_active {
            match key.code {
                KeyCode::Esc => state.browser.parameter_modal_search_cancel(),
                KeyCode::Enter => state.browser.parameter_modal_search_commit(),
                KeyCode::Backspace => state.browser.parameter_modal_search_pop(),
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.browser.parameter_modal_search_push(ch);
                }
                _ => return None,
            }
            return Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            });
        }

        // Action-history modal's search input.
        let ah_active = state
            .browser
            .action_history_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if ah_active {
            match key.code {
                KeyCode::Esc => state.browser.action_history_modal_search_cancel(),
                KeyCode::Enter => state.browser.action_history_modal_search_commit(),
                KeyCode::Backspace => state.browser.action_history_modal_search_pop(),
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.browser.action_history_modal_search_push(ch);
                }
                _ => return None,
            }
            return Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            });
        }

        // Queue-listing peek-modal search input.
        let peek_search_active = state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|l| l.peek.as_ref())
            .and_then(|p| p.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if peek_search_active {
            match key.code {
                KeyCode::Esc => state.browser.peek_search_cancel(),
                KeyCode::Enter => state.browser.peek_search_commit(),
                KeyCode::Backspace => state.browser.peek_search_pop(),
                KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    state.browser.peek_search_push(ch);
                }
                _ => return None,
            }
            return Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            });
        }

        // Queue-listing filter prompt.
        let filter_active = state
            .browser
            .queue_listing
            .as_ref()
            .and_then(|l| l.filter_prompt.as_ref())
            .is_some();
        if !filter_active {
            return None;
        }
        let listing = state.browser.queue_listing.as_mut()?;
        match key.code {
            KeyCode::Esc => listing.cancel_filter_prompt(),
            KeyCode::Enter => listing.commit_filter_prompt(),
            KeyCode::Backspace => listing.backspace_filter_char(),
            KeyCode::Char(ch) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                listing.push_filter_char(ch);
            }
            _ => return None,
        }
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
            sparkline_followup: None,
            queue_listing_followup: None,
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
        sparkline_followup: None,
        queue_listing_followup: None,
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
                sparkline_followup: None,
                queue_listing_followup: None,
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
        sparkline_followup: None,
        queue_listing_followup: None,
    };

    use crate::view::browser::state::parameter_context_modal::ParameterContextPane;

    match v {
        V::Close => {
            // Priority: cancel search → unfocus Body → close modal.
            let has_search = state
                .browser
                .parameter_modal
                .as_ref()
                .is_some_and(|m| m.search.is_some());
            if has_search {
                state.browser.parameter_modal_search_cancel();
            } else if state
                .browser
                .parameter_modal
                .as_ref()
                .is_some_and(|m| m.focused_pane == ParameterContextPane::Body)
            {
                // Unfocus body → return to sidebar.
                if let Some(modal) = state.browser.parameter_modal.as_mut() {
                    modal.focused_pane = ParameterContextPane::Sidebar;
                }
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
                }
            } else {
                state.browser.apply_parameter_context_modal_failed(
                    pg_id,
                    "no bound parameter context found".into(),
                );
                redraw()
            }
        }
        V::FocusBody => {
            // Enter shifts focus from Sidebar → Body. Already in Body: no-op.
            if let Some(modal) = state.browser.parameter_modal.as_mut()
                && modal.focused_pane == ParameterContextPane::Sidebar
            {
                modal.focused_pane = ParameterContextPane::Body;
            }
            redraw()
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
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        modal.sidebar_index = modal.sidebar_index.saturating_sub(1);
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.scroll_by(-1, usize::MAX);
                    }
                }
            }
            redraw()
        }
        V::RowDown => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        let chain_len = match &modal.load {
                            crate::view::browser::state::parameter_context_modal::ParameterContextLoad::Loaded { chain } => chain.len(),
                            _ => 0,
                        };
                        if chain_len > 0 {
                            modal.sidebar_index =
                                (modal.sidebar_index + 1).min(chain_len.saturating_sub(1));
                        }
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.scroll_by(1, usize::MAX);
                    }
                }
            }
            redraw()
        }
        V::PageUp => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        modal.sidebar_index = 0;
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.page_up();
                    }
                }
            }
            redraw()
        }
        V::PageDown => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        let chain_len = match &modal.load {
                            crate::view::browser::state::parameter_context_modal::ParameterContextLoad::Loaded { chain } => chain.len(),
                            _ => 0,
                        };
                        if chain_len > 0 {
                            modal.sidebar_index = chain_len.saturating_sub(1);
                        }
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.page_down(usize::MAX);
                    }
                }
            }
            redraw()
        }
        V::JumpTop => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        modal.sidebar_index = 0;
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.jump_top();
                    }
                }
            }
            redraw()
        }
        V::JumpBottom => {
            if let Some(modal) = state.browser.parameter_modal.as_mut() {
                match modal.focused_pane {
                    ParameterContextPane::Sidebar => {
                        let chain_len = match &modal.load {
                            crate::view::browser::state::parameter_context_modal::ParameterContextLoad::Loaded { chain } => chain.len(),
                            _ => 0,
                        };
                        if chain_len > 0 {
                            modal.sidebar_index = chain_len.saturating_sub(1);
                        }
                    }
                    ParameterContextPane::Body => {
                        modal.scroll.jump_bottom(usize::MAX);
                    }
                }
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

/// Dispatch an `ActionHistoryModalVerb` action. Called when the
/// action-history modal is open and the keymap has routed
/// `ViewVerb::ActionHistoryModal(v)` here.
fn handle_action_history_modal_verb(
    state: &mut AppState,
    v: crate::input::ActionHistoryModalVerb,
) -> UpdateResult {
    use crate::input::ActionHistoryModalVerb as V;

    let redraw = || UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
        sparkline_followup: None,
        queue_listing_followup: None,
    };

    match v {
        V::Close => {
            // Priority cascade: cancel search → collapse expanded → close modal.
            let Some(modal) = state.browser.action_history_modal.as_mut() else {
                return UpdateResult::default();
            };
            if modal.search.is_some() {
                modal.search = None;
                return redraw();
            }
            if modal.expanded_index.is_some() {
                modal.expanded_index = None;
                return redraw();
            }
            state.browser.close_action_history_modal();
            redraw()
        }
        V::Refresh => {
            let Some(modal) = state.browser.action_history_modal.as_mut() else {
                return UpdateResult::default();
            };
            let source_id = modal.source_id.clone();
            modal.reset_for_refresh();
            let fetch_signal = modal.fetch_signal.clone();
            // Abort the current worker; the dispatcher spawns a fresh one.
            if let Some(h) = state.browser.action_history_modal_handle.take() {
                h.abort();
            }
            UpdateResult {
                redraw: true,
                intent: Some(super::PendingIntent::SpawnActionHistoryModalFetch {
                    source_id,
                    fetch_signal,
                }),
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        V::ToggleExpand => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                let selected = modal.selected;
                modal.toggle_expanded(selected);
            }
            redraw()
        }
        V::ScrollUp => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.move_selection_up();
            }
            redraw()
        }
        V::ScrollDown => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.move_selection_down();
                action_history_check_signal_next_page(modal);
            }
            redraw()
        }
        V::PageUp => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.scroll.page_up();
                let viewport = modal.scroll.last_viewport_rows.max(1);
                modal.selected = modal.selected.saturating_sub(viewport);
            }
            redraw()
        }
        V::PageDown => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                let len = modal.actions.len();
                modal.scroll.page_down(len);
                let viewport = modal.scroll.last_viewport_rows.max(1);
                modal.selected = (modal.selected + viewport).min(len.saturating_sub(1));
                action_history_check_signal_next_page(modal);
            }
            redraw()
        }
        V::JumpTop => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.scroll.jump_top();
                modal.selected = 0;
            }
            redraw()
        }
        V::JumpBottom => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                let len = modal.actions.len();
                modal.scroll.jump_bottom(len);
                modal.selected = len.saturating_sub(1);
                action_history_check_signal_next_page(modal);
            }
            redraw()
        }
        V::OpenSearch => {
            if let Some(modal) = state.browser.action_history_modal.as_mut() {
                modal.search = Some(crate::widget::search::SearchState {
                    input_active: true,
                    ..Default::default()
                });
            }
            redraw()
        }
        V::SearchNext => {
            if let Some(modal) = state.browser.action_history_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && !s.matches.is_empty()
            {
                s.current = Some(s.current.map_or(0, |c| (c + 1) % s.matches.len()));
            }
            redraw()
        }
        V::SearchPrev => {
            if let Some(modal) = state.browser.action_history_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && !s.matches.is_empty()
            {
                let len = s.matches.len();
                s.current = Some(s.current.map_or(len - 1, |c| (c + len - 1) % len));
            }
            redraw()
        }
        V::Copy => {
            // Capture the TSV before borrowing state mutably for
            // copy_to_clipboard (two borrows of state at once).
            let tsv = state
                .browser
                .action_history_modal
                .as_ref()
                .and_then(|m| m.actions.get(m.selected))
                .map(format_action_tsv);
            if let Some(tsv) = tsv
                && let Err(err) = state.copy_to_clipboard(tsv)
            {
                state.post_warning(format!("clipboard: {err}"));
            }
            redraw()
        }
    }
}

/// Dispatch a `BrowserQueueVerb` action.  Called when the keymap routes
/// `ViewVerb::BrowserQueue(v)` to the Browser tab handler.  `Refresh` drops
/// the prior listing request (triggering the `QueueListingHandle` Drop-DELETE)
/// and emits `PendingIntent::SpawnQueueListingRefresh` so the dispatcher can
/// post a fresh listing request.
fn handle_browser_queue_verb(
    state: &mut AppState,
    verb: crate::input::BrowserQueueVerb,
) -> UpdateResult {
    use crate::app::state::PendingIntent;
    use crate::input::BrowserQueueVerb;
    use crate::intent::CrossLink;

    match verb {
        BrowserQueueVerb::Common(CommonVerb::Refresh) => {
            let Some(listing) = state.browser.queue_listing.as_mut() else {
                return UpdateResult::default();
            };
            // Drop the prior handle so its Drop fires DELETE for the old request.
            listing.handle = None;
            listing.request_id = None;
            listing.rows.clear();
            listing.total = 0;
            listing.truncated = false;
            listing.percent = 0;
            listing.error = None;
            listing.timed_out = false;
            let queue_id = listing.queue_id.clone();
            UpdateResult {
                redraw: true,
                intent: Some(PendingIntent::SpawnQueueListingRefresh { queue_id }),
                ..Default::default()
            }
        }

        BrowserQueueVerb::FocusListing => {
            // Tab while listing is focused → drop focus back to Tree
            // (continuing the rotation Endpoints → Listing → Tree). The
            // keymap shadow gate routes Tab here only when listing is
            // already focused; the entry into listing focus happens via
            // FocusAction::NextPane wrapping past the last detail
            // section.
            if state.browser.listing_focused {
                state.browser.listing_focused = false;
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }

        BrowserQueueVerb::PeekAttributes => {
            // `i`: open the peek modal pre-filled with the selected row's
            // identity, then emit the intent that spawns the GET worker
            // for the full attribute map.
            state.browser.open_queue_listing_peek_modal();
            let Some(listing) = state.browser.queue_listing.as_ref() else {
                return UpdateResult::default();
            };
            let Some(peek) = listing.peek.as_ref() else {
                return UpdateResult::default();
            };
            UpdateResult {
                redraw: true,
                intent: Some(PendingIntent::SpawnFlowfilePeekFetch {
                    queue_id: peek.queue_id.clone(),
                    uuid: peek.uuid.clone(),
                    cluster_node_id: peek.cluster_node_id.clone(),
                }),
                ..Default::default()
            }
        }

        BrowserQueueVerb::TraceLineage => {
            // `t`: switch to Tracer and start lineage for the selected
            // flowfile's UUID via the existing TraceByUuid cross-link path.
            let Some(listing) = state.browser.queue_listing.as_ref() else {
                return UpdateResult::default();
            };
            let visible = listing.visible_indices();
            let Some(&idx) = visible.get(listing.selected) else {
                return UpdateResult::default();
            };
            let Some(row) = listing.rows.get(idx) else {
                return UpdateResult::default();
            };
            let uuid = row.uuid.clone();
            UpdateResult {
                redraw: true,
                intent: Some(PendingIntent::Goto(CrossLink::TraceByUuid { uuid })),
                ..Default::default()
            }
        }

        BrowserQueueVerb::CopyUuid => {
            // `c`: copy the selected flowfile's UUID to the clipboard.
            let uuid = {
                let Some(listing) = state.browser.queue_listing.as_ref() else {
                    return UpdateResult::default();
                };
                let visible = listing.visible_indices();
                let Some(&idx) = visible.get(listing.selected) else {
                    return UpdateResult::default();
                };
                let Some(row) = listing.rows.get(idx) else {
                    return UpdateResult::default();
                };
                row.uuid.clone()
            };
            let preview: String = uuid.chars().take(40).collect();
            match state.copy_to_clipboard(uuid) {
                Ok(()) => state.post_info(format!("copied: {preview}")),
                Err(err) => state.post_warning(format!("clipboard: {err}")),
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }

        BrowserQueueVerb::Filter => {
            // `/`: open the inline filter prompt for the listing panel.
            if let Some(listing) = state.browser.queue_listing.as_mut() {
                listing.open_filter_prompt();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }

        BrowserQueueVerb::Common(CommonVerb::Close) => {
            // `Esc` cascade: close filter prompt → clear committed filter
            // → drop listing focus.  Returns a default UpdateResult when
            // nothing changed so that Esc can fall through to the outer
            // tree handler (Ascend / close modals).
            let Some(listing) = state.browser.queue_listing.as_mut() else {
                return UpdateResult::default();
            };
            if listing.filter_prompt.is_some() {
                listing.cancel_filter_prompt();
            } else if listing.filter.is_some() {
                listing.set_filter(None);
            } else if state.browser.listing_focused {
                state.browser.listing_focused = false;
            } else {
                return UpdateResult::default();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }

        BrowserQueueVerb::Common(
            CommonVerb::Copy
            | CommonVerb::OpenSearch
            | CommonVerb::SearchNext
            | CommonVerb::SearchPrev,
        ) => UpdateResult::default(),
    }
}

fn handle_browser_peek_verb(
    state: &mut AppState,
    verb: crate::input::BrowserPeekVerb,
) -> UpdateResult {
    use crate::input::BrowserPeekVerb;

    match verb {
        BrowserPeekVerb::Close => {
            // Cascade: close search prompt → close modal.
            let close_modal = match state
                .browser
                .queue_listing
                .as_mut()
                .and_then(|l| l.peek.as_mut())
            {
                Some(peek) => {
                    if peek.search.is_some() {
                        peek.close_search();
                        false
                    } else {
                        true
                    }
                }
                None => false,
            };
            if close_modal {
                state.browser.close_queue_listing_peek_modal();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }
        BrowserPeekVerb::OpenSearch => {
            if let Some(peek) = state
                .browser
                .queue_listing
                .as_mut()
                .and_then(|l| l.peek.as_mut())
            {
                peek.open_search();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }
        BrowserPeekVerb::SearchNext => {
            if let Some(peek) = state
                .browser
                .queue_listing
                .as_mut()
                .and_then(|l| l.peek.as_mut())
            {
                peek.cycle_search_next();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }
        BrowserPeekVerb::SearchPrev => {
            if let Some(peek) = state
                .browser
                .queue_listing
                .as_mut()
                .and_then(|l| l.peek.as_mut())
            {
                peek.cycle_search_prev();
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }
        BrowserPeekVerb::CopyAsJson => {
            let json = state
                .browser
                .queue_listing
                .as_ref()
                .and_then(|l| l.peek.as_ref())
                .and_then(|p| p.attrs_as_json());
            if let Some(json) = json {
                let preview: String = json.chars().take(40).collect();
                match state.copy_to_clipboard(json) {
                    Ok(()) => state.post_info(format!("copied: {preview}")),
                    Err(err) => state.post_warning(format!("clipboard: {err}")),
                }
            }
            UpdateResult {
                redraw: true,
                ..Default::default()
            }
        }
    }
}

fn action_history_check_signal_next_page(
    modal: &mut crate::view::browser::state::action_history_modal::ActionHistoryModalState,
) {
    let viewport_bottom = modal
        .scroll
        .offset
        .saturating_add(modal.scroll.last_viewport_rows);
    if modal.should_signal_next_page(viewport_bottom, 10) {
        modal.loading = true;
        modal.fetch_signal.notify_one();
    }
}

fn format_action_tsv(action: &nifi_rust_client::dynamic::types::ActionEntity) -> String {
    let inner = action.action.as_ref();
    let timestamp = action.timestamp.as_deref().unwrap_or("");
    let user = inner.and_then(|a| a.user_identity.as_deref()).unwrap_or("");
    let op = inner.and_then(|a| a.operation.as_deref()).unwrap_or("");
    let stype = inner.and_then(|a| a.source_type.as_deref()).unwrap_or("");
    let sname = inner.and_then(|a| a.source_name.as_deref()).unwrap_or("");
    format!("{timestamp}\t{user}\t{op}\t{stype}\t{sname}")
}

#[cfg(test)]
mod tests;
