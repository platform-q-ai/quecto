//! The ended swarm boards port's contract (#2481 S4c follow-up), run
//! against its in-process adapter: the board of an ended run, as its
//! member left, is kept under the session and the member's label (a
//! board run's coordinator's only), its newest replacing any earlier one;
//! one that is no whole ended run is not kept; and only the newest boards are kept, so a long-lived process
//! never grows without end.
use quecto::application::environments::dto::SwarmBoardSnapshot;
use quecto::application::environments::ports::hosted_swarm::EndedSwarmBoards;
use quecto::application::project_board::dto::SwarmEnd;
use quecto::infrastructure::tools::agents::coordinator_launch::{KeptSwarmBoards, MAX_KEPT};
use serde_json::json;

/// A whole board whose run is `status`, having spent `tokens`.
fn board(status: &str, tokens: u64) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({
        "run": {"status": status, "budget": {"observed_tokens": tokens}},
        "items": [{"title": "a: A", "state": "completed", "owner": "w1", "evidence": []}],
        "totals": {"items": 1},
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
    truncated["totals"]["items"] = json!(2);
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
fn only_a_board_runs_coordinator_is_kept() {
    let kept = KeptSwarmBoards::default();
    kept.keep("s1", "my-ad-hoc-swarm", board("succeeded", 1));
    assert_eq!(kept.view("s1", "my-ad-hoc-swarm"), None);
}

#[test]
fn only_the_newest_boards_are_kept() {
    let kept = KeptSwarmBoards::default();
    let many = MAX_KEPT * 4;
    for member in 0..many {
        kept.keep(
            "s1",
            &format!("board-m{member}-run-1"),
            board("succeeded", member as u64),
        );
    }
    assert_eq!(kept.view("s1", "board-m0-run-1"), None, "the oldest go");
    assert_eq!(
        spent(&kept, "s1", &format!("board-m{}-run-1", many - 1)),
        Some(many as u64 - 1),
        "the newest stays"
    );
}

/// #2674: the board a run had reached as its owner ended its coordinator
/// while it ran or was paused is kept as left, apart from an ended one;
/// one of any other status is not; and either replaces the other.
#[test]
fn a_running_or_paused_runs_board_is_kept_as_left() {
    let kept = KeptSwarmBoards::default();
    let port: &dyn EndedSwarmBoards = &kept;
    port.keep_left("s1", "board-a-run-1", board("running", 5));
    assert_eq!(kept.view("s1", "board-a-run-1"), None, "not an ended run");
    let left = kept.left_view("s1", "board-a-run-1").expect("kept as left");
    assert_eq!((left.end, left.observed_tokens), (SwarmEnd::Cancelled, 5));
    port.keep_left("s1", "board-a-run-2", board("paused", 6));
    assert!(kept.left_view("s1", "board-a-run-2").is_some());
    for status in ["succeeded", "setup", "odd"] {
        port.keep_left("s1", "board-a-run-3", board(status, 7));
        assert_eq!(kept.left_view("s1", "board-a-run-3"), None, "{status}");
    }
    port.keep_left("s1", "my-ad-hoc-swarm", board("running", 8));
    assert_eq!(kept.left_view("s1", "my-ad-hoc-swarm"), None);
    port.keep("s1", "board-a-run-1", board("cancelled", 9));
    assert_eq!(kept.left_view("s1", "board-a-run-1"), None, "replaced");
    assert_eq!(spent(&kept, "s1", "board-a-run-1"), Some(9));
}

/// #2674 round 2: past the bound, a left board goes before any ended one,
/// so boards runs only reached never evict ended runs' boards.
#[test]
fn left_boards_are_evicted_before_ended_ones() {
    let kept = KeptSwarmBoards::default();
    kept.keep_left("s1", "board-left-run-1", board("running", 0));
    for member in 0..MAX_KEPT - 1 {
        kept.keep(
            "s1",
            &format!("board-e{member}-run-1"),
            board("succeeded", 1),
        );
    }
    kept.keep("s1", "board-last-run-1", board("succeeded", 2));
    assert_eq!(
        kept.left_view("s1", "board-left-run-1"),
        None,
        "the left board went"
    );
    assert_eq!(
        spent(&kept, "s1", "board-e0-run-1"),
        Some(1),
        "the oldest ended one stays"
    );
    assert_eq!(spent(&kept, "s1", "board-last-run-1"), Some(2));
    kept.keep_left("s1", "board-new-left-run-1", board("paused", 3));
    assert_eq!(
        spent(&kept, "s1", "board-e0-run-1"),
        Some(1),
        "a left board never evicts an ended one"
    );
}
