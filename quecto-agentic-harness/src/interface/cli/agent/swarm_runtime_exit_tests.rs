//! #2338 review round 1: the agent command writes the run watch's held
//! ticks however it ends.
use std::sync::{Arc, Mutex};

use serde_json::json;

use super::WatchTicksOnExit;
use crate::application::swarm::dto::DroppedRecords;
use crate::application::swarm::ports::{BoardOpLog, SessionOpLog};
use crate::composition::swarm::{board_wire, build_swarm_board_handles};
use crate::domain::swarm::{BoardOpObservation, SwarmRunSummary};
use crate::infrastructure::tools::swarm_bridge::{SwarmBoard, SwarmContext};

#[derive(Default)]
struct Recorded(Mutex<Vec<BoardOpObservation>>);

impl BoardOpLog for Recorded {
    fn record(&self, observation: BoardOpObservation) {
        self.0.lock().unwrap().push(observation);
    }

    fn summarize(&self, _summary: SwarmRunSummary) {}
}

impl SessionOpLog for Recorded {
    fn dropped(&self, _drops: DroppedRecords) {}

    fn take_unnoted(&self) -> DroppedRecords {
        DroppedRecords::default()
    }
}

#[test]
fn the_command_ending_writes_the_ticks_the_process_board_held() {
    let directory = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(directory.path().join(".quecto")).unwrap();
    let board: &'static SwarmBoard = Box::leak(Box::new(SwarmBoard::new(
        build_swarm_board_handles,
        board_wire(),
    )));
    let log = Arc::new(Recorded::default());
    assert!(board.record_in(log.clone()));
    let context = SwarmContext {
        lifecycle: Arc::new(crate::application::swarm::LifecycleService),
        checkout: directory.path().to_path_buf(),
        member: "coordinator".into(),
        board: board.clone(),
    };
    let deadline = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs_f64()
        + 600.0;
    context
        .call(
            "create",
            json!(["exit", [], [{"id":"t","kind":"command","description":"pass"}], 2, deadline]),
        )
        .unwrap();
    let cursor = context.watch(None).unwrap().event_cursor;
    for _ in 0..3 {
        assert!(context.watch(Some(cursor)).unwrap().snapshot.is_none());
    }
    let held = |log: &Recorded| {
        log.0
            .lock()
            .unwrap()
            .iter()
            .filter(|record| record.op == "_watch")
            .map(|record| record.detail.polls.unwrap_or(1))
            .sum::<u64>()
    };
    assert_eq!(held(&log), 1, "the three unchanged ticks are held");
    drop(WatchTicksOnExit::of(Some(board)));
    assert_eq!(held(&log), 4, "written when the command ends");
    drop(context);
}
