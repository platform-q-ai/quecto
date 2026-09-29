use super::ReadRunSnapshot;
use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, SteppingClock, member_row, running_board,
};
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
    assert_eq!(board.transactions(), [false, false], "the two-step gate");
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
