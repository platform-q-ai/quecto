use serde_json::{Value, json};

use super::AcceptWake;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, RecordedEvent, SteppingClock, StoredMessage, member_row, running_board,
};
use crate::application::swarm::dto::{AcceptWakeRequest, WakeAccepted};
use crate::domain::swarm::{RefusalKind, RunState};

/// `worker`'s unread message 1 to `parent` (event 1), then `parent`'s own
/// amendment (event 2).
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.messages.push(StoredMessage {
        id: 1,
        sender: "worker".to_owned(),
        recipient: json!("parent"),
        body: "b".to_owned(),
        status: "accepted".to_owned(),
        ..StoredMessage::default()
    });
    for (actor, action, detail) in [
        (
            "worker",
            "message_accepted",
            json!({"message": 1, "recipient": "parent", "revision": null}),
        ),
        ("parent", "amended", json!({"reason": "r"})),
    ] {
        state.events.push(RecordedEvent {
            actor: actor.to_owned(),
            time: 1.0,
            action: action.to_owned(),
            detail,
        });
    }
    state
}

fn accept(actor: &str, generation: Value) -> AcceptWakeRequest {
    AcceptWakeRequest {
        actor: actor.to_owned(),
        generation,
    }
}

const fn answered(woken: bool, cursor_moved: bool) -> WakeAccepted {
    WakeAccepted {
        woken,
        cursor_moved,
    }
}

/// A generation wakes once: the claim moves the wake cursor to it, so the
/// same generation, or an earlier one, claims nothing again.
#[test]
fn a_generation_wakes_once() {
    let board = MemoryBoard::with(board());
    let service = AcceptWake::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(accept("parent", json!(1))).unwrap(),
        answered(true, true)
    );
    for generation in [1, 0] {
        assert_eq!(
            service
                .execute(accept("parent", json!(generation)))
                .unwrap(),
            answered(false, false)
        );
    }
    assert_eq!(
        service.execute(accept("parent", json!(2))).unwrap(),
        answered(false, true),
        "the caller's own amendment does not wake it"
    );
    assert_eq!(
        board.snapshot().wake_cursors,
        Some(vec![("parent".to_owned(), 2)])
    );
}

/// The generation is Python's nonnegative `int`, checked before the gate
/// opens any transaction.
#[test]
fn the_generation_is_checked_before_the_gate() {
    let board = MemoryBoard::with(board());
    let service = AcceptWake::new(board.clone(), SteppingClock::fixed(50.0));
    for generation in [json!(true), json!(1.0), json!(-1), json!("1"), Value::Null] {
        let refused = service.execute(accept("stranger", generation)).unwrap_err();
        assert_eq!(refused.kind(), RefusalKind::Invalid);
        assert_eq!(
            refused.message(),
            "wake generation must be a nonnegative integer"
        );
    }
    assert!(board.transactions().is_empty());
}

/// A generation ahead of the board is refused, and the refused claim's
/// table goes with its transaction.
#[test]
fn a_generation_ahead_of_the_board_is_refused() {
    let board = MemoryBoard::with(board());
    let service = AcceptWake::new(board.clone(), SteppingClock::fixed(50.0));
    for generation in [json!(3), json!(u64::MAX)] {
        let refused = service.execute(accept("parent", generation)).unwrap_err();
        assert_eq!(
            (refused.kind(), refused.message()),
            (
                RefusalKind::Invalid,
                "wake generation is ahead of the board"
            )
        );
    }
    assert_eq!(board.snapshot().wake_cursors, None);
}

/// A paused run claims nothing and keeps the frontier (the table is
/// created, the cursor is not written), so the generation wakes after a
/// resume.
#[test]
fn a_paused_run_keeps_the_frontier() {
    let mut state = board();
    state.run.as_mut().unwrap().record.status = Some(RunState::PAUSED);
    let board = MemoryBoard::with(state);
    let service = AcceptWake::new(board.clone(), SteppingClock::fixed(50.0));
    assert_eq!(
        service.execute(accept("parent", json!(1))).unwrap(),
        answered(false, false)
    );
    assert_eq!(board.snapshot().wake_cursors, Some(Vec::new()));
    board
        .state
        .lock()
        .unwrap()
        .run
        .as_mut()
        .unwrap()
        .record
        .status = Some(RunState::RUNNING);
    assert_eq!(
        service.execute(accept("parent", json!(1))).unwrap(),
        answered(true, true)
    );
}
