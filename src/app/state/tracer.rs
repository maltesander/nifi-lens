//! Tracer tab key handler.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use super::{
    AppState, Banner, BannerSeverity, Modal, PendingIntent, PendingSave, StatusLine, UpdateResult,
    ViewKeyHandler,
};

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

    fn hints(state: &AppState) -> Vec<crate::widget::hint_bar::HintSpan> {
        use crate::view::tracer::state::TracerMode;
        use crate::widget::hint_bar::HintSpan;

        match &state.tracer.mode {
            TracerMode::Entry(_) => vec![
                HintSpan {
                    key: "type",
                    action: "uuid",
                },
                HintSpan {
                    key: "Enter",
                    action: "submit",
                },
                HintSpan {
                    key: "Esc",
                    action: "clear",
                },
            ],
            TracerMode::LatestEvents(_) => vec![
                HintSpan {
                    key: "j/k",
                    action: "nav",
                },
                HintSpan {
                    key: "Enter",
                    action: "detail",
                },
                HintSpan {
                    key: "Esc",
                    action: "back",
                },
            ],
            TracerMode::LineageRunning(_) => vec![HintSpan {
                key: "Esc",
                action: "cancel",
            }],
            TracerMode::Lineage(_) => vec![
                HintSpan {
                    key: "j/k",
                    action: "nav",
                },
                HintSpan {
                    key: "Enter",
                    action: "detail",
                },
                HintSpan {
                    key: "Tab",
                    action: "content",
                },
                HintSpan {
                    key: "s",
                    action: "save",
                },
                HintSpan {
                    key: "Esc",
                    action: "back",
                },
            ],
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
        KeyCode::Down => {
            ts::latest_events_move_down(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Up => {
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
    use crate::intent::Intent;
    use crate::view::tracer::state::{self as ts, ContentPane, EventDetail, TracerMode};

    if !matches!(key.modifiers, KeyModifiers::NONE | KeyModifiers::SHIFT) {
        return None;
    }

    match key.code {
        KeyCode::Down => {
            ts::lineage_move_down(&mut state.tracer);
            Some(UpdateResult {
                redraw: true,
                intent: None,
                tracer_followup: None,
            })
        }
        KeyCode::Up => {
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
        KeyCode::Char('i') => dispatch_content_fetch(state, crate::client::ContentSide::Input),
        KeyCode::Char('o') => dispatch_content_fetch(state, crate::client::ContentSide::Output),
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

/// Handles `i`/`o` in Lineage mode: only dispatches a content fetch when the
/// currently loaded event has a claim on the requested side, otherwise surfaces
/// an info banner. Without this guard NiFi returns HTTP 400 "Input Content
/// Claim not specified" for events like CREATE that have no input claim.
fn dispatch_content_fetch(
    state: &mut AppState,
    side: crate::client::ContentSide,
) -> Option<UpdateResult> {
    use crate::client::ContentSide;
    use crate::intent::{ContentSide as IntentSide, Intent};
    use crate::view::tracer::state::{self as ts, EventDetail, TracerMode};

    let side_label = match side {
        ContentSide::Input => "input",
        ContentSide::Output => "output",
    };

    let (event_id, available) = match state.tracer.mode {
        TracerMode::Lineage(ref view) => match view.event_detail {
            EventDetail::Loaded { ref event, .. } => {
                let avail = match side {
                    ContentSide::Input => event.input_available,
                    ContentSide::Output => event.output_available,
                };
                (event.summary.event_id, avail)
            }
            _ => {
                state.status = StatusLine {
                    banner: Some(Banner {
                        severity: BannerSeverity::Info,
                        message: "press Enter to load event detail first".to_string(),
                        detail: None,
                    }),
                };
                return Some(UpdateResult {
                    redraw: true,
                    intent: None,
                    tracer_followup: None,
                });
            }
        },
        _ => return Some(UpdateResult::default()),
    };

    if !available {
        state.status = StatusLine {
            banner: Some(Banner {
                severity: BannerSeverity::Info,
                message: format!("no {side_label} content for this event"),
                detail: None,
            }),
        };
        return Some(UpdateResult {
            redraw: true,
            intent: None,
            tracer_followup: None,
        });
    }

    ts::lineage_mark_content_loading(&mut state.tracer, side);
    let intent_side = match side {
        ContentSide::Input => IntentSide::Input,
        ContentSide::Output => IntentSide::Output,
    };
    Some(UpdateResult {
        redraw: true,
        intent: Some(PendingIntent::Dispatch(Intent::FetchEventContent {
            event_id,
            side: intent_side,
        })),
        tracer_followup: None,
    })
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

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, key, tiny_config};
    use super::super::update;
    use crate::app::state::{BannerSeverity, PendingIntent, ViewId};
    use crate::client::tracer::ProvenanceEventDetail;
    use crate::intent::Intent;
    use crate::view::tracer::state::{
        self as ts, ContentPane, EventDetail, LineageView, TracerMode,
    };
    use crossterm::event::{KeyCode, KeyModifiers};
    use std::time::SystemTime;

    fn seed_lineage_with_detail(
        input_available: bool,
        output_available: bool,
    ) -> (crate::app::state::AppState, crate::config::Config) {
        use crate::client::LineageSnapshot;
        use crate::client::tracer::ProvenanceEventSummary;

        let mut s = fresh_state();
        let c = tiny_config();
        s.current_tab = ViewId::Tracer;

        let summary = ProvenanceEventSummary {
            event_id: 42,
            event_time_iso: "2026-01-01T00:00:00Z".to_string(),
            event_type: "CREATE".to_string(),
            component_id: "comp-42".to_string(),
            component_name: "GenerateFlowFile".to_string(),
            component_type: "GenerateFlowFile".to_string(),
            group_id: "root".to_string(),
            flow_file_uuid: "ff-42".to_string(),
            relationship: None,
            details: None,
        };
        let detail = ProvenanceEventDetail {
            summary: summary.clone(),
            attributes: vec![],
            transit_uri: None,
            input_available,
            output_available,
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
                content: ContentPane::default(),
            },
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: SystemTime::now(),
        }));
        (s, c)
    }

    #[test]
    fn lineage_i_without_input_claim_shows_info_banner_and_no_intent() {
        let (mut s, c) = seed_lineage_with_detail(false, true);
        let r = update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        assert!(r.intent.is_none());
        let banner = s.status.banner.as_ref().expect("banner should be set");
        assert_eq!(banner.severity, BannerSeverity::Info);
        assert!(
            banner.message.contains("no input content"),
            "unexpected banner: {}",
            banner.message
        );
    }

    #[test]
    fn lineage_o_without_output_claim_shows_info_banner_and_no_intent() {
        let (mut s, c) = seed_lineage_with_detail(true, false);
        let r = update(&mut s, key(KeyCode::Char('o'), KeyModifiers::NONE), &c);
        assert!(r.intent.is_none());
        let banner = s.status.banner.as_ref().expect("banner should be set");
        assert_eq!(banner.severity, BannerSeverity::Info);
        assert!(
            banner.message.contains("no output content"),
            "unexpected banner: {}",
            banner.message
        );
    }

    #[test]
    fn lineage_i_with_input_claim_dispatches_fetch_content_intent() {
        let (mut s, c) = seed_lineage_with_detail(true, true);
        let r = update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        match r.intent {
            Some(PendingIntent::Dispatch(Intent::FetchEventContent { event_id, .. })) => {
                assert_eq!(event_id, 42);
            }
            other => panic!("expected FetchEventContent intent, got {other:?}"),
        }
    }

    #[test]
    fn lineage_i_without_loaded_detail_shows_hint_banner_and_no_intent() {
        let (mut s, c) = seed_lineage_with_detail(true, true);
        // Clear the loaded detail.
        if let TracerMode::Lineage(ref mut view) = s.tracer.mode {
            view.event_detail = EventDetail::NotLoaded;
        }
        let r = update(&mut s, key(KeyCode::Char('i'), KeyModifiers::NONE), &c);
        assert!(r.intent.is_none());
        let banner = s.status.banner.as_ref().expect("banner should be set");
        assert_eq!(banner.severity, BannerSeverity::Info);
        assert!(
            banner.message.contains("Enter"),
            "unexpected banner: {}",
            banner.message
        );
    }

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
            diff_mode: ts::AttributeDiffMode::default(),
            fetched_at: SystemTime::now(),
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
}
