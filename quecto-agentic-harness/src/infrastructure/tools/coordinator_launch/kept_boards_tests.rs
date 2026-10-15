use serde_json::{Value, json};

use std::sync::Arc;

use super::*;
use crate::application::project_board::dto::SwarmEnd;

/// An ended board whose run is `status`, having spent `tokens`.
fn snapshot(status: &str, tokens: u64) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({
        "run": {"status": status, "budget": {"observed_tokens": tokens}},
        "items": [],
        "truncated": false,
    }))
    .unwrap()
}

fn tokens(view: Option<SwarmRunView>) -> Option<u64> {
    view.map(|view| view.observed_tokens)
}

/// A member's board is kept under its session and label, its newest
/// replacing any earlier one; another session's is its own.
#[test]
fn a_members_newest_board_is_kept_under_its_session_and_label() {
    let kept = KeptSwarmBoards::default();
    assert_eq!(kept.view("s1", "board-a-run-1"), None, "nothing kept yet");
    kept.keep("s1", "board-a-run-1", snapshot("succeeded", 1));
    kept.keep("s1", "board-a-run-2", snapshot("failed", 2));
    kept.keep("s1", "board-a-run-1", snapshot("succeeded", 3));
    kept.keep("s2", "board-a-run-1", snapshot("cancelled", 4));
    assert_eq!(tokens(kept.view("s1", "board-a-run-1")), Some(3));
    assert_eq!(tokens(kept.view("s1", "board-a-run-2")), Some(2));
    assert_eq!(
        kept.view("s2", "board-a-run-1").map(|view| view.end),
        Some(SwarmEnd::Cancelled)
    );
    assert_eq!(kept.view("s1", "board-b-run-1"), None);
}

/// A board recorded is dropped; others stay.
#[test]
fn a_forgotten_board_is_dropped() {
    let kept = KeptSwarmBoards::default();
    kept.keep("s1", "board-a-run-1", snapshot("succeeded", 1));
    kept.keep("s1", "board-a-run-2", snapshot("succeeded", 2));
    kept.forget("s1", "board-a-run-1");
    kept.forget("s1", "never-kept");
    assert_eq!(kept.view("s1", "board-a-run-1"), None);
    assert_eq!(tokens(kept.view("s1", "board-a-run-2")), Some(2));
}

/// Past [`MAX_KEPT`] boards, the oldest goes.
#[test]
fn only_the_newest_boards_are_kept() {
    let kept = KeptSwarmBoards::default();
    for member in 0..=MAX_KEPT {
        kept.keep(
            "s1",
            &format!("m{member}"),
            snapshot("succeeded", member as u64),
        );
    }
    assert_eq!(kept.view("s1", "m0"), None, "the oldest went");
    assert_eq!(tokens(kept.view("s1", "m1")), Some(1));
    assert_eq!(
        tokens(kept.view("s1", &format!("m{MAX_KEPT}"))),
        Some(MAX_KEPT as u64)
    );
}

/// Only a whole ended run is kept: not one still running or paused, nor
/// a projection that left rows out; an earlier board stays as it was.
#[test]
fn only_a_whole_ended_run_is_kept() {
    let kept = KeptSwarmBoards::default();
    kept.keep("s1", "board-a-run-1", snapshot("succeeded", 1));
    let mut truncated = snapshot("succeeded", 9).into_value();
    truncated["truncated"] = Value::Bool(true);
    for board in [
        snapshot("running", 9),
        snapshot("paused", 9),
        SwarmBoardSnapshot::object(truncated).unwrap(),
    ] {
        kept.keep("s1", "board-a-run-1", board);
        kept.keep("s1", "board-b-run-1", snapshot("paused", 9));
    }
    assert_eq!(tokens(kept.view("s1", "board-a-run-1")), Some(1));
    assert_eq!(kept.view("s1", "board-b-run-1"), None);
}

/// #2481 S4c follow-up: a run's swarm is reported from the board kept
/// under the session and the label its coordinator is spawned with,
/// with no memory of the launch (reports built afresh, as after a
/// restart, read it the same); another run, task or session reads none;
/// once recorded, the board is dropped.
#[test]
fn a_runs_swarm_is_reported_from_the_board_kept_under_its_label() {
    let kept = Arc::new(KeptSwarmBoards::default());
    let task = Slug::parse("light-well-tanking").unwrap();
    let reports = |session: &str| KeptRunReports::new(Arc::clone(&kept), session);
    assert_eq!(reports("s1").ended_run(&task, 1), None, "nothing kept");
    kept.keep(
        "s1",
        &super::super::label("light-well-tanking", 1),
        snapshot("succeeded", 52_000),
    );
    let restarted = reports("s1");
    assert_eq!(tokens(restarted.ended_run(&task, 1)), Some(52_000));
    assert_eq!(restarted.ended_run(&task, 2), None, "another run");
    assert_eq!(
        restarted.ended_run(&Slug::parse("other").unwrap(), 1),
        None,
        "another task"
    );
    assert_eq!(reports("s2").ended_run(&task, 1), None, "another session");
    restarted.recorded(&task, 1);
    assert_eq!(restarted.ended_run(&task, 1), None, "dropped once recorded");
}
