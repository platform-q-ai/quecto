use serde_json::{Value, json};

use super::{Notice, notify_revoked, reopen};
use crate::application::swarm::board_operation::atomic;
use crate::application::swarm::board_test_support::{
    BoardState, CompactEncoding, MemoryBoard, StoredFile, StoredMessage, member_row, running_board,
    stored_task,
};
use crate::application::swarm::ports::Clock;

/// A clock that always reads 7.
struct FixedClock;

impl Clock for FixedClock {
    fn now_seconds(&self) -> f64 {
        7.0
    }
}

fn notice<'a>(previous: &'a Value, task_id: &'a Value) -> Notice<'a> {
    Notice {
        actor: "parent",
        clock: &FixedClock,
        previous,
        task_id,
        reason: "silent",
    }
}

fn message(id: i64) -> StoredMessage {
    StoredMessage {
        id,
        sender: "other".to_owned(),
        recipient: json!("worker"),
        body: "noise".to_owned(),
        status: "accepted".to_owned(),
    }
}

/// `_reopen`: every reservation of the task goes, whichever claim made
/// it, and the task is ready without owner, token, blocker or evidence.
#[test]
fn reopen_frees_every_reservation_of_the_task() {
    let mut state = running_board(100.0);
    state.tasks = vec![stored_task(1, "blocked", json!([]), Some("worker"))];
    state.files = [("a", 1, "c1"), ("b", 1, "c2"), ("c", 2, "c1")]
        .map(|(path, task, claim)| StoredFile {
            path: path.to_owned(),
            task,
            owner: "worker".to_owned(),
            claim: claim.to_owned(),
            token: "t".to_owned(),
        })
        .to_vec();
    let board = MemoryBoard::with(state);
    atomic(&*board, false, |transaction| reopen(transaction, &json!(1))).unwrap();
    let state = board.snapshot();
    let paths: Vec<&str> = state.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, ["c"]);
    assert_eq!(state.tasks[0].text("status"), Some("ready"));
    assert_eq!(state.tasks[0].get("owner"), Some(&Value::Null));
}

/// `_notify_revoked` is best effort: a dead or unknown owner, or an inbox
/// holding a hundred unread messages, gets nothing; otherwise one
/// accepted message and its `message_accepted` event.
#[test]
fn the_notice_is_skipped_for_a_dead_owner_or_a_full_inbox() {
    let full: Vec<StoredMessage> = (1..=100).map(message).collect();
    let mut read = full.clone();
    read[0].status = "consumed".to_owned();
    for (status, messages, sent) in [
        ("dead", Vec::new(), false),
        ("live", full, false),
        ("live", read, true),
        ("reserved", Vec::new(), true),
    ] {
        let mut state: BoardState = running_board(100.0);
        state.members.push(member_row("worker", status));
        state.messages = messages;
        let before = state.messages.len();
        let board = MemoryBoard::with(state);
        let told = atomic(&*board, false, |transaction| {
            notify_revoked(
                transaction,
                &CompactEncoding,
                &notice(&json!("worker"), &json!(4)),
            )
        })
        .unwrap();
        assert_eq!(told, sent, "{status}");
        let state = board.snapshot();
        assert_eq!(state.messages.len(), before + usize::from(sent));
        assert_eq!(state.events.len(), usize::from(sent));
    }
    let board = MemoryBoard::with(running_board(100.0));
    let told = atomic(&*board, false, |transaction| {
        notify_revoked(
            transaction,
            &CompactEncoding,
            &notice(&json!("nobody"), &json!(4)),
        )
    })
    .unwrap();
    assert!(!told, "an unknown owner is not told");
}

/// `unknown_member_status_is_not_alive` (#2295, #2275): an owner whose
/// status the board never writes (NULL, or unknown) is not told, where
/// Python messages any owner that is not `dead`.
#[test]
fn an_unknown_owner_status_is_not_told() {
    for status in [Value::Null, json!("idle")] {
        let mut state = running_board(100.0);
        let mut row = member_row("worker", "live");
        row.columns[2].1 = status.clone();
        state.members.push(row);
        let board = MemoryBoard::with(state);
        let told = atomic(&*board, false, |transaction| {
            notify_revoked(
                transaction,
                &CompactEncoding,
                &notice(&json!("worker"), &json!(4)),
            )
        })
        .unwrap();
        assert!(!told, "{status}");
        assert!(board.snapshot().messages.is_empty());
    }
}
