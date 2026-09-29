use serde_json::json;

use super::ResumeRunExternally;
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, paused, running_board, usage,
};
use crate::application::swarm::dto::RunTransition;
use crate::domain::swarm::{BoardError, RunState};

/// `resume_refuses_members_and_lists_blockers_for_the_supervisor` (the
/// supervisor half): a pause whose resume would end again at once is
/// refused listing why; otherwise the deadline moves on by the paused
/// interval, the held outcome clears and `resumed` names both.
#[test]
fn the_supervisor_resumes_unless_a_blocker_remains() {
    let mut state = paused(running_board(100.0), 40.0, Some(("blocked", "why")));
    state.usage = Some(usage(
        json!({"token_limit": 10, "strict_unknown": true, "warned": false}),
        0,
        1,
    ));
    let board = MemoryBoard::with(state);
    let service = ResumeRunExternally::new(board.clone(), SteppingClock::fixed(70.0));
    assert_eq!(
        service.execute("parent").unwrap_err(),
        BoardError::new(
            "resume would pause again at once: raise or disable the token budget \
             (swarm_control usage_budget) before resuming"
        )
    );
    let mut state = board.snapshot();
    state.usage = None;
    let board = MemoryBoard::with(state);
    let service = ResumeRunExternally::new(board.clone(), SteppingClock::fixed(70.0));
    let answer = service.execute("parent").unwrap();
    assert_eq!(answer.transition, RunTransition::Applied);
    assert_eq!(
        (answer.receipt.status.as_deref(), answer.receipt.outcome),
        (Some("running"), None)
    );
    let state = board.snapshot();
    let run = state.run.unwrap().record;
    assert_eq!((run.deadline, run.outcome_reason), (130.0, None));
    let resumed = state.events.last().unwrap();
    assert_eq!(
        (resumed.action.as_str(), &resumed.detail),
        (
            "resumed",
            &json!({"paused_seconds": 30.0, "outcome": "blocked"})
        )
    );
    assert_eq!(answer.receipt.generation, 2);
    let again = service.execute("parent").unwrap();
    assert_eq!(again.transition, RunTransition::Unchanged);
    assert_eq!(
        board.snapshot().events.len(),
        2,
        "a running run stays as it is"
    );
}

/// Resumed at the instant it paused, the interval is Python's integer 0;
/// only a paused run resumes, and only the coordinator resumes it.
#[test]
fn only_a_paused_run_resumes_and_an_instant_pause_is_zero_seconds() {
    let mut state = paused(running_board(100.0), 40.0, None);
    state.members.push(member_row("worker", "live"));
    let board = MemoryBoard::with(state);
    let service = ResumeRunExternally::new(board.clone(), SteppingClock::fixed(40.0));
    assert_eq!(
        service.execute("worker").unwrap_err(),
        BoardError::new("only the designated coordinator may do this")
    );
    service.execute("parent").unwrap();
    assert_eq!(
        board.snapshot().events.last().unwrap().detail,
        json!({"paused_seconds": 0, "outcome": null})
    );
    let mut closed = running_board(100.0);
    closed.run.as_mut().unwrap().record.status = Some(RunState::SUCCEEDED);
    let service = ResumeRunExternally::new(MemoryBoard::with(closed), SteppingClock::fixed(40.0));
    assert_eq!(
        service.execute("parent").unwrap_err(),
        BoardError::new("only a paused run may resume")
    );
}
