use std::sync::Arc;

use serde_json::{Value, json};

use super::WithdrawMessage;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, StoredMessage, member_row,
    running_board,
};
use crate::application::swarm::dto::{MessageIdRequest, Settled};
use crate::domain::swarm::{BoardError, RefusalKind, RunState};

/// A paused run (withdrawal is bookkeeping) with message 1 from `worker`
/// to `parent`, unread.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    if let Some(run) = state.run.as_mut() {
        run.record.status = Some(RunState::PAUSED);
    }
    state.members.push(member_row("worker", "live"));
    state.messages.push(StoredMessage {
        id: 1,
        sender: "worker".to_owned(),
        recipient: json!("parent"),
        body: "never mind".to_owned(),
        status: "accepted".to_owned(),
        ..StoredMessage::default()
    });
    state
}

fn withdraw(actor: &str, message_id: Value) -> MessageIdRequest {
    MessageIdRequest {
        actor: actor.to_owned(),
        message_id,
    }
}

/// The sender withdraws its unread message once, recorded as
/// `message_withdrawn`; repeating it is a no-op with no event.
#[test]
fn the_sender_withdraws_its_message_once() {
    let board = MemoryBoard::with(board());
    let service = WithdrawMessage::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        Arc::new(CompactEncoding),
    );
    for changed in [true, false] {
        assert_eq!(
            service.execute(withdraw("worker", json!(1))).unwrap(),
            Settled {
                message_id: json!(1),
                changed,
            }
        );
    }
    let state = board.snapshot();
    assert_eq!(state.messages[0].status, "withdrawn");
    let actions: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), &event.detail))
        .collect();
    assert_eq!(actions, [("message_withdrawn", &json!({"message": 1}))]);
}

/// Only an integer id of at least 1 is taken, and only the sender's own
/// unread message; a consumed one names its status.
#[test]
fn a_withdrawal_refuses_what_python_refuses() {
    let mut state = board();
    state.messages.push(StoredMessage {
        id: 2,
        sender: "worker".to_owned(),
        recipient: json!("parent"),
        status: "consumed".to_owned(),
        ..StoredMessage::default()
    });
    let board = MemoryBoard::with(state);
    let service = WithdrawMessage::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        Arc::new(CompactEncoding),
    );
    for bad in [json!("1"), json!(true), json!(0), json!(null), json!(1.0)] {
        assert_eq!(
            service.execute(withdraw("worker", bad)).unwrap_err(),
            BoardError::new(
                RefusalKind::Invalid,
                "message id must be a positive integer"
            )
        );
    }
    for (actor, id, kind, text) in [
        (
            "parent",
            1,
            RefusalKind::NotOwner,
            "only your own message can be withdrawn",
        ),
        (
            "worker",
            9,
            RefusalKind::NotOwner,
            "only your own message can be withdrawn",
        ),
        (
            "worker",
            2,
            RefusalKind::WrongState,
            "message 2 is already consumed",
        ),
    ] {
        assert_eq!(
            service.execute(withdraw(actor, json!(id))).unwrap_err(),
            BoardError::new(kind, text)
        );
    }
    assert!(board.snapshot().events.is_empty());
}
