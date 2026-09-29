use std::sync::Arc;

use serde_json::{Value, json};

use super::SendMessage;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, SteppingClock, StoredMessage, member_row,
    running_board,
};
use crate::application::swarm::dto::SendMessageRequest;
use crate::domain::swarm::{BoardError, RefusalKind};

/// A running run with the live `worker`, the reserved `later` and the dead
/// `gone`.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.members.push(member_row("later", "reserved"));
    state.members.push(member_row("gone", "dead"));
    state
}

fn service(board: &Arc<MemoryBoard>) -> SendMessage {
    SendMessage::new(
        board.clone(),
        SteppingClock::new(&[10.0, 11.0, 12.0, 13.0, 14.0, 15.0, 16.0, 17.0]),
        Arc::new(CompactEncoding),
    )
}

fn send(request: &str, recipient: &str, revision: Value, supersedes: Value) -> SendMessageRequest {
    SendMessageRequest {
        actor: "worker".to_owned(),
        request: json!(request),
        recipient: json!(recipient),
        body: json!("review this"),
        revision,
        supersedes,
    }
}

fn plain(request: &str, recipient: &str) -> SendMessageRequest {
    send(request, recipient, Value::Null, Value::Null)
}

/// A send inserts the message accepted, records `message_accepted`, and
/// answers the receipt the ledger replays for the same request.
#[test]
fn a_send_is_accepted_once_per_request() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    let first = service.execute(plain("m1", "parent")).unwrap();
    assert_eq!(first.receipt, json!({"id": 1, "status": "accepted"}));
    assert!(first.sent);
    let replay = service.execute(plain("m1", "parent")).unwrap();
    assert_eq!((replay.receipt, replay.sent), (first.receipt, false));
    let state = board.snapshot();
    assert_eq!(state.messages.len(), 1);
    assert_eq!(
        state.requests[0].payload,
        json!(["send", "parent", "review this"])
    );
    let events: Vec<(&str, &Value)> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), &event.detail))
        .collect();
    assert_eq!(
        events,
        [(
            "message_accepted",
            &json!({"message": 1, "recipient": "parent", "revision": null})
        )]
    );
}

/// Superseding retires the caller's earlier message in the same
/// transaction, records who superseded it, and keys the ledger by the
/// revision and the superseded id too; each event reads the clock.
#[test]
fn a_superseding_send_retires_the_earlier_message() {
    let board = MemoryBoard::with(board());
    let service = service(&board);
    service
        .execute(send("r1", "parent", json!("abc1"), Value::Null))
        .unwrap();
    let second = service
        .execute(send("r2", "parent", json!("abc2"), json!(1)))
        .unwrap();
    assert_eq!(second.receipt, json!({"id": 2, "status": "accepted"}));
    let state = board.snapshot();
    let first = &state.messages[0];
    assert_eq!(
        (first.status.as_str(), first.superseded_by),
        ("superseded", Some(2))
    );
    assert_eq!(state.messages[1].supersedes, Some(1));
    assert_eq!(
        state.requests[1].payload,
        json!(["send", "parent", "review this", "abc2", 1])
    );
    let actions: Vec<(&str, f64)> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), event.time))
        .collect();
    assert_eq!(
        actions,
        [
            ("message_accepted", 12.0),
            ("message_superseded", 15.0),
            ("message_accepted", 16.0),
        ]
    );
}

/// Arguments are checked before the gate, the recipient must be a live or
/// reserved member, and a full inbox refuses; nothing is written.
#[test]
fn a_send_refuses_what_python_refuses() {
    let mut state = board();
    state.messages = (1..=100)
        .map(|id| StoredMessage {
            id,
            sender: "parent".to_owned(),
            recipient: json!("later"),
            body: "x".to_owned(),
            status: "accepted".to_owned(),
            ..StoredMessage::default()
        })
        .collect();
    let board = MemoryBoard::with(state);
    let service = service(&board);
    let refused = |request: SendMessageRequest| service.execute(request).unwrap_err();
    let mut body = plain("b", "parent");
    body.body = json!(5);
    for (request, kind, text) in [
        (
            body,
            RefusalKind::Invalid,
            "message must be nonempty and at most 8192 bytes",
        ),
        (
            send("r", "parent", json!(""), Value::Null),
            RefusalKind::Invalid,
            "message revision must be nonempty and at most 256 bytes",
        ),
        (
            send("s", "parent", Value::Null, json!(true)),
            RefusalKind::Invalid,
            "supersedes must be a message id",
        ),
        (
            plain("g", "gone"),
            RefusalKind::NotFound,
            "unknown or out-of-swarm recipient",
        ),
        (
            plain("f", "later"),
            RefusalKind::CapacityFull,
            "recipient inbox full (100 unconsumed messages)",
        ),
        (
            send("x", "later", Value::Null, json!(1)),
            RefusalKind::NotOwner,
            "only your own message can be superseded",
        ),
    ] {
        assert_eq!(refused(request), BoardError::new(kind, text));
    }
    let after = board.snapshot();
    assert_eq!(after.messages.len(), 100);
    assert!(after.events.is_empty() && after.requests.is_empty());
}

/// A paused run refuses a send with Python's own text.
#[test]
fn a_send_needs_a_running_run() {
    let mut state = board();
    if let Some(run) = state.run.as_mut() {
        run.record.status = Some(crate::domain::swarm::RunState::PAUSED);
    }
    let board = MemoryBoard::with(state);
    assert_eq!(
        service(&board).execute(plain("m", "parent")).unwrap_err(),
        BoardError::new(
            RefusalKind::NotRunning,
            "run is paused; no new work permitted"
        )
    );
}

/// `unknown_member_status_is_not_alive`, for `send` too: a recipient whose
/// status is unknown or NULL (only a hand edit writes one) is out of the
/// swarm, where Python's `status == 'dead'` check sends to it.
#[test]
fn an_unknown_recipient_status_is_out_of_the_swarm() {
    let mut state = board();
    state.members.push(member_row("odd", "zombie"));
    let mut null = member_row("none", "live");
    null.columns[2].1 = Value::Null;
    state.members.push(null);
    let board = MemoryBoard::with(state);
    for recipient in ["odd", "none"] {
        assert_eq!(
            service(&board)
                .execute(plain(recipient, recipient))
                .unwrap_err(),
            BoardError::new(RefusalKind::NotFound, "unknown or out-of-swarm recipient"),
            "{recipient}"
        );
    }
    assert!(board.snapshot().messages.is_empty());
}
