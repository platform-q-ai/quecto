use serde_json::{Value, json};

use super::ClaimNotifications;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, RecordedEvent, SteppingClock, StoredMessage, member_row, running_board,
};
use crate::application::swarm::dto::ClaimNotificationsRequest;
use crate::domain::swarm::{RefusalKind, RunState};

/// `parent`'s unread message 1 to `worker`, recorded as event 1.
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.messages.push(StoredMessage {
        id: 1,
        sender: "parent".to_owned(),
        recipient: json!("worker"),
        body: "b".to_owned(),
        status: "accepted".to_owned(),
        ..StoredMessage::default()
    });
    state.events.push(RecordedEvent {
        actor: "parent".to_owned(),
        time: 1.0,
        action: "message_accepted".to_owned(),
        detail: json!({"message": 1, "recipient": "worker", "revision": null}),
    });
    state
}

fn claim(actor: &str, with_generation: Value) -> ClaimNotificationsRequest {
    ClaimNotificationsRequest {
        actor: actor.to_owned(),
        with_generation,
    }
}

/// The recipient is woken once, as its full member row: the cursor moves
/// to the board's generation before the hint is sent, so a second claim
/// finds nothing and moves nothing.
#[test]
fn a_message_wakes_its_recipient_once() {
    let board = MemoryBoard::with(board());
    let service = ClaimNotifications::new(board.clone(), SteppingClock::fixed(50.0));
    let first = service.execute(claim("parent", json!(true))).unwrap();
    assert_eq!(first.members, [member_row("worker", "live")]);
    assert_eq!((first.generation, first.cursor_moved), (1, true));
    assert!(first.with_generation);
    let second = service.execute(claim("parent", json!(false))).unwrap();
    assert!(second.members.is_empty());
    assert_eq!((second.generation, second.cursor_moved), (1, false));
    assert!(!second.with_generation);
    assert_eq!(
        board.snapshot().notification_cursors,
        [("parent".to_owned(), 1)]
    );
    assert_eq!(board.snapshot().wake_cursors, None, "no wake table");
}

/// Another member's events are not the caller's to hint.
#[test]
fn only_the_callers_own_events_are_claimed() {
    let board = MemoryBoard::with(board());
    let service = ClaimNotifications::new(board.clone(), SteppingClock::fixed(50.0));
    let batch = service.execute(claim("worker", Value::Null)).unwrap();
    assert!(batch.members.is_empty());
    assert!(batch.cursor_moved);
}

/// A run that is not running hints nobody, and the cursor still moves.
#[test]
fn a_paused_run_hints_nobody_and_advances() {
    let mut state = board();
    state.run.as_mut().unwrap().record.status = Some(RunState::PAUSED);
    let board = MemoryBoard::with(state);
    let service = ClaimNotifications::new(board.clone(), SteppingClock::fixed(50.0));
    let batch = service.execute(claim("parent", json!("x"))).unwrap();
    assert!(batch.members.is_empty());
    assert!(batch.with_generation, "Python's truth of 'x'");
    assert_eq!((batch.generation, batch.cursor_moved), (1, true));
}

/// A caller that is no member is refused by the gate, and nothing moves.
#[test]
fn a_stranger_is_refused() {
    let board = MemoryBoard::with(board());
    let service = ClaimNotifications::new(board.clone(), SteppingClock::fixed(50.0));
    let refused = service.execute(claim("stranger", json!([]))).unwrap_err();
    assert_eq!(refused.kind(), RefusalKind::NotMember);
    assert!(board.snapshot().notification_cursors.is_empty());
}
