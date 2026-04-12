//! Tracer tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{AppState, Modal, PendingIntent, PendingSave, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Tracer tab.
pub(crate) struct TracerHandler;

impl ViewKeyHandler for TracerHandler {
    fn handle_key(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
        use crate::view::tracer::state::TracerMode;

        match &state.tracer.mode {
            TracerMode::Entry(_) => handle_entry(state, key),
            TracerMode::LatestEvents(_) => handle_latest_events(state, key),
            TracerMode::LineageRunning(_) => handle_lineage_running(state, key),
            TracerMode::Lineage(_) => handle_lineage(state, key),
        }
    }
}

fn handle_entry(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    use crate::intent::Intent;
    use crate::view::tracer::state as ts;

    // Entry mode: character keys go into the UUID input. Ctrl modifiers
    // fall through to global handlers.
    match (key.code, key.modifiers) {
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

fn handle_latest_events(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    use crate::intent::Intent;
    use crate::view::tracer::state as ts;

    if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
        return None;
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            ts::latest_events_move_down(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Up | KeyCode::Char('k') => {
            ts::latest_events_move_up(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Enter => {
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
        KeyCode::Char('r') => {
            if let ts::TracerMode::LatestEvents(ref view) = state.tracer.mode {
                let component_id = view.component_id.clone();
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(Intent::RefreshLatestEvents {
                        component_id,
                    })),
                    tracer_followup: None,
                })
            } else {
                Some(UpdateResult::default())
            }
        }
        KeyCode::Esc => {
            ts::cancel_lineage(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Char('c') => {
            if let Some(uuid) = ts::latest_events_selected_uuid(&state.tracer) {
                super::clipboard_copy(state, &uuid);
            }
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        _ => None,
    }
}

fn handle_lineage_running(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    use crate::view::tracer::state as ts;

    if key.code == KeyCode::Esc {
        let followup = ts::cancel_lineage(&mut state.tracer);
        Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: followup,
        })
    } else {
        Some(UpdateResult::default())
    }
}

fn handle_lineage(state: &mut AppState, key: KeyEvent) -> Option<UpdateResult> {
    use crate::intent::{ContentSide as IntentSide, Intent};
    use crate::view::tracer::state::{self as ts, ContentPane, EventDetail, TracerMode};

    if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
        return None;
    }

    match key.code {
        KeyCode::Down | KeyCode::Char('j') => {
            ts::lineage_move_down(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Up | KeyCode::Char('k') => {
            ts::lineage_move_up(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Enter => {
            if let Some(event_id) = ts::lineage_selected_event_id(&state.tracer) {
                ts::lineage_mark_detail_loading(&mut state.tracer);
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(Intent::LoadEventDetail {
                        event_id,
                    })),
                    tracer_followup: None,
                })
            } else {
                Some(UpdateResult::default())
            }
        }
        KeyCode::Char('i') => {
            if let Some(event_id) = ts::lineage_selected_event_id(&state.tracer) {
                ts::lineage_mark_content_loading(
                    &mut state.tracer,
                    crate::client::ContentSide::Input,
                );
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(Intent::FetchEventContent {
                        event_id,
                        side: IntentSide::Input,
                    })),
                    tracer_followup: None,
                })
            } else {
                Some(UpdateResult::default())
            }
        }
        KeyCode::Char('o') => {
            if let Some(event_id) = ts::lineage_selected_event_id(&state.tracer) {
                ts::lineage_mark_content_loading(
                    &mut state.tracer,
                    crate::client::ContentSide::Output,
                );
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(Intent::FetchEventContent {
                        event_id,
                        side: IntentSide::Output,
                    })),
                    tracer_followup: None,
                })
            } else {
                Some(UpdateResult::default())
            }
        }
        KeyCode::Char('s') => {
            // Open the save modal if content is currently shown.
            if let TracerMode::Lineage(ref view) = state.tracer.mode
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
            Some(UpdateResult::default())
        }
        KeyCode::Char('a') => {
            ts::lineage_toggle_diff_mode(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Char('r') => {
            if let TracerMode::Lineage(ref view) = state.tracer.mode {
                let uuid = view.uuid.clone();
                Some(UpdateResult {
                    redraw: true,
                    intent: Some(PendingIntent::Dispatch(Intent::RefreshLineage { uuid })),
                    tracer_followup: None,
                })
            } else {
                Some(UpdateResult::default())
            }
        }
        KeyCode::Esc => {
            ts::cancel_lineage(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Char('c') => {
            // Copy the selected event's flowfile UUID.
            let uuid = if let TracerMode::Lineage(ref view) = state.tracer.mode {
                view.snapshot
                    .events
                    .get(view.selected_event)
                    .map(|ev| ev.flow_file_uuid.clone())
            } else {
                None
            };
            if let Some(uuid) = uuid {
                super::clipboard_copy(state, &uuid);
            }
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        _ => None,
    }
}

/// Extracts the raw bytes from the Tracer's content pane and builds a
/// `PendingSave`. Returns `None` if the content pane is not in the
/// `Shown` state.
pub(super) fn extract_raw_for_save(
    state: &AppState,
    path: std::path::PathBuf,
) -> Option<PendingIntent> {
    use crate::view::tracer::state::{ContentPane, EventDetail, TracerMode};
    if let TracerMode::Lineage(ref view) = state.tracer.mode
        && let EventDetail::Loaded { ref content, .. } = view.event_detail
        && let ContentPane::Shown { raw, .. } = content
    {
        Some(PendingIntent::SaveEventContent(PendingSave {
            path,
            raw: std::sync::Arc::clone(raw),
        }))
    } else {
        None
    }
}
