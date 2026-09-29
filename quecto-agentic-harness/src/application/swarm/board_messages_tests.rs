use serde_json::{Value, json};

use super::{Retirement, is_message_id, retire};
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, StoredMessage, running_board,
};
use crate::domain::swarm::{BoardError, RefusalKind};

/// Python's `type(value) is int and value >= 1`.
#[test]
fn a_message_id_is_an_integer_of_at_least_one() {
    for (value, expected) in [
        (json!(1), true),
        (json!(u64::MAX), true),
        (json!(0), false),
        (json!(-1), false),
        (json!(1.0), false),
        (json!(true), false),
        (json!("1"), false),
        (json!(null), false),
        (json!([1]), false),
    ] {
        assert_eq!(is_message_id(&value), expected, "{value}");
    }
}

fn stored(id: i64, status: &str) -> StoredMessage {
    StoredMessage {
        id,
        sender: "worker".to_owned(),
        recipient: json!("parent"),
        status: status.to_owned(),
        ..StoredMessage::default()
    }
}

fn retired(
    state: BoardState,
    id: i64,
    recipient: Option<Value>,
    retirement: Retirement,
) -> Result<bool, BoardError> {
    let board = MemoryBoard::with(state);
    atomic(&*board, false, |transaction| {
        retire(
            transaction,
            &CompactEncoding,
            "worker",
            &json!(id),
            recipient.as_ref(),
            retirement,
        )
    })
}

/// `_retire`: the sender's own unread message is retired; a withdrawn one
/// is left by a withdrawal and named by a supersession; the recipient must
/// match by Python's `==` (`'5' != 5`).
#[test]
fn only_the_senders_unread_message_is_retired() {
    let mut state = running_board(100.0);
    state.messages = vec![stored(1, "accepted"), stored(2, "withdrawn")];
    state.messages.push(StoredMessage {
        recipient: json!("5"),
        ..stored(3, "accepted")
    });
    let superseded = Retirement::Superseded;
    assert_eq!(
        retired(state.clone(), 1, Some(json!("parent")), superseded),
        Ok(true)
    );
    assert_eq!(
        retired(state.clone(), 2, None, Retirement::Withdrawn),
        Ok(false)
    );
    for (id, recipient, kind, text) in [
        (
            2,
            json!("parent"),
            RefusalKind::WrongState,
            "message 2 is already withdrawn",
        ),
        (
            3,
            json!(5),
            RefusalKind::WrongState,
            "only a message to the same recipient can be superseded",
        ),
        (
            7,
            json!("parent"),
            RefusalKind::NotOwner,
            "only your own message can be superseded",
        ),
    ] {
        assert_eq!(
            retired(state.clone(), id, Some(recipient), superseded),
            Err(BoardError::new(kind, text))
        );
    }
}
