//! The ended swarm boards port's contract (#2481 S4c follow-up), run
//! against its in-process adapter: the board of an ended run, as its
//! member left, is kept under the session and the member's label, its
//! newest replacing any earlier one; one that is no whole ended run is not
//! kept; and only the newest boards are kept, so a long-lived process
//! never grows without end.
use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::EndedSwarmBoards;
use quecto::application::project_board::dto::SwarmEnd;
use quecto::infrastructure::tools::coordinator_launch::{KeptSwarmBoards, MAX_KEPT};
use serde_json::json;

/// A whole board whose run is `status`, having spent `tokens`.
fn board(status: &str, tokens: u64) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({
        "run": {"status": status, "budget": {"observed_tokens": tokens}},
        "items": [{"title": "a: A", "state": "completed", "owner": "w1", "evidence": []}],
        "truncated": false,
    }))
    .unwrap()
}

fn spent(kept: &KeptSwarmBoards, session: &str, label: &str) -> Option<u64> {
    kept.view(session, label).map(|view| view.observed_tokens)
}

#[test]
fn a_board_is_kept_under_its_session_and_label_the_newest_replacing_any_earlier() {
    let kept = KeptSwarmBoards::default();
    let port: &dyn EndedSwarmBoards = &kept;
    port.keep("s1", "board-a-run-1", board("succeeded", 1));
    port.keep("s1", "board-a-run-2", board("failed", 2));
    port.keep("s1", "board-a-run-1", board("succeeded", 3));
    port.keep("s2", "board-a-run-1", board("cancelled", 4));
    assert_eq!(spent(&kept, "s1", "board-a-run-1"), Some(3));
    assert_eq!(spent(&kept, "s1", "board-a-run-2"), Some(2));
    assert_eq!(
        kept.view("s2", "board-a-run-1").map(|view| view.end),
        Some(SwarmEnd::Cancelled)
    );
    assert_eq!(kept.view("s1", "board-b-run-1"), None);
    kept.forget("s1", "board-a-run-1");
    assert_eq!(kept.view("s1", "board-a-run-1"), None, "forgotten");
}

#[test]
fn only_a_whole_ended_run_is_kept() {
    let kept = KeptSwarmBoards::default();
    let mut truncated = board("succeeded", 1).into_value();
    truncated["truncated"] = json!(true);
    for snapshot in [
        board("running", 1),
        board("paused", 1),
        SwarmBoardSnapshot::object(truncated).unwrap(),
    ] {
        kept.keep("s1", "board-a-run-1", snapshot);
    }
    assert_eq!(kept.view("s1", "board-a-run-1"), None);
}

#[test]
fn only_the_newest_boards_are_kept() {
    let kept = KeptSwarmBoards::default();
    let many = MAX_KEPT * 4;
    for member in 0..many {
        kept.keep(
            "s1",
            &format!("m{member}"),
            board("succeeded", member as u64),
        );
    }
    assert_eq!(kept.view("s1", "m0"), None, "the oldest go");
    assert_eq!(
        spent(&kept, "s1", &format!("m{}", many - 1)),
        Some(many as u64 - 1),
        "the newest stays"
    );
}
