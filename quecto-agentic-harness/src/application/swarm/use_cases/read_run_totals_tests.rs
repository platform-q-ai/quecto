//! #2313 review M1: the run's totals, read from the board through the
//! read-only gate: the tasks by state, the messages by what became of
//! them, each member's usage, and the run's creation time.
use serde_json::json;

use super::ReadRunTotals;
use crate::application::swarm::board_test_support::{
    MemoryBoard, RecordedEvent, SteppingClock, StoredMessage, member_row, recorded, running_board,
    stored_task, usage,
};
use crate::application::swarm::dto::UsageRow;
use crate::domain::swarm::{BoardError, RefusalKind};

fn message(id: i64, status: &str) -> StoredMessage {
    StoredMessage {
        id,
        sender: "worker".to_owned(),
        recipient: json!("parent"),
        body: "hello".to_owned(),
        status: status.to_owned(),
        ..StoredMessage::default()
    }
}

/// A member reads every member's totals as the board holds them.
#[test]
fn the_totals_are_the_boards_for_every_member() {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "submitted", json!([]), Some("worker")),
        stored_task(2, "ready", json!([]), None),
        stored_task(3, "ready", json!([1]), None),
        stored_task(4, "completed", json!([]), None),
    ];
    state.messages = vec![
        message(1, "accepted"),
        message(2, "consumed"),
        message(3, "withdrawn"),
    ];
    let at = |time, action| RecordedEvent {
        time,
        ..recorded("parent", action, json!({}))
    };
    state.events = vec![
        at(3.0, "container_setup"),
        at(12.5, "created"),
        at(20.0, "submitted"),
    ];
    let mut report = usage(json!({}), 0, 0);
    report.members = vec![UsageRow {
        columns: vec![
            ("member".to_owned(), json!("worker")),
            ("requests".to_owned(), json!(2)),
            ("reported_input_tokens".to_owned(), json!(40)),
        ],
    }];
    state.usage = Some(report.clone());
    let board = MemoryBoard::with(state);
    let totals = ReadRunTotals::new(board, SteppingClock::fixed(50.0))
        .execute("parent")
        .unwrap();
    assert_eq!(totals.run_id.as_deref(), Some("run-1"));
    assert_eq!(totals.task_count, 4);
    assert_eq!(
        (
            totals.counts.ready,
            totals.counts.blocked,
            totals.counts.submitted,
            totals.counts.completed
        ),
        (1, 1, 1, 1),
        "a ready task waiting on a dependency counts as blocked"
    );
    assert_eq!(
        (
            totals.messages.sent,
            totals.messages.consumed,
            totals.messages.withdrawn
        ),
        (3, 1, 1)
    );
    assert_eq!(totals.usage, report.members);
    assert_eq!(totals.created_at, Some(12.5), "the created event's time");
    assert_eq!(totals.read_at, 50.0);
}

/// Only a member reads them: the gate refuses anyone else first.
#[test]
fn a_stranger_is_refused_by_the_gate() {
    let board = MemoryBoard::with(running_board(100.0));
    assert_eq!(
        ReadRunTotals::new(board, SteppingClock::fixed(50.0))
            .execute("stranger")
            .unwrap_err(),
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
}

/// A run with no `created` event has no creation time.
#[test]
fn a_run_without_a_created_event_has_no_creation_time() {
    let board = MemoryBoard::with(running_board(100.0));
    let totals = ReadRunTotals::new(board, SteppingClock::fixed(50.0))
        .execute("parent")
        .unwrap();
    assert_eq!(totals.created_at, None);
    assert_eq!(totals.task_count, 0);
}
