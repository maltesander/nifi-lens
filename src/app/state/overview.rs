//! Overview tab key handler.

use crossterm::event::KeyEvent;

use super::{AppState, UpdateResult, ViewKeyHandler};

/// Zero-sized dispatch struct for the Overview tab.
pub(crate) struct OverviewHandler;

impl ViewKeyHandler for OverviewHandler {
    fn handle_key(_state: &mut AppState, _key: KeyEvent) -> Option<UpdateResult> {
        // Overview has no tab-local keys; everything falls through to globals.
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
    use crate::event::{AppEvent, OverviewPayload, ViewPayload};
    use std::time::SystemTime;

    #[test]
    fn overview_data_event_updates_state_and_triggers_redraw() {
        let mut s = fresh_state();
        let c = tiny_config();
        let payload = OverviewPayload {
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
        };
        let r = update(&mut s, AppEvent::Data(ViewPayload::Overview(payload)), &c);
        assert!(r.redraw);
        let snap = s.overview.snapshot.as_ref().unwrap();
        assert_eq!(snap.controller.running, 7);
    }
}
