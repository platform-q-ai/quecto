use super::ReadRunStatus;
use crate::application::swarm::dto::{MemberClaimCounts, StatusDeadline};
use crate::application::swarm::use_cases::fakes::{
    BoardState, MemoryBoard, member_row, running_board,
};

#[test]
fn a_board_without_a_run_reads_as_setup_with_no_deadline() {
    let board = MemoryBoard::with(BoardState::default());
    let status = ReadRunStatus::new(board.clone()).execute().unwrap();
    assert_eq!(status.status, "setup");
    assert_eq!(status.deadline, StatusDeadline::NoRun);
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
    assert_eq!(status.status, "running");
    assert_eq!(status.deadline, StatusDeadline::Stored(250.5));
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
