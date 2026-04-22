//! Tracer tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Modal, PendingIntent, PendingSave, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Tracer tab.
pub(crate) struct TracerHandler;

impl ViewKeyHandler for TracerHandler {
    fn handle_verb(state: &mut AppState, verb: crate::input::ViewVerb) -> Option<UpdateResult> {
        use crate::input::TracerVerb;
        use crate::view::tracer::state::{ContentPane, DetailTab, EventDetail, TracerMode};

        // ContentModalVerb takes priority when the modal is open.
        if let crate::input::ViewVerb::ContentModal(v) = verb {
            return Some(handle_content_modal_verb(state, v));
        }

        let tv = match verb {
            crate::input::ViewVerb::Tracer(v) => v,
            _ => return None,
        };

        match tv {
            TracerVerb::Refresh => match &state.tracer.mode {
                TracerMode::Lineage(view) => {
                    let uuid = view.uuid.clone();
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::Dispatch(
                            crate::intent::Intent::RefreshLineage { uuid },
                        )),
                        tracer_followup: None,
                    });
                }
                TracerMode::LatestEvents(view) => {
                    let component_id = view.component_id.clone();
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::Dispatch(
                            crate::intent::Intent::RefreshLatestEvents { component_id },
                        )),
                        tracer_followup: None,
                    });
                }
                _ => {}
            },
            TracerVerb::Copy => {
                use crate::view::tracer::state::LineageFocus;
                match &state.tracer.mode {
                    TracerMode::Lineage(view) => {
                        // When focused on attributes, copy focused attribute value;
                        // otherwise copy the selected event's flowfile UUID.
                        let text = if let LineageFocus::Attributes { .. } = view.focus {
                            crate::view::tracer::state::lineage_focused_attribute_value(
                                &state.tracer,
                            )
                        } else {
                            view.snapshot
                                .events
                                .get(view.selected_event)
                                .map(|ev| ev.flow_file_uuid.clone())
                        };
                        if let Some(text) = text {
                            super::clipboard_copy(state, &text);
                        }
                    }
                    TracerMode::LatestEvents(_) => {
                        if let Some(uuid) =
                            crate::view::tracer::state::latest_events_selected_uuid(&state.tracer)
                        {
                            super::clipboard_copy(state, &uuid);
                        }
                    }
                    _ => {}
                }
            }
            TracerVerb::Save => {
                // Only open save modal when Input or Output tab is active and content is Shown.
                if let TracerMode::Lineage(ref view) = state.tracer.mode
                    && matches!(view.active_detail_tab, DetailTab::Input | DetailTab::Output)
                    && let EventDetail::Loaded { ref content, .. } = view.event_detail
                    && let ContentPane::Shown { side, .. } = content
                {
                    let event_id = view
                        .snapshot
                        .events
                        .get(view.selected_event)
                        .map(|e| e.event_id)
                        .unwrap_or(0);
                    state.modal = Some(Modal::SaveEventContent(
                        crate::widget::save_modal::SaveEventContentState::new(event_id, *side),
                    ));
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                // No-op when Attributes tab is active or content not shown.
                return Some(UpdateResult::default());
            }
            TracerVerb::ToggleDiff => {
                // Only toggle diff when Attributes tab is active.
                if let TracerMode::Lineage(ref view) = state.tracer.mode
                    && view.active_detail_tab == DetailTab::Attributes
                {
                    crate::view::tracer::state::lineage_toggle_diff_mode(&mut state.tracer);
                    return Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    });
                }
                return Some(UpdateResult::default());
            }
            TracerVerb::OpenContentModal => {
                use crate::view::tracer::state::{
                    ContentModalTab, EventDetail, open_content_modal,
                };
                // Extract the data we need before taking a mutable borrow on
                // `state.tracer` — the immutable borrow on `view` ends here.
                let extracted = if let TracerMode::Lineage(ref view) = state.tracer.mode
                    && let EventDetail::Loaded { ref event, .. } = view.event_detail
                {
                    // Always land on Input when it's available — Input is
                    // the "before" side and is what a user expects to see
                    // first when drilling into a content-modifying event.
                    // Fall back to Output only when Input isn't available
                    // (e.g. a CREATE event with no upstream content).
                    let active_tab = if event.input_available {
                        ContentModalTab::Input
                    } else {
                        ContentModalTab::Output
                    };
                    Some((active_tab, event.as_ref().clone()))
                } else {
                    None
                };
                if let Some((active_tab, detail)) = extracted {
                    let ceiling = state.tracer_config.modal_streaming_ceiling;
                    let fired = open_content_modal(&mut state.tracer, &detail, active_tab, ceiling);
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::SpawnModalChunks(fired)),
                        tracer_followup: None,
                    });
                }
                return Some(UpdateResult::default());
            }
        }

        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        })
    }

    fn handle_focus(
        state: &mut AppState,
        action: crate::input::FocusAction,
    ) -> Option<UpdateResult> {
        use crate::input::FocusAction;
        use crate::intent::Intent;
        use crate::view::tracer::state::{
            self as ts, DetailTab, EventDetail, LineageFocus, TracerMode,
        };

        // When the content modal is open, scroll actions are routed here
        // (the keymap shadow block lets Up/Down/PgUp/PgDn/Home/End through
        // as FocusAction). Handle them before the per-mode dispatch below.
        if state.tracer.content_modal.is_some() {
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = match action {
                FocusAction::Up => ts::content_modal_scroll_by(&mut state.tracer, -1, ceiling),
                FocusAction::Down => ts::content_modal_scroll_by(&mut state.tracer, 1, ceiling),
                FocusAction::PageUp => {
                    let rows = state
                        .tracer
                        .content_modal
                        .as_ref()
                        .map(|m| m.last_viewport_rows.max(1))
                        .unwrap_or(1) as isize;
                    ts::content_modal_scroll_by(&mut state.tracer, -rows, ceiling)
                }
                FocusAction::PageDown => {
                    let rows = state
                        .tracer
                        .content_modal
                        .as_ref()
                        .map(|m| m.last_viewport_rows.max(1))
                        .unwrap_or(1) as isize;
                    ts::content_modal_scroll_by(&mut state.tracer, rows, ceiling)
                }
                FocusAction::First => {
                    ts::content_modal_scroll_horizontal_home(&mut state.tracer);
                    ts::content_modal_scroll_to(&mut state.tracer, 0, ceiling)
                }
                FocusAction::Last => {
                    let line_count = ts::content_modal_line_count(&state.tracer);
                    ts::content_modal_scroll_to(
                        &mut state.tracer,
                        line_count.saturating_sub(1),
                        ceiling,
                    )
                }
                // Left/Right scroll the body sideways (for wide CSV rows,
                // long JSON property paths, etc.). Unlike vertical scroll
                // these don't trigger any fetch — purely a render offset.
                FocusAction::Left => {
                    ts::content_modal_scroll_horizontal_by(&mut state.tracer, -1);
                    Vec::new()
                }
                FocusAction::Right => {
                    ts::content_modal_scroll_horizontal_by(&mut state.tracer, 1);
                    Vec::new()
                }
                // Other focus actions are not modal-scroll; fall through to
                // the per-mode dispatch (no-op for most cases while modal is open).
                _ => {
                    return Some(UpdateResult::default());
                }
            };
            return Some(UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            });
        }

        match &state.tracer.mode {
            TracerMode::Entry(_) => match action {
                FocusAction::Descend => {
                    if let Some(uuid) = ts::entry_submit(&mut state.tracer) {
                        Some(UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Dispatch(Intent::TraceFlowfile(uuid))),
                            tracer_followup: None,
                        })
                    } else {
                        Some(UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                        })
                    }
                }
                FocusAction::Ascend => {
                    ts::handle_entry_clear(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                _ => None,
            },
            TracerMode::LatestEvents(_) => match action {
                FocusAction::Up => {
                    ts::latest_events_move_up(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Down => {
                    ts::latest_events_move_down(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                FocusAction::Descend => {
                    if let Some(uuid) = ts::latest_events_selected_uuid(&state.tracer) {
                        Some(UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Dispatch(Intent::TraceFlowfile(uuid))),
                            tracer_followup: None,
                        })
                    } else {
                        Some(UpdateResult::default())
                    }
                }
                FocusAction::Ascend => {
                    ts::cancel_lineage(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
                _ => None,
            },
            TracerMode::LineageRunning(_) => match action {
                FocusAction::Ascend => {
                    let followup = ts::cancel_lineage(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: followup,
                    })
                }
                _ => Some(UpdateResult::default()),
            },
            TracerMode::Lineage(view) => {
                let focus = view.focus;
                let active_tab = view.active_detail_tab;
                match focus {
                    LineageFocus::Timeline => match action {
                        FocusAction::Up => {
                            ts::lineage_move_up(&mut state.tracer);
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::Down => {
                            ts::lineage_move_down(&mut state.tracer);
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::PageUp => {
                            for _ in 0..10 {
                                ts::lineage_move_up(&mut state.tracer);
                            }
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::PageDown => {
                            for _ in 0..10 {
                                ts::lineage_move_down(&mut state.tracer);
                            }
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::First => {
                            if let TracerMode::Lineage(ref mut v) = state.tracer.mode
                                && !v.snapshot.events.is_empty()
                            {
                                v.selected_event = 0;
                                v.event_detail = EventDetail::NotLoaded;
                                v.focus = LineageFocus::Timeline;
                                v.active_detail_tab = DetailTab::default();
                            }
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::Last => {
                            if let TracerMode::Lineage(ref mut v) = state.tracer.mode {
                                let len = v.snapshot.events.len();
                                if len > 0 {
                                    v.selected_event = len - 1;
                                    v.event_detail = EventDetail::NotLoaded;
                                    v.focus = LineageFocus::Timeline;
                                    v.active_detail_tab = DetailTab::default();
                                }
                            }
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::Descend => {
                            if let Some(event_id) = ts::lineage_selected_event_id(&state.tracer) {
                                // If the detail for the selected event is
                                // already loaded, skip the re-fetch — just
                                // move focus into the detail pane. Otherwise
                                // mark loading and dispatch the fetch.
                                let already_loaded = matches!(
                                    &state.tracer.mode,
                                    TracerMode::Lineage(v)
                                        if matches!(
                                            &v.event_detail,
                                            EventDetail::Loaded { event, .. }
                                                if event.summary.event_id == event_id
                                        )
                                );
                                let intent = if already_loaded {
                                    None
                                } else {
                                    ts::lineage_mark_detail_loading(&mut state.tracer);
                                    Some(PendingIntent::Dispatch(Intent::LoadEventDetail {
                                        event_id,
                                    }))
                                };
                                // Move focus into the detail pane, defaulting
                                // to the Attributes tab.
                                if let TracerMode::Lineage(ref mut v) = state.tracer.mode {
                                    v.active_detail_tab = DetailTab::Attributes;
                                    v.focus = LineageFocus::Attributes { row: 0 };
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent,
                                    tracer_followup: None,
                                })
                            } else {
                                Some(UpdateResult::default())
                            }
                        }
                        FocusAction::Ascend => {
                            ts::cancel_lineage(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent: None,
                                tracer_followup: None,
                            })
                        }
                        FocusAction::NextPane => {
                            // Same as Descend: enter the detail pane, defaulting
                            // to the Attributes tab.
                            if let Some(event_id) = ts::lineage_selected_event_id(&state.tracer) {
                                let already_loaded = matches!(
                                    &state.tracer.mode,
                                    TracerMode::Lineage(v)
                                        if matches!(
                                            &v.event_detail,
                                            EventDetail::Loaded { event, .. }
                                                if event.summary.event_id == event_id
                                        )
                                );
                                let intent = if already_loaded {
                                    None
                                } else {
                                    ts::lineage_mark_detail_loading(&mut state.tracer);
                                    Some(PendingIntent::Dispatch(Intent::LoadEventDetail {
                                        event_id,
                                    }))
                                };
                                if let TracerMode::Lineage(ref mut v) = state.tracer.mode {
                                    v.active_detail_tab = DetailTab::Attributes;
                                    v.focus = LineageFocus::Attributes { row: 0 };
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent,
                                    tracer_followup: None,
                                })
                            } else {
                                Some(UpdateResult::default())
                            }
                        }
                        FocusAction::PrevPane => {
                            // Timeline is the first pane — PrevPane is a no-op.
                            Some(UpdateResult::default())
                        }
                        _ => None,
                    },
                    LineageFocus::Attributes { .. } | LineageFocus::Content { .. } => {
                        match action {
                            FocusAction::Up => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        ts::lineage_attr_move_up(&mut state.tracer);
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_up(&mut state.tracer, 1);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::Down => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        ts::lineage_attr_move_down(&mut state.tracer);
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_down(&mut state.tracer, 1);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::PageUp => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        for _ in 0..5 {
                                            ts::lineage_attr_move_up(&mut state.tracer);
                                        }
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_up(&mut state.tracer, 10);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::PageDown => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        for _ in 0..5 {
                                            ts::lineage_attr_move_down(&mut state.tracer);
                                        }
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_down(&mut state.tracer, 10);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::First => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        if let TracerMode::Lineage(ref mut v) = state.tracer.mode
                                            && let LineageFocus::Attributes { ref mut row } =
                                                v.focus
                                        {
                                            *row = 0;
                                        }
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_home(&mut state.tracer);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::Last => {
                                match active_tab {
                                    DetailTab::Attributes => {
                                        if let TracerMode::Lineage(ref mut v) = state.tracer.mode {
                                            let len = ts::lineage_visible_attributes(v).len();
                                            if len > 0 {
                                                v.focus = LineageFocus::Attributes { row: len - 1 };
                                            }
                                        }
                                    }
                                    DetailTab::Input | DetailTab::Output => {
                                        ts::lineage_content_scroll_end(&mut state.tracer);
                                    }
                                }
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::Right => {
                                ts::lineage_cycle_detail_tab_right(&mut state.tracer);
                                let intent =
                                    dispatch_content_fetch_for_active_tab(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::Left => {
                                ts::lineage_cycle_detail_tab_left(&mut state.tracer);
                                let intent =
                                    dispatch_content_fetch_for_active_tab(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::Descend => None,
                            FocusAction::Ascend => {
                                ts::lineage_focus_timeline(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::NextPane => {
                                // Cycle tab right — same as Right.
                                ts::lineage_cycle_detail_tab_right(&mut state.tracer);
                                let intent =
                                    dispatch_content_fetch_for_active_tab(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent,
                                    tracer_followup: None,
                                })
                            }
                            FocusAction::PrevPane => {
                                // Exit to Timeline — same as Ascend.
                                ts::lineage_focus_timeline(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                })
                            }
                        }
                    }
                }
            }
        }
    }

    fn is_text_input_focused(state: &AppState) -> bool {
        // Entry mode text input.
        if matches!(
            state.tracer.mode,
            crate::view::tracer::state::TracerMode::Entry(_)
        ) {
            return true;
        }
        // Content modal search input.
        state
            .tracer
            .content_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false)
    }

    fn handle_text_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        use crate::intent::Intent;
        use crate::view::tracer::state as ts;

        // Modal search input takes priority when active.
        let modal_search_active = state
            .tracer
            .content_modal
            .as_ref()
            .and_then(|m| m.search.as_ref())
            .map(|s| s.input_active)
            .unwrap_or(false);
        if modal_search_active {
            return handle_content_modal_search_input(state, key);
        }

        // Entry mode: character keys go into the UUID input. Ctrl modifiers
        // fall through to global handlers.
        match (key.code, key.modifiers) {
            (KeyCode::Char('v'), KeyModifiers::NONE) => {
                match state.get_from_clipboard() {
                    Ok(text) => {
                        for ch in text.chars() {
                            ts::handle_entry_char(&mut state.tracer, ch);
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
                })
            }
            (KeyCode::Char('x'), KeyModifiers::NONE) => {
                let text = ts::entry_value(&state.tracer).to_owned();
                if !text.is_empty() {
                    let _ = state.copy_to_clipboard(text);
                }
                ts::handle_entry_clear(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            (KeyCode::Char(ch), KeyModifiers::NONE) | (KeyCode::Char(ch), KeyModifiers::SHIFT) => {
                ts::handle_entry_char(&mut state.tracer, ch);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                ts::handle_entry_backspace(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => {
                ts::handle_entry_clear(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                })
            }
            (KeyCode::Enter, _) => {
                if let Some(uuid) = ts::entry_submit(&mut state.tracer) {
                    Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::Dispatch(Intent::TraceFlowfile(uuid))),
                        tracer_followup: None,
                    })
                } else {
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                    })
                }
            }
            _ => None,
        }
    }

    fn default_cross_link(_state: &AppState) -> Option<crate::input::GoTarget> {
        None
    }
}

/// When the user cycles into the Input or Output tab, trigger a
/// content fetch if the tab isn't already loaded / loading for that
/// side. Mutates the content pane to the appropriate `Loading` state
/// and returns the `FetchEventContent` intent for the dispatcher.
///
/// Returns `None` when:
/// - Not in Lineage mode.
/// - Event detail isn't `Loaded` yet (the detail fetch is still in
///   flight — a later cycle press will kick off the content fetch).
/// - Active tab is `Attributes` (no fetch needed).
/// - Content pane is already showing or loading the requested side.
fn dispatch_content_fetch_for_active_tab(
    state: &mut crate::view::tracer::state::TracerState,
) -> Option<PendingIntent> {
    use crate::client::ContentSide as ClientSide;
    use crate::intent::{ContentSide as IntentSide, Intent};
    use crate::view::tracer::state::{
        ContentPane, DetailTab, EventDetail, TracerMode, lineage_mark_content_loading,
    };

    // Extract what we need without holding a mutable borrow, so we can
    // call `lineage_mark_content_loading` afterwards.
    let (event_id, want_client_side, want_intent_side) = {
        let TracerMode::Lineage(ref view) = state.mode else {
            return None;
        };
        let EventDetail::Loaded {
            ref event,
            ref content,
        } = view.event_detail
        else {
            return None;
        };

        let (client_side, intent_side) = match view.active_detail_tab {
            DetailTab::Attributes => return None,
            DetailTab::Input => (ClientSide::Input, IntentSide::Input),
            DetailTab::Output => (ClientSide::Output, IntentSide::Output),
        };

        // Skip if already showing or loading the requested side.
        match content {
            ContentPane::Shown { side, .. } if *side == client_side => return None,
            ContentPane::LoadingInput if client_side == ClientSide::Input => return None,
            ContentPane::LoadingOutput if client_side == ClientSide::Output => return None,
            _ => {}
        }

        (event.summary.event_id, client_side, intent_side)
    };

    lineage_mark_content_loading(state, want_client_side);

    Some(PendingIntent::Dispatch(Intent::FetchEventContent {
        event_id,
        side: want_intent_side,
    }))
}

/// Builds a `PendingSave` referencing the currently-shown content
/// pane's event id and side. Returns `None` if the content pane is
/// not in the `Shown` state. The worker re-fetches the full body at
/// save time rather than re-using the preview's (potentially
/// truncated) bytes.
pub(super) fn build_pending_save(
    state: &AppState,
    path: std::path::PathBuf,
) -> Option<PendingIntent> {
    use crate::view::tracer::state::{ContentPane, EventDetail, TracerMode};
    if let TracerMode::Lineage(ref view) = state.tracer.mode
        && let EventDetail::Loaded {
            ref event,
            ref content,
            ..
        } = view.event_detail
        && let ContentPane::Shown { side, .. } = content
    {
        Some(PendingIntent::SaveEventContent(PendingSave {
            path,
            event_id: event.summary.event_id,
            side: *side,
        }))
    } else {
        None
    }
}

/// Marks the current lineage event detail as loading and returns a
/// `PendingIntent` to fetch it. Returns `None` when not in Lineage mode or
/// no event is selected.
fn lineage_load_detail_intent(
    state: &mut crate::view::tracer::state::TracerState,
) -> Option<PendingIntent> {
    use crate::intent::Intent;
    use crate::view::tracer::state::{lineage_mark_detail_loading, lineage_selected_event_id};

    let event_id = lineage_selected_event_id(state)?;
    lineage_mark_detail_loading(state);
    Some(PendingIntent::Dispatch(Intent::LoadEventDetail {
        event_id,
    }))
}

// ── Content modal search text input ──────────────────────────────────────────

/// Handles text-input keypresses while the content modal's search input is
/// active. Mirrors the pattern from `bulletins.rs:handle_modal_search_input`.
fn handle_content_modal_search_input(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    use crate::view::tracer::state as ts;
    match key.code {
        KeyCode::Esc => ts::content_modal_search_cancel(&mut state.tracer),
        KeyCode::Enter => {
            ts::content_modal_search_commit(&mut state.tracer);
            // Scroll to the first match if one exists.
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, ceiling);
            if !fired.is_empty() {
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::SpawnModalChunks(fired)),
                    tracer_followup: None,
                });
            }
        }
        KeyCode::Backspace => ts::content_modal_search_pop(&mut state.tracer),
        KeyCode::Char(ch) => ts::content_modal_search_push(&mut state.tracer, ch),
        _ => return Some(UpdateResult::default()),
    }
    Some(UpdateResult {
        redraw: true,
        intent: None,
        tracer_followup: None,
    })
}

// ── ContentModalVerb dispatch ─────────────────────────────────────────────────

/// Dispatch a `ContentModalVerb` action. Called when the content viewer modal
/// is open and the keymap has routed `ViewVerb::ContentModal(v)` here.
fn handle_content_modal_verb(
    state: &mut AppState,
    v: crate::input::ContentModalVerb,
) -> UpdateResult {
    use crate::input::ContentModalVerb;
    use crate::view::tracer::state::{
        self as ts, ContentModalTab, Diffable, close_content_modal, content_modal_copy_text,
        hunk_next, hunk_prev, switch_content_modal_tab,
    };

    match v {
        ContentModalVerb::Close => {
            // Esc cancels an active search first (input-active Esc is
            // already handled in `handle_content_modal_search_input`;
            // this path handles Esc after a search has been committed).
            // Only when no search is active does Esc close the modal.
            let has_search = state
                .tracer
                .content_modal
                .as_ref()
                .is_some_and(|m| m.search.is_some());
            if has_search {
                ts::content_modal_search_cancel(&mut state.tracer);
            } else {
                close_content_modal(&mut state.tracer);
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }

        ContentModalVerb::Copy => {
            if let Some(text) = content_modal_copy_text(&state.tracer) {
                super::clipboard_copy(state, &text);
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }

        ContentModalVerb::SwitchTabNext => {
            let Some(modal) = state.tracer.content_modal.as_ref() else {
                return UpdateResult::default();
            };
            let next = match modal.active_tab {
                ContentModalTab::Input => {
                    if modal.header.output_available {
                        ContentModalTab::Output
                    } else if matches!(modal.diffable, Diffable::Ok) {
                        ContentModalTab::Diff
                    } else {
                        ContentModalTab::Input
                    }
                }
                ContentModalTab::Output => {
                    if matches!(modal.diffable, Diffable::Ok) {
                        ContentModalTab::Diff
                    } else {
                        ContentModalTab::Output
                    }
                }
                ContentModalTab::Diff => ContentModalTab::Input,
            };
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = switch_content_modal_tab(&mut state.tracer, next, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }

        ContentModalVerb::SwitchTabPrev => {
            let Some(modal) = state.tracer.content_modal.as_ref() else {
                return UpdateResult::default();
            };
            let prev = match modal.active_tab {
                ContentModalTab::Input => {
                    if matches!(modal.diffable, Diffable::Ok) {
                        ContentModalTab::Diff
                    } else if modal.header.output_available {
                        ContentModalTab::Output
                    } else {
                        ContentModalTab::Input
                    }
                }
                ContentModalTab::Output => ContentModalTab::Input,
                ContentModalTab::Diff => {
                    if modal.header.output_available {
                        ContentModalTab::Output
                    } else {
                        ContentModalTab::Input
                    }
                }
            };
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = switch_content_modal_tab(&mut state.tracer, prev, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }

        ContentModalVerb::JumpInput => {
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired =
                switch_content_modal_tab(&mut state.tracer, ContentModalTab::Input, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }

        ContentModalVerb::JumpOutput => {
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired =
                switch_content_modal_tab(&mut state.tracer, ContentModalTab::Output, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }

        ContentModalVerb::JumpDiff => {
            // Only act when the diff is ready.
            let diffable_ok = state
                .tracer
                .content_modal
                .as_ref()
                .map(|m| matches!(m.diffable, Diffable::Ok))
                .unwrap_or(false);
            if !diffable_ok {
                return UpdateResult::default();
            }
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = switch_content_modal_tab(&mut state.tracer, ContentModalTab::Diff, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }

        ContentModalVerb::HunkNext => {
            hunk_next(&mut state.tracer);
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }

        ContentModalVerb::HunkPrev => {
            hunk_prev(&mut state.tracer);
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }

        ContentModalVerb::OpenSearch => {
            use crate::widget::search::SearchState;
            if let Some(modal) = state.tracer.content_modal.as_mut() {
                modal.search = Some(SearchState {
                    input_active: true,
                    ..Default::default()
                });
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            }
        }
        ContentModalVerb::SearchNext => {
            if let Some(modal) = state.tracer.content_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && s.committed
                && !s.matches.is_empty()
            {
                let i = s.current.unwrap_or(0);
                s.current = Some((i + 1) % s.matches.len());
            }
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }
        ContentModalVerb::SearchPrev => {
            if let Some(modal) = state.tracer.content_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && s.committed
                && !s.matches.is_empty()
            {
                let i = s.current.unwrap_or(0);
                let n = s.matches.len();
                s.current = Some((i + n - 1) % n);
            }
            let ceiling = state.tracer_config.modal_streaming_ceiling;
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, ceiling);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
            }
        }
        ContentModalVerb::Save => {
            // Open the save modal for the active side (or last_nondiff_tab when on
            // the Diff tab). Uses the same Modal::SaveEventContent path as
            // TracerVerb::Save — event_id and side are derived from the modal header.
            let save_state = state.tracer.content_modal.as_ref().map(|modal| {
                let side = match modal.active_tab {
                    ContentModalTab::Diff => match modal.last_nondiff_tab {
                        ContentModalTab::Output => crate::client::ContentSide::Output,
                        _ => crate::client::ContentSide::Input,
                    },
                    ContentModalTab::Output => crate::client::ContentSide::Output,
                    _ => crate::client::ContentSide::Input,
                };
                crate::widget::save_modal::SaveEventContentState::new(modal.event_id, side)
            });
            if let Some(save) = save_state {
                state.modal = Some(Modal::SaveEventContent(save));
                UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                }
            } else {
                UpdateResult::default()
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::{ViewId, ViewKeyHandler};
    use crate::client::tracer::ProvenanceEventDetail;
    use crate::view::tracer::state::{
        self as ts, ContentPane, EventDetail, LineageView, TracerMode,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    // Note: 'i'/'o' are no longer bound in Lineage mode (use ←/→ tab cycling instead).
    // The old tests that verified 'i'/'o' banner behavior have been removed.
    // io_letters_no_longer_bound (in Task 15 tests) verifies they are true no-ops.

    #[test]
    fn tracer_lineage_row_nav_uses_arrows_only_no_jk() {
        use crate::client::LineageSnapshot;
        use crate::client::tracer::ProvenanceEventSummary;

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Tracer;
        let make_summary = |id: i64| ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: "comp-1".to_string(),
            component_name: "Gen".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-1".to_string(),
            relationship: None,
            details: None,
        };
        s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff-1".to_string(),
            snapshot: LineageSnapshot {
                events: vec![make_summary(1), make_summary(2)],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 0,
            event_detail: EventDetail::NotLoaded,
            loaded_details: std::collections::HashMap::new(),
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: SystemTime::now(),
            focus: ts::LineageFocus::default(),
            active_detail_tab: ts::DetailTab::default(),
        }));

        // j is a no-op (returns None, global handler no-ops).
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.selected_event
        } else {
            panic!("expected Lineage mode")
        };
        assert_eq!(sel, 0, "j dropped");

        // Down moves selection forward.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.selected_event
        } else {
            panic!("expected Lineage mode")
        };
        assert!(sel > 0, "Down still works");

        let before = sel;
        // k is a no-op.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.selected_event
        } else {
            panic!("expected Lineage mode")
        };
        assert_eq!(sel, before, "k dropped");

        // Up moves selection back.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        let sel = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.selected_event
        } else {
            panic!("expected Lineage mode")
        };
        assert!(sel < before, "Up still works");
    }

    #[test]
    fn tracer_latest_events_row_nav_uses_arrows_only_no_jk() {
        use crate::client::LatestEventsSnapshot;
        use crate::client::tracer::ProvenanceEventSummary;
        use crate::view::tracer::state::LatestEventsView;

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Tracer;
        let make_summary = |id: i64| ProvenanceEventSummary {
            event_id: id,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: "comp-1".to_string(),
            component_name: "Gen".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-1".to_string(),
            relationship: None,
            details: None,
        };
        let snap = LatestEventsSnapshot {
            component_id: "comp-1".to_string(),
            component_label: "Gen".to_string(),
            events: vec![make_summary(1), make_summary(2)],
            fetched_at: SystemTime::now(),
        };
        s.tracer.mode = TracerMode::LatestEvents(LatestEventsView::from_snapshot(snap));

        // j is a no-op.
        update(&mut s, key(KeyCode::Char('j'), KeyModifiers::NONE), &c);
        let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
            v.selected
        } else {
            panic!("expected LatestEvents mode")
        };
        assert_eq!(sel, 0, "j dropped");

        // Down moves selection forward.
        update(&mut s, key(KeyCode::Down, KeyModifiers::NONE), &c);
        let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
            v.selected
        } else {
            panic!("expected LatestEvents mode")
        };
        assert!(sel > 0, "Down still works");

        let before = sel;
        // k is a no-op.
        update(&mut s, key(KeyCode::Char('k'), KeyModifiers::NONE), &c);
        let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
            v.selected
        } else {
            panic!("expected LatestEvents mode")
        };
        assert_eq!(sel, before, "k dropped");

        // Up moves selection back.
        update(&mut s, key(KeyCode::Up, KeyModifiers::NONE), &c);
        let sel = if let TracerMode::LatestEvents(ref v) = s.tracer.mode {
            v.selected
        } else {
            panic!("expected LatestEvents mode")
        };
        assert!(sel < before, "Up still works");
    }

    // ── Task 15 helpers ───────────────────────────────────────────────────────

    /// Seeds a Lineage state with a loaded event detail that has BOTH input
    /// and output available. The detail pane focus is on Attributes.
    fn seed_tracer_with_loaded_detail(s: &mut crate::app::state::AppState) {
        use crate::client::LineageSnapshot;
        use crate::client::tracer::ProvenanceEventSummary;

        let summary = ProvenanceEventSummary {
            event_id: 99,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CONTENT_MODIFIED".to_string(),
            component_id: "comp-99".to_string(),
            component_name: "UpdateAttribute".to_string(),
            component_type: "UpdateAttribute".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-99".to_string(),
            relationship: None,
            details: None,
        };
        let detail = ProvenanceEventDetail {
            summary: summary.clone(),
            attributes: vec![],
            transit_uri: None,
            input_available: true,
            output_available: true,
            input_size: None,
            output_size: None,
        };
        s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff-99".to_string(),
            snapshot: LineageSnapshot {
                events: vec![summary],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 0,
            event_detail: EventDetail::Loaded {
                event: Box::new(detail),
                content: ContentPane::default(),
            },
            loaded_details: std::collections::HashMap::new(),
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: SystemTime::now(),
            focus: ts::LineageFocus::Attributes { row: 0 },
            active_detail_tab: ts::DetailTab::Attributes,
        }));
    }

    /// Seeds a Lineage state with output only (no input available).
    fn seed_tracer_with_output_only(s: &mut crate::app::state::AppState) {
        use crate::client::LineageSnapshot;
        use crate::client::tracer::ProvenanceEventSummary;

        let summary = ProvenanceEventSummary {
            event_id: 88,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: "comp-88".to_string(),
            component_name: "GenerateFlowFile".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-88".to_string(),
            relationship: None,
            details: None,
        };
        let detail = ProvenanceEventDetail {
            summary: summary.clone(),
            attributes: vec![],
            transit_uri: None,
            input_available: false,
            output_available: true,
            input_size: None,
            output_size: None,
        };
        s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff-88".to_string(),
            snapshot: LineageSnapshot {
                events: vec![summary],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 0,
            event_detail: EventDetail::Loaded {
                event: Box::new(detail),
                content: ContentPane::default(),
            },
            loaded_details: std::collections::HashMap::new(),
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: SystemTime::now(),
            focus: ts::LineageFocus::Attributes { row: 0 },
            active_detail_tab: ts::DetailTab::Attributes,
        }));
    }

    // ── Task 15 tests ─────────────────────────────────────────────────────────

    #[test]
    fn right_cycles_detail_tabs() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        // Ensure we're on Attributes tab.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
        }

        super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
        let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.active_detail_tab
        } else {
            panic!("expected Lineage")
        };
        assert_eq!(tab, ts::DetailTab::Input);
    }

    #[test]
    fn right_skips_disabled_tab() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_output_only(&mut s);
        // Ensure we're on Attributes tab.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
        }

        super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
        let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.active_detail_tab
        } else {
            panic!("expected Lineage")
        };
        assert_eq!(tab, ts::DetailTab::Output, "should skip Input (disabled)");
    }

    #[test]
    fn d_toggles_diff() {
        use crate::input::{TracerVerb, ViewVerb};
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        let before = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.diff_mode
        } else {
            panic!("expected Lineage")
        };
        super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::ToggleDiff));
        let after = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.diff_mode
        } else {
            panic!("expected Lineage")
        };
        assert_ne!(after, before, "ToggleDiff should flip diff_mode");
    }

    #[test]
    fn s_saves_only_on_content_tab() {
        use crate::client::tracer::{ContentRender, ContentSide};
        use crate::input::{TracerVerb, ViewVerb};

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);

        // Attributes tab → Save should be a no-op.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
        }
        super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::Save));
        assert!(
            s.modal.is_none(),
            "Save must be a no-op when Attributes tab is active"
        );

        // Input tab with content shown → Save should open modal.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Input;
            v.focus = ts::LineageFocus::Content { scroll: 0 };
            let existing_event = if let EventDetail::Loaded { ref event, .. } = v.event_detail {
                event.clone()
            } else {
                panic!("expected Loaded")
            };
            v.event_detail = EventDetail::Loaded {
                event: existing_event,
                content: ContentPane::Shown {
                    side: ContentSide::Input,
                    render: ContentRender::Text {
                        text: "data".to_string(),
                        pretty_printed: false,
                    },
                    bytes_fetched: 4,
                    truncated: false,
                },
            };
        }
        super::TracerHandler::handle_verb(&mut s, ViewVerb::Tracer(TracerVerb::Save));
        assert!(
            matches!(s.modal, Some(crate::app::state::Modal::SaveEventContent(_))),
            "Save should open modal when Input tab is active and content is Shown"
        );
    }

    #[test]
    fn io_letters_no_longer_bound() {
        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
        }

        // 'i' should not switch to Input tab — only Left/Right does that.
        update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        let tab = if let TracerMode::Lineage(ref v) = s.tracer.mode {
            v.active_detail_tab
        } else {
            panic!("expected Lineage")
        };
        assert_eq!(
            tab,
            ts::DetailTab::Attributes,
            "'i' must not switch to Input — use Left/Right"
        );
    }

    #[test]
    fn descend_on_timeline_focuses_detail_pane() {
        // Regression: pressing Enter on a Timeline selection must move
        // focus into the Detail pane. Before the fix, handle_focus only
        // set `active_detail_tab` and `lineage_mark_detail_loading`
        // reset `focus` back to Timeline, so users couldn't reach the
        // detail pane to check attribute values or cycle tabs.
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        // Start with focus on the timeline, matching the state after a
        // lineage query completes.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.focus = ts::LineageFocus::Timeline;
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Descend)
            .expect("Descend on Timeline must be consumed");
        assert!(r.redraw);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert!(
            matches!(v.focus, ts::LineageFocus::Attributes { .. }),
            "Descend must move focus into the Attributes tab, got {:?}",
            v.focus
        );
        assert_eq!(v.active_detail_tab, ts::DetailTab::Attributes);
    }

    #[test]
    fn right_from_timeline_descent_cycles_to_input_tab() {
        // After Descend lands focus on Attributes, pressing Right must
        // cycle to the Input tab (and set focus to Content). This
        // verifies the full Timeline → Detail → tab-cycle flow.
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.focus = ts::LineageFocus::Timeline;
        }

        super::TracerHandler::handle_focus(&mut s, FocusAction::Descend);
        super::TracerHandler::handle_focus(&mut s, FocusAction::Right);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert_eq!(v.active_detail_tab, ts::DetailTab::Input);
        assert!(matches!(v.focus, ts::LineageFocus::Content { .. }));
    }

    #[test]
    fn cycling_to_input_tab_dispatches_fetch_event_content() {
        // Regression: cycling into the Input tab must trigger a
        // FetchEventContent intent and mark the content pane as
        // LoadingInput. Before the fix, the cycle just updated
        // `active_detail_tab` and left the content pane Collapsed, so
        // users saw an empty tab forever.
        use crate::app::state::PendingIntent;
        use crate::input::FocusAction;
        use crate::intent::{ContentSide as IntentSide, Intent};
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        // Reset to Attributes so Right cycles into Input.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
            v.focus = ts::LineageFocus::Attributes { row: 0 };
            if let EventDetail::Loaded {
                ref mut content, ..
            } = v.event_detail
            {
                *content = ContentPane::Collapsed;
            }
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Right)
            .expect("Right on Attributes tab must be consumed");
        assert!(r.redraw);
        assert!(
            matches!(
                r.intent,
                Some(PendingIntent::Dispatch(Intent::FetchEventContent {
                    side: IntentSide::Input,
                    ..
                }))
            ),
            "expected FetchEventContent(Input), got {:?}",
            r.intent
        );
        // Content pane should now be in LoadingInput state.
        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        let EventDetail::Loaded { ref content, .. } = v.event_detail else {
            panic!("expected Loaded detail");
        };
        assert!(matches!(content, ContentPane::LoadingInput));
    }

    #[test]
    fn cycling_to_input_tab_is_noop_when_already_loaded() {
        // When content for the target side is already Shown, cycling
        // must not re-dispatch the fetch. Users don't want a network
        // round-trip every time they flip between tabs.
        use crate::client::ContentRender;
        use crate::client::ContentSide as ClientSide;
        use crate::input::FocusAction;

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
            v.focus = ts::LineageFocus::Attributes { row: 0 };
            if let EventDetail::Loaded {
                ref mut content, ..
            } = v.event_detail
            {
                *content = ContentPane::Shown {
                    side: ClientSide::Input,
                    render: ContentRender::Text {
                        text: "hi".into(),
                        pretty_printed: false,
                    },
                    bytes_fetched: 2,
                    truncated: false,
                };
            }
        }

        // Cycle to Input (which is already loaded).
        let r =
            super::TracerHandler::handle_focus(&mut s, FocusAction::Right).expect("Right consumed");
        assert!(
            r.intent.is_none(),
            "no fetch should dispatch when side is already Shown"
        );
    }

    #[test]
    fn next_pane_in_timeline_enters_attributes() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.focus = ts::LineageFocus::Timeline;
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::NextPane)
            .expect("NextPane on Timeline must be consumed");
        assert!(r.redraw);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert!(
            matches!(v.focus, ts::LineageFocus::Attributes { .. }),
            "NextPane from Timeline must move focus to Attributes, got {:?}",
            v.focus
        );
        assert_eq!(v.active_detail_tab, ts::DetailTab::Attributes);
    }

    #[test]
    fn prev_pane_in_timeline_is_noop() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.focus = ts::LineageFocus::Timeline;
        }

        let _r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
            .expect("PrevPane on Timeline must be consumed (no-op)");

        // Focus must remain on Timeline.
        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert!(
            matches!(v.focus, ts::LineageFocus::Timeline),
            "PrevPane from Timeline must not change focus, got {:?}",
            v.focus
        );
    }

    #[test]
    fn next_pane_in_attributes_cycles_tab_right() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
            v.focus = ts::LineageFocus::Attributes { row: 0 };
            if let EventDetail::Loaded {
                ref mut content, ..
            } = v.event_detail
            {
                *content = ContentPane::Collapsed;
            }
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::NextPane)
            .expect("NextPane in Attributes must be consumed");
        assert!(r.redraw);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert_eq!(
            v.active_detail_tab,
            ts::DetailTab::Input,
            "NextPane from Attributes must cycle to Input tab"
        );
    }

    #[test]
    fn prev_pane_in_attributes_returns_to_timeline() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
            v.focus = ts::LineageFocus::Attributes { row: 0 };
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
            .expect("PrevPane in Attributes must be consumed");
        assert!(r.redraw);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert!(
            matches!(v.focus, ts::LineageFocus::Timeline),
            "PrevPane from Attributes must return focus to Timeline, got {:?}",
            v.focus
        );
    }

    #[test]
    fn prev_pane_in_content_returns_to_timeline() {
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Input;
            v.focus = ts::LineageFocus::Content { scroll: 0 };
        }

        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::PrevPane)
            .expect("PrevPane in Content must be consumed");
        assert!(r.redraw);

        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert!(
            matches!(v.focus, ts::LineageFocus::Timeline),
            "PrevPane from Content must return focus to Timeline, got {:?}",
            v.focus
        );
    }

    #[test]
    fn left_right_in_detail_still_cycle_subtab() {
        // Left/Right must still cycle sub-tabs in the Attributes/Content focus,
        // unchanged by the NextPane/PrevPane additions.
        use crate::input::FocusAction;
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            v.active_detail_tab = ts::DetailTab::Attributes;
            v.focus = ts::LineageFocus::Attributes { row: 0 };
        }

        // Right should cycle to Input.
        super::TracerHandler::handle_focus(&mut s, FocusAction::Right);
        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert_eq!(
            v.active_detail_tab,
            ts::DetailTab::Input,
            "Right cycles to Input"
        );

        // Left should cycle back to Attributes.
        super::TracerHandler::handle_focus(&mut s, FocusAction::Left);
        let TracerMode::Lineage(ref v) = s.tracer.mode else {
            panic!("expected Lineage");
        };
        assert_eq!(
            v.active_detail_tab,
            ts::DetailTab::Attributes,
            "Left cycles back to Attributes"
        );
    }

    #[test]
    fn build_pending_save_returns_event_id_and_side() {
        use crate::app::state::{PendingIntent, PendingSave};
        use crate::client::tracer::ProvenanceEventDetail;
        use crate::client::{ContentRender, ContentSide};

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        seed_tracer_with_loaded_detail(&mut s);
        // Override the loaded detail: event_id 77, output side content shown.
        if let TracerMode::Lineage(ref mut v) = s.tracer.mode {
            let summary = crate::client::tracer::ProvenanceEventSummary {
                event_id: 77,
                event_time_iso: "2026-01-01T00:00:00Z".to_string(),
                event_type: "CREATE".to_string(),
                component_id: "comp-77".to_string(),
                component_name: "Gen".to_string(),
                component_type: "GenerateFlowFile".to_string(),
                group_id: "root".to_string(),
                flow_file_uuid: "ff-77".to_string(),
                relationship: None,
                details: None,
            };
            let detail = ProvenanceEventDetail {
                summary,
                attributes: vec![],
                transit_uri: None,
                input_available: false,
                output_available: true,
                input_size: None,
                output_size: None,
            };
            v.event_detail = EventDetail::Loaded {
                event: Box::new(detail),
                content: ContentPane::Shown {
                    side: ContentSide::Output,
                    render: ContentRender::Text {
                        text: "hi".to_string(),
                        pretty_printed: false,
                    },
                    bytes_fetched: 2,
                    truncated: false,
                },
            };
        } else {
            panic!("expected lineage mode");
        }

        let path = std::path::PathBuf::from("/tmp/out.bin");
        let pending =
            super::build_pending_save(&s, path.clone()).expect("should build pending save");
        match pending {
            PendingIntent::SaveEventContent(PendingSave {
                path: p,
                event_id,
                side,
            }) => {
                assert_eq!(p, path);
                assert_eq!(event_id, 77);
                assert_eq!(side, ContentSide::Output);
            }
            _ => panic!("expected SaveEventContent"),
        }
    }

    #[test]
    fn collect_hints_save_label_shows_full_size_when_content_truncated() {
        use crate::client::tracer::ProvenanceEventSummary;
        use crate::client::{ContentRender, ContentSide, LineageSnapshot};

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;

        // 512 MiB output; content pane shows a truncated preview.
        let total_output_bytes: u64 = 512 << 20;
        let summary = ProvenanceEventSummary {
            event_id: 42,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CONTENT_MODIFIED".to_string(),
            component_id: "comp-42".to_string(),
            component_name: "SomeProcessor".to_string(),
            component_type: "SomeProcessor".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-42".to_string(),
            relationship: None,
            details: None,
        };
        let detail = ProvenanceEventDetail {
            summary: summary.clone(),
            attributes: vec![],
            transit_uri: None,
            input_available: false,
            output_available: true,
            input_size: None,
            output_size: Some(total_output_bytes),
        };
        s.tracer.mode = TracerMode::Lineage(Box::new(LineageView {
            uuid: "ff-42".to_string(),
            snapshot: LineageSnapshot {
                events: vec![summary],
                percent_completed: 100,
                finished: true,
            },
            selected_event: 0,
            event_detail: EventDetail::Loaded {
                event: Box::new(detail),
                content: ContentPane::Shown {
                    side: ContentSide::Output,
                    render: ContentRender::Text {
                        text: "preview...".to_string(),
                        pretty_printed: false,
                    },
                    bytes_fetched: 1024,
                    truncated: true,
                },
            },
            loaded_details: std::collections::HashMap::new(),
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: std::time::SystemTime::now(),
            focus: ts::LineageFocus::default(),
            active_detail_tab: ts::DetailTab::default(),
        }));

        let hints = super::super::collect_hints(&s);
        let save_hint = hints
            .iter()
            .find(|h| h.action.as_ref().contains("fetches full"))
            .expect("save hint should contain 'fetches full' when content is truncated");
        assert!(
            save_hint.action.as_ref().contains("512.0 MiB"),
            "save hint action should include the human-readable total size; got: {}",
            save_hint.action
        );
    }

    // ── Content modal scroll via handle_focus ─────────────────────────────────

    /// Build a state with a content modal open and 100 fully-loaded lines on
    /// the Input tab.
    fn state_with_open_modal() -> crate::app::state::AppState {
        use crate::client::tracer::ContentRender;
        use crate::view::tracer::state::{
            ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
        };

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        let text = "row\n".repeat(100);
        let modal = ContentModalState {
            event_id: 1,
            header: ContentModalHeader {
                event_type: "DROP".into(),
                event_timestamp_iso: "".into(),
                component_name: "x".into(),
                pg_path: "pg".into(),
                input_size: Some(400),
                output_size: Some(400),
                input_mime: None,
                output_mime: None,
                input_available: true,
                output_available: true,
            },
            active_tab: ContentModalTab::Input,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Pending,
            input: SideBuffer {
                loaded: text.clone().into_bytes(),
                decoded: ContentRender::Text {
                    text,
                    pretty_printed: false,
                },
                in_flight: false,
                fully_loaded: true,
                ceiling_hit: false,
                last_error: None,
            },
            output: SideBuffer::default(),
            diff_cache: None,
            scroll_offset: 50,
            horizontal_scroll_offset: 0,
            last_viewport_rows: 20,
            last_viewport_body_cols: 0,
            search: None,
        };
        s.tracer.content_modal = Some(modal);
        s
    }

    #[test]
    fn handle_focus_up_scrolls_modal_when_open() {
        use crate::input::FocusAction;
        let mut s = state_with_open_modal();
        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Up)
            .expect("Up must be consumed when modal is open");
        assert!(r.redraw);
        assert_eq!(
            s.tracer.content_modal.as_ref().unwrap().scroll_offset,
            49,
            "Up should decrement scroll offset by 1"
        );
    }

    #[test]
    fn handle_focus_down_scrolls_modal_when_open() {
        use crate::input::FocusAction;
        let mut s = state_with_open_modal();
        let r = super::TracerHandler::handle_focus(&mut s, FocusAction::Down)
            .expect("Down must be consumed when modal is open");
        assert!(r.redraw);
        assert_eq!(
            s.tracer.content_modal.as_ref().unwrap().scroll_offset,
            51,
            "Down should increment scroll offset by 1"
        );
    }

    #[test]
    fn handle_focus_first_jumps_to_top_when_modal_open() {
        use crate::input::FocusAction;
        let mut s = state_with_open_modal();
        super::TracerHandler::handle_focus(&mut s, FocusAction::First);
        assert_eq!(
            s.tracer.content_modal.as_ref().unwrap().scroll_offset,
            0,
            "First/Home should jump to offset 0"
        );
    }

    #[test]
    fn handle_focus_last_jumps_to_tail_when_modal_open() {
        use crate::input::FocusAction;
        let mut s = state_with_open_modal();
        super::TracerHandler::handle_focus(&mut s, FocusAction::Last);
        // 100 lines fully loaded, last valid offset = 99
        assert_eq!(
            s.tracer.content_modal.as_ref().unwrap().scroll_offset,
            99,
            "Last/End should jump to last line"
        );
    }

    #[test]
    fn handle_focus_page_down_advances_by_viewport_when_modal_open() {
        use crate::input::FocusAction;
        let mut s = state_with_open_modal();
        // scroll_offset = 50, last_viewport_rows = 20 → new offset = 70
        super::TracerHandler::handle_focus(&mut s, FocusAction::PageDown);
        assert_eq!(
            s.tracer.content_modal.as_ref().unwrap().scroll_offset,
            70,
            "PageDown should advance by viewport rows"
        );
    }

    // ── Modal search text input (C2) ──────────────────────────────────────────

    fn state_with_searchable_modal(body: &str) -> crate::app::state::AppState {
        use crate::client::tracer::ContentRender;
        use crate::view::tracer::state::{
            ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
        };
        use crate::widget::search::SearchState;

        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;
        let text = body.to_owned();
        let modal = ContentModalState {
            event_id: 2,
            header: ContentModalHeader {
                event_type: "CONTENT_MODIFIED".into(),
                event_timestamp_iso: "".into(),
                component_name: "x".into(),
                pg_path: "pg".into(),
                input_size: None,
                output_size: None,
                input_mime: None,
                output_mime: None,
                input_available: true,
                output_available: false,
            },
            active_tab: ContentModalTab::Input,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Pending,
            input: SideBuffer {
                loaded: text.clone().into_bytes(),
                decoded: ContentRender::Text {
                    text,
                    pretty_printed: false,
                },
                fully_loaded: true,
                in_flight: false,
                ceiling_hit: false,
                last_error: None,
            },
            output: SideBuffer::default(),
            diff_cache: None,
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            last_viewport_rows: 10,
            last_viewport_body_cols: 0,
            search: Some(SearchState {
                input_active: true,
                ..Default::default()
            }),
        };
        s.tracer.content_modal = Some(modal);
        s
    }

    #[test]
    fn modal_search_typing_appends_to_query_and_recomputes_matches() {
        let body = "error line\nok line\nerror again";
        let mut s = state_with_searchable_modal(body);
        let c = tiny_config();

        // Simulate typing 'e', 'r', 'r' while search input is active.
        update(&mut s, key(KeyCode::Char('e'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);

        let modal = s.tracer.content_modal.as_ref().unwrap();
        let search = modal.search.as_ref().unwrap();
        assert_eq!(search.query, "err");
        assert_eq!(search.matches.len(), 2, "2 lines contain 'err'");
    }

    #[test]
    fn modal_search_enter_commits_and_picks_first_match() {
        let body = "alpha\nbeta\nalpha again";
        let mut s = state_with_searchable_modal(body);
        let c = tiny_config();

        update(&mut s, key(KeyCode::Char('a'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('l'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Enter, KeyModifiers::NONE), &c);

        let modal = s.tracer.content_modal.as_ref().unwrap();
        let search = modal.search.as_ref().unwrap();
        assert!(
            !search.input_active,
            "Enter should commit (deactivate input)"
        );
        assert!(search.committed, "Enter should set committed = true");
        assert_eq!(search.current, Some(0), "first match selected");
    }

    #[test]
    fn modal_search_esc_cancels() {
        let body = "some text";
        let mut s = state_with_searchable_modal(body);
        let c = tiny_config();

        update(&mut s, key(KeyCode::Esc, KeyModifiers::NONE), &c);

        let modal = s.tracer.content_modal.as_ref().unwrap();
        assert!(
            modal.search.is_none(),
            "Esc should clear search state entirely"
        );
    }

    #[test]
    fn modal_search_backspace_removes_char() {
        let body = "hello world";
        let mut s = state_with_searchable_modal(body);
        let c = tiny_config();

        update(&mut s, key(KeyCode::Char('w'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        update(&mut s, key(KeyCode::Char('r'), KeyModifiers::NONE), &c);

        {
            let modal = s.tracer.content_modal.as_ref().unwrap();
            let search = modal.search.as_ref().unwrap();
            assert_eq!(search.query, "wor");
        }

        update(&mut s, key(KeyCode::Backspace, KeyModifiers::NONE), &c);

        let modal = s.tracer.content_modal.as_ref().unwrap();
        let search = modal.search.as_ref().unwrap();
        assert_eq!(search.query, "wo");
    }

    // ── SearchNext scroll-to-match (C3) ───────────────────────────────────────

    #[test]
    fn modal_search_next_scrolls_offset_into_viewport() {
        use crate::client::tracer::ContentRender;
        use crate::input::ContentModalVerb;
        use crate::view::tracer::state::{
            ContentModalHeader, ContentModalState, ContentModalTab, Diffable, SideBuffer,
        };
        use crate::widget::search::{MatchSpan, SearchState};

        // Build a state where search is committed with matches on lines 0 and 60,
        // current match at 0, viewport 10 rows at offset 0. SearchNext should
        // advance current to Some(1) and scroll to line 60.
        let mut s = fresh_state();
        s.current_tab = ViewId::Tracer;

        let text = "row\n".repeat(100);
        let modal = ContentModalState {
            event_id: 3,
            header: ContentModalHeader {
                event_type: "DROP".into(),
                event_timestamp_iso: "".into(),
                component_name: "x".into(),
                pg_path: "pg".into(),
                input_size: None,
                output_size: None,
                input_mime: None,
                output_mime: None,
                input_available: true,
                output_available: false,
            },
            active_tab: ContentModalTab::Input,
            last_nondiff_tab: ContentModalTab::Input,
            diffable: Diffable::Pending,
            input: SideBuffer {
                loaded: text.clone().into_bytes(),
                decoded: ContentRender::Text {
                    text,
                    pretty_printed: false,
                },
                fully_loaded: true,
                in_flight: false,
                ceiling_hit: false,
                last_error: None,
            },
            output: SideBuffer::default(),
            diff_cache: None,
            scroll_offset: 0,
            horizontal_scroll_offset: 0,
            last_viewport_rows: 10,
            last_viewport_body_cols: 0,
            search: Some(SearchState {
                query: "row".to_owned(),
                input_active: false,
                committed: true,
                matches: vec![
                    MatchSpan {
                        line_idx: 0,
                        byte_start: 0,
                        byte_end: 3,
                    },
                    MatchSpan {
                        line_idx: 60,
                        byte_start: 0,
                        byte_end: 3,
                    },
                ],
                current: Some(0),
            }),
        };
        s.tracer.content_modal = Some(modal);

        // Dispatch SearchNext via handle_verb.
        let r = super::TracerHandler::handle_verb(
            &mut s,
            crate::input::ViewVerb::ContentModal(ContentModalVerb::SearchNext),
        )
        .expect("SearchNext should be consumed");
        assert!(r.redraw);

        let modal = s.tracer.content_modal.as_ref().unwrap();
        let search = modal.search.as_ref().unwrap();
        assert_eq!(search.current, Some(1), "SearchNext advances to match 1");
        assert_eq!(modal.scroll_offset, 60, "scroll must jump to match line 60");
    }
}
