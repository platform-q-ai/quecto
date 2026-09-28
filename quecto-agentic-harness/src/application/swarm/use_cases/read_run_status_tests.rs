use super::ReadRunStatus;
use serde_json::Value;

use crate::application::swarm::board_test_support::{
    BoardState, MemoryBoard, member_row, running_board,
};
use crate::application::swarm::dto::MemberClaimCounts;

#[test]
fn a_board_without_a_run_reads_as_setup_with_no_deadline() {
    let board = MemoryBoard::with(BoardState::default());
    let status = ReadRunStatus::new(board.clone()).execute().unwrap();
    assert_eq!(status.status.as_deref(), Some("setup"));
    // Python's integer 0, never a float.
    assert_eq!(serde_json::to_string(&status.deadline).unwrap(), "0");
    assert_eq!(
        (status.id, status.coordinator, status.outcome),
        (None, None, None)
    );
    assert_eq!(
        status.counts,
        MemberClaimCounts {
            members_without_claim: 0,
            members_dead: 0
        }
    );
    assert_eq!(
        board.transactions(),
        [false],
        "status never creates a board"
    );
}

#[test]
fn status_reads_the_run_and_the_membership_counts_without_membership() {
    let mut state = running_board(250.5);
    state.members.push(member_row("worker", "reserved"));
    state.members.push(member_row("gone", "dead"));
    let run = &mut state.run.as_mut().unwrap().record;
    run.outcome = Some("failed".to_owned());
    let board = MemoryBoard::with(state);
    let status = ReadRunStatus::new(board.clone()).execute().unwrap();
    assert_eq!(status.id.as_deref(), Some("run-1"));
    assert_eq!(status.status.as_deref(), Some("running"));
    assert_eq!(status.deadline, Value::from(250.5));
    assert_eq!(status.coordinator.as_deref(), Some("parent"));
    assert_eq!(status.outcome.as_deref(), Some("failed"));
    // The coordinator is excluded from the unclaimed count.
    assert_eq!(
        status.counts,
        MemberClaimCounts {
            members_without_claim: 1,
            members_dead: 1
        }
    );
    assert!(board.snapshot().events.is_empty(), "a read writes nothing");
}

/// A run row edited outside the board (#2270 review L5): no coordinator
/// and no status read as `None`, and every admitted member is unclaimed.
#[test]
fn status_reads_a_missing_coordinator_and_status_as_none() {
    let mut state = running_board(250.5);
    state.run.as_mut().unwrap().record.coordinator = None;
    let board = MemoryBoard::with(state);
    let status = ReadRunStatus::new(board).execute().unwrap();
    assert_eq!(status.coordinator, None);
    assert_eq!(status.counts.members_without_claim, 1, "parent is counted");
}
