use serde_json::{Value, json};

use super::ClaimTask;
use crate::application::swarm::board_test_support::{
    BoardState, CounterIds, MemoryBoard, SteppingClock, member_row, running_board, stored_task,
};
use crate::application::swarm::dto::ClaimTaskRequest;
use crate::application::swarm::use_cases::OverRepository;
use crate::domain::swarm::{BoardError, RefusalKind};

/// Tasks 1 (ready), 2 (ready, on 1), 3 (claimed by the parent).
fn board() -> BoardState {
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "live"));
    state.tasks = vec![
        stored_task(1, "ready", json!([]), None),
        stored_task(2, "ready", json!([1]), None),
        stored_task(3, "claimed", json!([]), Some("parent")),
    ];
    state
}

fn claim(task_id: Value) -> ClaimTaskRequest {
    ClaimTaskRequest {
        actor: "worker".to_owned(),
        task_id,
    }
}

/// `test_claims_are_atomic_and_dependencies_block_claims`: a dependent
/// task is refused while its dependency is incomplete, a claimed task is
/// not ready, and a claim draws its token only once every check passed:
/// one draw per successful claim.
#[test]
fn claims_check_dependencies_and_readiness_before_drawing_a_token() {
    let board = MemoryBoard::with(board());
    let service = ClaimTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        CounterIds::journalling(&board.journal),
    );
    for (task_id, kind, refusal) in [
        (json!(2), RefusalKind::WrongState, "unmet dependencies"),
        (
            json!(3),
            RefusalKind::WrongState,
            "task is not ready to claim",
        ),
        (json!(7), RefusalKind::NotFound, "unknown task"),
    ] {
        assert_eq!(
            service.execute(claim(task_id.clone())).unwrap_err(),
            BoardError::new(kind, refusal),
            "{task_id}"
        );
    }
    assert!(
        board.journal().is_empty(),
        "no token drawn: {:?}",
        board.journal()
    );
    let claimed = service.execute(claim(json!("1"))).unwrap();
    let token = format!("{:032x}", 1);
    assert_eq!(claimed.text("status"), Some("claimed"));
    assert_eq!(claimed.text("owner"), Some("worker"));
    assert_eq!(claimed.text("token"), Some(token.as_str()));
    assert_eq!(
        board.journal(),
        [
            format!("draw {token}"),
            "update_task_claim \"1\" worker".to_owned(),
            "event claimed".to_owned(),
        ]
    );
    let state = board.snapshot();
    assert_eq!(
        state.events[0].detail,
        json!({"task": "1", "token": token}),
        "the task id as the caller gave it"
    );
    assert_eq!(
        service.execute(claim(json!(1))).unwrap_err(),
        BoardError::new(RefusalKind::WrongState, "task is not ready to claim")
    );
}

/// Only a running run takes new claims.
#[test]
fn a_paused_run_takes_no_claim() {
    let mut state = board();
    let run = state.run.as_mut().unwrap();
    run.record.status = Some(crate::domain::swarm::RunState::PAUSED);
    let board = MemoryBoard::with(state);
    let service = ClaimTask::new(
        board.clone(),
        SteppingClock::fixed(50.0),
        CounterIds::journalling(&board.journal),
    );
    assert_eq!(
        service.execute(claim(json!(1))).unwrap_err(),
        BoardError::new(
            RefusalKind::NotRunning,
            "run is paused; no new work permitted"
        )
    );
}

/// Served over another repository (#2303 reconcile), the claim is made on
/// that board alone, its token drawn from the source it was composed with.
#[test]
fn over_claims_on_the_given_board_with_the_composed_ports() {
    let composed_over = MemoryBoard::with(board());
    let other = MemoryBoard::with(board());
    let claimed = ClaimTask::new(
        composed_over.clone(),
        SteppingClock::fixed(50.0),
        CounterIds::journalling(&composed_over.journal),
    )
    .over(other.clone())
    .execute(claim(json!(1)))
    .unwrap();
    assert_eq!(claimed.text("owner"), Some("worker"));
    assert_eq!(other.snapshot().tasks[0].text("status"), Some("claimed"));
    assert!(composed_over.transactions().is_empty());
    assert!(
        composed_over
            .journal()
            .iter()
            .any(|entry| entry.starts_with("draw ")),
        "the composed id source drew the token"
    );
}
