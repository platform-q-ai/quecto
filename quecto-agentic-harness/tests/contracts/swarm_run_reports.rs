//! The swarm run reports port's contract (#2481 S4c follow-up), run
//! against its kept-boards adapter: a run's swarm is reported from the
//! board the teardown kept under the session and the run's coordinator
//! label, with no memory of the launch (reports built after a restart
//! read it the same); a run whose board was not kept, another
//! run or another session's reports nothing; and a board recorded is
//! dropped.
use std::sync::Arc;

use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::EndedSwarmBoards;
use quecto::application::project_board::dto::{SwarmEnd, SwarmItemState};
use quecto::application::project_board::ports::SwarmRunReports;
use quecto::domain::project_board::value_objects::slug::Slug;
use quecto::infrastructure::tools::agents::coordinator_launch::{KeptRunReports, KeptSwarmBoards};
use serde_json::json;

/// The reports of `session` over `kept`, as composition builds them.
fn launcher(kept: &Arc<KeptSwarmBoards>, session: &str) -> KeptRunReports {
    KeptRunReports::new(Arc::clone(kept), session)
}

fn task() -> Slug {
    Slug::parse("light-well-tanking").unwrap()
}

/// A whole board whose run is `status`, with one completed item.
fn board(status: &str) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({
        "run": {"status": status, "budget": {"observed_tokens": 52_000}},
        "items": [{"title": "assess-failure: Assess", "state": "completed", "owner": "w1",
                   "evidence": [{"artifact": "docs/a.md", "revision": "abc"}]}],
        "totals": {"items": 1},
        "truncated": false,
    }))
    .unwrap()
}

#[test]
fn a_runs_swarm_is_reported_from_the_board_kept_under_its_label() {
    let kept = Arc::new(KeptSwarmBoards::default());
    assert_eq!(
        launcher(&kept, "s1").ended_run(&task(), 1),
        None,
        "none kept"
    );
    kept.keep("s1", "board-light-well-tanking-run-1", board("succeeded"));
    let restarted = launcher(&kept, "s1");
    let view = restarted.ended_run(&task(), 1).expect("an ended run");
    assert_eq!(
        (view.end, view.observed_tokens, view.items[0].state),
        (SwarmEnd::Succeeded, 52_000, SwarmItemState::Completed)
    );
    assert_eq!(restarted.ended_run(&task(), 2), None, "another run");
    assert_eq!(
        launcher(&kept, "s2").ended_run(&task(), 1),
        None,
        "another session"
    );
    restarted.recorded(&task(), 1);
    assert_eq!(
        restarted.ended_run(&task(), 1),
        None,
        "dropped once recorded"
    );
}
