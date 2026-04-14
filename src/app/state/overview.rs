//! Overview tab key handler.

use super::{AppState, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Overview tab.
pub(crate) struct OverviewHandler;

impl ViewKeyHandler for OverviewHandler {
    fn handle_verb(_state: &mut AppState, _verb: crate::input::ViewVerb) -> Option<UpdateResult> {
        None
    }

    fn handle_focus(
        _state: &mut AppState,
        _action: crate::input::FocusAction,
    ) -> Option<UpdateResult> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::super::tests::{fresh_state, tiny_config};
    use super::super::update;
    use crate::client::{
        AboutSnapshot, BulletinBoardSnapshot, ControllerStatusSnapshot, RootPgStatusSnapshot,
    };
    use crate::event::{AppEvent, OverviewPayload, OverviewPgStatusPayload, ViewPayload};
    use std::time::SystemTime;

    #[test]
    fn overview_handle_verb_is_noop() {
        use crate::app::state::ViewKeyHandler;
        use crate::input::{BulletinsVerb, FocusAction, ViewVerb};
        let mut s = fresh_state();
        s.current_tab = crate::app::state::ViewId::Overview;
        assert!(
            super::OverviewHandler::handle_verb(
                &mut s,
                ViewVerb::Bulletins(BulletinsVerb::Refresh)
            )
            .is_none()
        );
        assert!(super::OverviewHandler::handle_focus(&mut s, FocusAction::Descend).is_none());
    }

    #[test]
    fn overview_data_event_updates_state_and_triggers_redraw() {
        let mut s = fresh_state();
        let c = tiny_config();
        let payload = OverviewPayload::PgStatus(OverviewPgStatusPayload {
            about: AboutSnapshot {
                version: "2.8.0".into(),
                title: "NiFi".into(),
            },
            controller: ControllerStatusSnapshot {
                running: 7,
                stopped: 3,
                invalid: 0,
                disabled: 1,
                active_threads: 0,
                flow_files_queued: 0,
                bytes_queued: 0,
            },
            root_pg: RootPgStatusSnapshot::default(),
            bulletin_board: BulletinBoardSnapshot::default(),
            fetched_at: SystemTime::now(),
        });
        let r = update(&mut s, AppEvent::Data(ViewPayload::Overview(payload)), &c);
        assert!(r.redraw);
        let snap = s.overview.snapshot.as_ref().unwrap();
        assert_eq!(snap.controller.running, 7);
    }
}
