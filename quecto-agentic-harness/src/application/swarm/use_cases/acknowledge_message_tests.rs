use serde_json::{Value, json};

use super::AcknowledgeMessage;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, StoredMessage, member_row, running_board,
};
use crate::application::swarm::dto::{MessageIdRequest, Settled};
use crate::domain::swarm::{BoardError, RefusalKind};

/// Message 1 from `worker` to `parent` unread, message 2 superseded, and
/// message 3 to `worker`.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    for (id, recipient, status) in [
        (1, "parent", "accepted"),
        (2, "parent", "superseded"),
        (3, "worker", "accepted"),
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

fn ack(actor: &str, message_id: Value) -> MessageIdRequest {
    MessageIdRequest {
        actor: actor.to_owned(),
        message_id,
    }
}

/// An unread message in the caller's inbox is consumed once, recorded as
/// `message_consumed`; a retired one is left as it is, with no event.
#[test]
fn the_recipient_consumes_an_unread_message_once() {
    let board = MemoryBoard::with(board());
    let service = AcknowledgeMessage::new(board.clone(), SteppingClock::fixed(50.0));
    for (id, changed) in [(1, true), (1, false), (2, false)] {
        assert_eq!(
            service.execute(ack("parent", json!(id))).unwrap(),
            Settled {
                message_id: json!(id),
                changed,
            }
        );
    }
    let state = board.snapshot();
    let statuses: Vec<&str> = state.messages.iter().map(|m| m.status.as_str()).collect();
    assert_eq!(statuses, ["consumed", "superseded", "accepted"]);
    let actions: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), &event.detail))
        .collect();
    assert_eq!(actions, [("message_consumed", &json!({"message": 1}))]);
}

/// Only an integer id of at least 1 is taken, and only a message in the
/// caller's own inbox.
#[test]
fn an_acknowledgment_refuses_what_python_refuses() {
    let board = MemoryBoard::with(board());
    let service = AcknowledgeMessage::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(ack("parent", json!("1"))).unwrap_err(),
        BoardError::new(
            RefusalKind::Invalid,
            "message id must be a positive integer"
        )
    );
    for (actor, id) in [("parent", 3), ("parent", 9), ("worker", 1)] {
        assert_eq!(
            service.execute(ack(actor, json!(id))).unwrap_err(),
            BoardError::new(RefusalKind::NotFound, "unknown message in own inbox")
        );
    }
    assert!(board.snapshot().events.is_empty());
}
