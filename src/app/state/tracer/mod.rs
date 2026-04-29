//! Tracer tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Modal, PendingIntent, PendingSave, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Tracer tab.
pub(crate) struct TracerHandler;

impl ViewKeyHandler for TracerHandler {
    fn handle_verb(state: &mut AppState, verb: crate::input::ViewVerb) -> Option<UpdateResult> {
        use crate::input::{CommonVerb, TracerVerb};
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
            TracerVerb::Common(CommonVerb::Refresh) => match &state.tracer.mode {
                TracerMode::Lineage(view) => {
                    let uuid = view.uuid.clone();
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::Dispatch(
                            crate::intent::Intent::RefreshLineage { uuid },
                        )),
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    });
                }
                _ => {}
            },
            TracerVerb::Common(CommonVerb::Copy) => {
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                    let cfg = state.tracer_config.ceiling.clone();
                    let fired = open_content_modal(&mut state.tracer, &detail, active_tab, &cfg);
                    return Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::SpawnModalChunks(fired)),
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    });
                }
                return Some(UpdateResult::default());
            }
            // OpenSearch / SearchNext / SearchPrev / Close are not bound on
            // the Tracer top-level (search and close are inside the content
            // modal). Keep the match exhaustive.
            TracerVerb::Common(
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
            let cfg = state.tracer_config.ceiling.clone();
            let fired = match action {
                FocusAction::Up => ts::content_modal_scroll_by(&mut state.tracer, -1, &cfg),
                FocusAction::Down => ts::content_modal_scroll_by(&mut state.tracer, 1, &cfg),
                FocusAction::PageUp => {
                    let rows = state
                        .tracer
                        .content_modal
                        .as_ref()
                        .map(|m| m.scroll.vertical.last_viewport_rows.max(1))
                        .unwrap_or(1) as isize;
                    ts::content_modal_scroll_by(&mut state.tracer, -rows, &cfg)
                }
                FocusAction::PageDown => {
                    let rows = state
                        .tracer
                        .content_modal
                        .as_ref()
                        .map(|m| m.scroll.vertical.last_viewport_rows.max(1))
                        .unwrap_or(1) as isize;
                    ts::content_modal_scroll_by(&mut state.tracer, rows, &cfg)
                }
                FocusAction::First => {
                    ts::content_modal_scroll_horizontal_home(&mut state.tracer);
                    ts::content_modal_scroll_to(&mut state.tracer, 0, &cfg)
                }
                FocusAction::Last => {
                    let line_count = ts::content_modal_line_count(&state.tracer);
                    ts::content_modal_scroll_to(
                        &mut state.tracer,
                        line_count.saturating_sub(1),
                        &cfg,
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
                sparkline_followup: None,
                queue_listing_followup: None,
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
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        })
                    } else {
                        Some(UpdateResult {
                            redraw: true,
                            intent: None,
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
                        })
                    }
                }
                FocusAction::Ascend => {
                    ts::handle_entry_clear(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    })
                }
                FocusAction::Down => {
                    ts::latest_events_move_down(&mut state.tracer);
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    })
                }
                FocusAction::Descend => {
                    if let Some(uuid) = ts::latest_events_selected_uuid(&state.tracer) {
                        Some(UpdateResult {
                            redraw: true,
                            intent: Some(PendingIntent::Dispatch(Intent::TraceFlowfile(uuid))),
                            tracer_followup: None,
                            sparkline_followup: None,
                            queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                        sparkline_followup: None,
                        queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
                            })
                        }
                        FocusAction::Down => {
                            ts::lineage_move_down(&mut state.tracer);
                            let intent = lineage_load_detail_intent(&mut state.tracer);
                            Some(UpdateResult {
                                redraw: true,
                                intent,
                                tracer_followup: None,
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                sparkline_followup: None,
                                queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
                                })
                            }
                            FocusAction::Descend => None,
                            FocusAction::Ascend => {
                                ts::lineage_focus_timeline(&mut state.tracer);
                                Some(UpdateResult {
                                    redraw: true,
                                    intent: None,
                                    tracer_followup: None,
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
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
                                    sparkline_followup: None,
                                    queue_listing_followup: None,
                                })
                            }
                            FocusAction::PrevPane => {
                                // Exit to Timeline — same as Ascend.
                                ts::lineage_focus_timeline(&mut state.tracer);
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
                })
            }
            (KeyCode::Char(ch), KeyModifiers::NONE) | (KeyCode::Char(ch), KeyModifiers::SHIFT) => {
                ts::handle_entry_char(&mut state.tracer, ch);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                })
            }
            (KeyCode::Backspace, KeyModifiers::NONE) => {
                ts::handle_entry_backspace(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                })
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) | (KeyCode::Esc, _) => {
                ts::handle_entry_clear(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
                })
            }
            (KeyCode::Enter, _) => {
                if let Some(uuid) = ts::entry_submit(&mut state.tracer) {
                    Some(UpdateResult {
                        redraw: true,
                        intent: Some(PendingIntent::Dispatch(Intent::TraceFlowfile(uuid))),
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
                    })
                } else {
                    Some(UpdateResult {
                        redraw: true,
                        intent: None,
                        tracer_followup: None,
                        sparkline_followup: None,
                        queue_listing_followup: None,
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

/// Builds a `PendingSave` referencing the event + side that should be
/// saved. Prefers the content viewer modal's active side when the
/// modal is open (since that's the user's current focus and the save
/// prompt was opened from there); falls back to the inline content
/// pane's `Shown` state otherwise. Returns `None` when neither source
/// can pin down a side.
///
/// The worker re-fetches the full body at save time rather than
/// re-using any preview's (potentially truncated) bytes.
pub(super) fn build_pending_save(
    state: &AppState,
    path: std::path::PathBuf,
) -> Option<PendingIntent> {
    use crate::view::tracer::state::{ContentModalTab, ContentPane, EventDetail, TracerMode};

    // 1. Content modal is the authoritative source when open — the
    //    save prompt was almost certainly triggered from it.
    if let Some(modal) = state.tracer.content_modal.as_ref() {
        let side = match modal.active_tab {
            ContentModalTab::Diff => match modal.last_nondiff_tab {
                ContentModalTab::Output => crate::client::ContentSide::Output,
                _ => crate::client::ContentSide::Input,
            },
            ContentModalTab::Output => crate::client::ContentSide::Output,
            ContentModalTab::Input => crate::client::ContentSide::Input,
        };
        return Some(PendingIntent::SaveEventContent(PendingSave {
            path,
            event_id: modal.event_id,
            side,
        }));
    }

    // 2. Fallback to the inline content pane (TracerVerb::Save path).
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
            let cfg = state.tracer_config.ceiling.clone();
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, &cfg);
            if !fired.is_empty() {
                return Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::SpawnModalChunks(fired)),
                    tracer_followup: None,
                    sparkline_followup: None,
                    queue_listing_followup: None,
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
        sparkline_followup: None,
        queue_listing_followup: None,
    })
}

// ── ContentModalVerb dispatch ─────────────────────────────────────────────────

/// Dispatch a `ContentModalVerb` action. Called when the content viewer modal
/// is open and the keymap has routed `ViewVerb::ContentModal(v)` here.
fn handle_content_modal_verb(
    state: &mut AppState,
    v: crate::input::ContentModalVerb,
) -> UpdateResult {
    use crate::input::{CommonVerb, ContentModalVerb};
    use crate::view::tracer::state::{
        self as ts, ContentModalTab, Diffable, close_content_modal, content_modal_copy_text,
        hunk_next, hunk_prev, switch_content_modal_tab,
    };

    match v {
        ContentModalVerb::Common(CommonVerb::Close) => {
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
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::Common(CommonVerb::Copy) => {
            if let Some(text) = content_modal_copy_text(&state.tracer) {
                super::clipboard_copy(state, &text);
            }
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
            let cfg = state.tracer_config.ceiling.clone();
            let fired = switch_content_modal_tab(&mut state.tracer, next, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
            let cfg = state.tracer_config.ceiling.clone();
            let fired = switch_content_modal_tab(&mut state.tracer, prev, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::JumpInput => {
            let cfg = state.tracer_config.ceiling.clone();
            let fired = switch_content_modal_tab(&mut state.tracer, ContentModalTab::Input, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::JumpOutput => {
            let cfg = state.tracer_config.ceiling.clone();
            let fired = switch_content_modal_tab(&mut state.tracer, ContentModalTab::Output, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
            let cfg = state.tracer_config.ceiling.clone();
            let fired = switch_content_modal_tab(&mut state.tracer, ContentModalTab::Diff, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::HunkNext => {
            hunk_next(&mut state.tracer);
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::HunkPrev => {
            hunk_prev(&mut state.tracer);
            UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }

        ContentModalVerb::Common(CommonVerb::OpenSearch) => {
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
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        ContentModalVerb::Common(CommonVerb::SearchNext) => {
            if let Some(modal) = state.tracer.content_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && s.committed
                && !s.matches.is_empty()
            {
                let i = s.current.unwrap_or(0);
                s.current = Some((i + 1) % s.matches.len());
            }
            let cfg = state.tracer_config.ceiling.clone();
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
            }
        }
        ContentModalVerb::Common(CommonVerb::SearchPrev) => {
            if let Some(modal) = state.tracer.content_modal.as_mut()
                && let Some(s) = modal.search.as_mut()
                && s.committed
                && !s.matches.is_empty()
            {
                let i = s.current.unwrap_or(0);
                let n = s.matches.len();
                s.current = Some((i + n - 1) % n);
            }
            let cfg = state.tracer_config.ceiling.clone();
            let fired = ts::content_modal_scroll_to_match(&mut state.tracer, &cfg);
            UpdateResult {
                redraw: true,
                intent: if fired.is_empty() {
                    None
                } else {
                    Some(PendingIntent::SpawnModalChunks(fired))
                },
                tracer_followup: None,
                sparkline_followup: None,
                queue_listing_followup: None,
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
                    sparkline_followup: None,
                    queue_listing_followup: None,
                }
            } else {
                UpdateResult::default()
            }
        }
        // Refresh is not bound inside the content modal — keep the match
        // exhaustive over the lifted Common chord set.
        ContentModalVerb::Common(CommonVerb::Refresh) => UpdateResult::default(),
    }
}

#[cfg(test)]
mod tests;
