use serde_json::{Value, json};

use super::ReadInbox;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredMessage, member_row, running_board,
};
use crate::application::swarm::dto::ReadInboxRequest;

/// Messages 1 (consumed) and 3 (unread) to `parent`, and 2 to `worker`.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "dead"));
    for (id, recipient, status) in [
        (1, "parent", "consumed"),
        (2, "worker", "accepted"),
        (3, "parent", "accepted"),
    ] {
        state.messages.push(StoredMessage {
            id,
            sender: "worker".to_owned(),
            recipient: json!(recipient),
            body: "b".to_owned(),
            status: status.to_owned(),
            ..StoredMessage::default()
        });
    }
    state
}

fn ids(board: &std::sync::Arc<MemoryBoard>, actor: &str, include_consumed: Value) -> Vec<Value> {
    ReadInbox::new(board.clone(), SteppingClock::fixed(50.0))
        .execute(ReadInboxRequest {
            actor: actor.to_owned(),
            include_consumed,
        })
        .unwrap()
        .into_iter()
        .map(|row| row.get("id").cloned().unwrap())
        .collect()
}

/// The caller's unread messages, or every message to it with the flag; a
/// dead member may still read its own; nothing is written.
#[test]
fn the_inbox_is_the_callers_own() {
    let board = MemoryBoard::with(board());
    assert_eq!(ids(&board, "parent", json!(false)), [json!(3)]);
    assert_eq!(ids(&board, "parent", json!(true)), [json!(1), json!(3)]);
    assert_eq!(ids(&board, "worker", json!(false)), [json!(2)]);
    assert!(board.snapshot().events.is_empty());
}
