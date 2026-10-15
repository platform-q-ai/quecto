//! The ended swarm boards port's contract (#2481 S4c follow-up), run
//! against its in-process adapter: the board a swarm environment held as
//! its member left is kept under that member, its newest replacing any
//! earlier one, and only the newest boards are kept, so a long-lived
//! process never grows without end. Every caller reaches one store.
use std::sync::Arc;

use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::EndedSwarmBoards;
use quecto::infrastructure::tools::coordinator_launch::{KeptSwarmBoards, process_kept_boards};
use serde_json::json;

fn snapshot(revision: usize) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({"revision": revision})).unwrap()
}

#[test]
fn a_board_is_kept_under_its_member_the_newest_replacing_any_earlier() {
    let kept = KeptSwarmBoards::default();
    let port: &dyn EndedSwarmBoards = &kept;
    port.keep("coordinator-1", snapshot(1));
    port.keep("coordinator-2", snapshot(2));
    port.keep("coordinator-1", snapshot(3));
    assert_eq!(kept.board("coordinator-1"), Some(json!({"revision": 3})));
    assert_eq!(kept.board("coordinator-2"), Some(json!({"revision": 2})));
    assert_eq!(kept.board("coordinator-3"), None);
}

#[test]
fn only_the_newest_boards_are_kept() {
    let kept = KeptSwarmBoards::default();
    let many = 100;
    for member in 0..many {
        kept.keep(&format!("m{member}"), snapshot(member));
    }
    assert_eq!(kept.board("m0"), None, "the oldest go");
    assert_eq!(
        kept.board(&format!("m{}", many - 1)),
        Some(json!({"revision": many - 1})),
        "the newest stays"
    );
}

#[test]
fn every_caller_reaches_the_process_store() {
    let kept = process_kept_boards();
    assert!(Arc::ptr_eq(&kept, &process_kept_boards()));
    kept.keep("contract-member-3e1f", snapshot(7));
    assert_eq!(
        process_kept_boards().board("contract-member-3e1f"),
        Some(json!({"revision": 7}))
    );
}
