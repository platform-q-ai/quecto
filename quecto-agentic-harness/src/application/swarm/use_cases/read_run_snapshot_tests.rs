use super::ReadRunSnapshot;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board,
};
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

#[test]
fn snapshot_reads_the_run_and_every_member_through_the_gate() {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "dead"));
    let board = MemoryBoard::with(state);
    // A dead member still reads.
    let snapshot = ReadRunSnapshot::new(board.clone(), SteppingClock::fixed(50.0))
        .execute("worker")
        .unwrap();
    assert_eq!(snapshot.status.as_deref(), Some("running"));
    assert_eq!(snapshot.coordinator.as_deref(), Some("parent"));
    assert_eq!(snapshot.outcome, None);
    assert_eq!(snapshot.control_generation, 0);
    assert_eq!(snapshot.deadline, 100.0);
    assert_eq!(
        snapshot.members,
        [member_row("parent", "live"), member_row("worker", "dead")]
    );
    assert_eq!(board.reads(), 1, "#2338: one read transaction");
    assert!(
        board.transactions().is_empty(),
        "the gate had nothing to write: no write transaction"
    );
}

#[test]
fn snapshot_after_the_deadline_sees_the_committed_pause() {
    let board = MemoryBoard::with(running_board(100.0));
    let snapshot = ReadRunSnapshot::new(board.clone(), SteppingClock::fixed(100.0))
        .execute("parent")
        .unwrap();
    assert_eq!(snapshot.status.as_deref(), Some("paused"));
    assert_eq!(snapshot.outcome.as_deref(), Some("budget-exhausted"));
    // `stop` is event 1, `paused` event 2: the control generation.
    assert_eq!(snapshot.control_generation, 2);
}

#[test]
fn snapshot_refuses_an_unknown_member_and_a_missing_run() {
    let board = MemoryBoard::with(running_board(100.0));
    let refused = ReadRunSnapshot::new(board, SteppingClock::fixed(1.0))
        .execute("stranger")
        .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    let empty = MemoryBoard::with(BoardState::default());
    let refused = ReadRunSnapshot::new(empty, SteppingClock::fixed(1.0))
        .execute("parent")
        .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(RefusalKind::RunMissing, "coordination run missing")
    );
}

/// Served over another repository (#2303 round-3 review M1), the use case
/// reads that board with the clock it was composed with: at the deadline,
/// the run is paused.
#[test]
fn over_reads_the_given_board_on_the_composed_clock() {
    let composed_over = MemoryBoard::with(BoardState::default());
    let other = MemoryBoard::with(running_board(100.0));
    let snapshot = ReadRunSnapshot::new(composed_over.clone(), SteppingClock::fixed(100.0))
        .over(other.clone())
        .execute("parent")
        .unwrap();
    assert_eq!(snapshot.status.as_deref(), Some("paused"));
    // #2338: the read found the deadline come, so the two-step gate ran.
    assert_eq!(other.reads(), 1);
    assert_eq!(other.transactions(), [false, false]);
    assert!(composed_over.transactions().is_empty());
}

/// #2338: the watch's tick answers only the cursor while it is the one
/// passed, and the snapshot with it otherwise, in one read transaction.
#[test]
fn a_watch_answers_the_snapshot_only_when_the_cursor_moved() {
    let board = MemoryBoard::with(running_board(100.0));
    let snapshot = ReadRunSnapshot::new(board.clone(), SteppingClock::fixed(50.0));
    let first = snapshot.watch("parent", None).unwrap();
    assert_eq!(first.event_cursor, 0, "an empty events table");
    assert_eq!(
        first
            .snapshot
            .as_ref()
            .and_then(|view| view.status.as_deref()),
        Some("running"),
        "no cursor passed: the snapshot"
    );
    let unchanged = snapshot.watch("parent", Some(0)).unwrap();
    assert_eq!(unchanged.event_cursor, 0);
    assert_eq!(unchanged.snapshot, None, "the cursor passed is the board's");
    let moved = snapshot.watch("parent", Some(5)).unwrap();
    assert!(moved.snapshot.is_some(), "another cursor: the snapshot");
    assert_eq!(board.reads(), 3, "one read transaction a tick");
    assert!(board.transactions().is_empty());
    let refused = snapshot.watch("stranger", Some(0)).unwrap_err();
    assert_eq!(
        refused.kind(),
        RefusalKind::NotMember,
        "the gate still reads"
    );
}

/// At the deadline the tick records the expiry through the gate, and
/// answers the paused run whatever cursor it was given.
#[test]
fn a_watch_at_the_deadline_records_the_expiry() {
    let board = MemoryBoard::with(running_board(100.0));
    let watched = ReadRunSnapshot::new(board.clone(), SteppingClock::fixed(100.0))
        .watch("parent", Some(0))
        .unwrap();
    assert_eq!(watched.event_cursor, 2, "the stop and paused events");
    let view = watched.snapshot.expect("the cursor moved: the snapshot");
    assert_eq!(view.status.as_deref(), Some("paused"));
    assert_eq!(board.transactions(), [false, false]);
}
