use serde_json::json;

use super::*;

fn snapshot(revision: i64) -> SwarmBoardSnapshot {
    SwarmBoardSnapshot::object(json!({"revision": revision})).unwrap()
}

/// A member's board is kept, and its newest replaces any earlier one.
#[test]
fn a_members_newest_board_is_kept() {
    let kept = KeptSwarmBoards::default();
    assert_eq!(kept.board("7f3a"), None, "nothing kept yet");
    kept.keep("7f3a", snapshot(1));
    kept.keep("9c1d", snapshot(2));
    kept.keep("7f3a", snapshot(3));
    assert_eq!(kept.board("7f3a"), Some(json!({"revision": 3})));
    assert_eq!(kept.board("9c1d"), Some(json!({"revision": 2})));
    assert_eq!(kept.board("other"), None);
}

/// Past [`MAX_KEPT`] boards, the oldest goes.
#[test]
fn only_the_newest_boards_are_kept() {
    let kept = KeptSwarmBoards::default();
    for member in 0..=MAX_KEPT {
        kept.keep(&format!("m{member}"), snapshot(member as i64));
    }
    assert_eq!(kept.board("m0"), None, "the oldest went");
    assert_eq!(kept.board("m1"), Some(json!({"revision": 1})));
    assert_eq!(
        kept.board(&format!("m{MAX_KEPT}")),
        Some(json!({"revision": MAX_KEPT}))
    );
}

/// Every caller reaches the one process store.
#[test]
fn the_process_keeps_one_store() {
    assert!(Arc::ptr_eq(&process_kept_boards(), &process_kept_boards()));
}
