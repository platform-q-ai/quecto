use serde_json::json;

use super::{atomic, end, operation};
use crate::application::swarm::board_test_support::{
    MemoryBoard, SteppingClock, member_row, running_board,
};
use crate::application::swarm::ports::BoardRepository;
use crate::domain::swarm::{Access, BoardError, RefusalKind, RunState};

const PAUSED_BY_DEADLINE: &str =
    "run is paused (budget-exhausted: deadline); no new work permitted";

fn mutation() -> Access {
    Access {
        active: true,
        coordinator: true,
        read_only: false,
    }
}

/// Port of `test_deadline_transition_commits_before_rejected_mutation`:
/// the first transaction commits the pause, the second refuses the work.
#[test]
fn deadline_transition_commits_before_rejected_mutation() {
    let board = MemoryBoard::with(running_board(100.0));
    let clock = SteppingClock::fixed(100.0);
    let ran = std::cell::Cell::new(false);

    let refused = operation(&*board, &*clock, "parent", mutation(), |_, _| {
        ran.set(true);
        Ok(())
    })
    .unwrap_err();

    assert_eq!(
        refused,
        BoardError::new(RefusalKind::BudgetExhausted, PAUSED_BY_DEADLINE)
    );
    assert!(!ran.get(), "a refused operation never runs its work");
    let state = board.snapshot();
    let run = state.run.unwrap().record;
    assert_eq!(run.status, Some(RunState::PAUSED));
    assert_eq!(run.outcome.as_deref(), Some("budget-exhausted"));
    assert_eq!(run.outcome_reason.as_deref(), Some("deadline"));
    let events: Vec<_> = state
        .events
        .iter()
        .map(|event| (event.action.as_str(), event.detail.clone()))
        .collect();
    assert_eq!(
        events,
        [
            (
                "stop",
                json!({"status": "budget-exhausted", "reason": "deadline"})
            ),
            (
                "paused",
                json!({"reason": "deadline", "started": 100.0, "outcome": "budget-exhausted"})
            ),
        ]
    );
    assert_eq!(board.transactions(), [false, false]);
}

#[test]
fn operation_authorises_twice_and_requires_budget_only_when_active() {
    // The deadline comes between the two transactions: the first sees the
    // run running and in budget, the second finds it spent.
    let active = MemoryBoard::with(running_board(100.0));
    let refused = operation(
        &*active,
        &*SteppingClock::new(&[99.0, 100.0]),
        "parent",
        mutation(),
        |_, _| Ok(()),
    )
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(RefusalKind::BudgetExhausted, PAUSED_BY_DEADLINE)
    );
    let state = active.snapshot();
    assert_eq!(state.run.unwrap().record.status, Some(RunState::RUNNING));
    assert!(state.events.is_empty(), "require_budget writes nothing");

    let inactive = MemoryBoard::with(running_board(100.0));
    let reading = Access {
        read_only: true,
        ..Access::default()
    };
    let run = operation(
        &*inactive,
        &*SteppingClock::new(&[99.0, 100.0]),
        "parent",
        reading,
        |_, run| Ok(run.clone()),
    )
    .unwrap();
    assert_eq!(
        run.status,
        Some(RunState::RUNNING),
        "no budget check when inactive"
    );
    assert_eq!(inactive.transactions(), [false, false]);

    // A dead member passes the first, read-only authorisation (the expiry
    // it finds is committed) and is refused by the second.
    let mut state = running_board(100.0);
    state.members.push(member_row("worker", "dead"));
    let dead = MemoryBoard::with(state);
    let refused = operation(
        &*dead,
        &*SteppingClock::fixed(100.0),
        "worker",
        Access::default(),
        |_, _| Ok(()),
    )
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(
            RefusalKind::NotMember,
            "invoking member is unknown or death confirmed"
        )
    );
    let state = dead.snapshot();
    assert_eq!(state.run.unwrap().record.status, Some(RunState::PAUSED));
    assert_eq!(state.events.len(), 2);
    assert_eq!(dead.transactions(), [false, false]);
}

#[test]
fn a_missing_run_is_refused_by_the_first_authorisation() {
    let board = MemoryBoard::with(Default::default());
    let refused = operation(
        &*board,
        &*SteppingClock::fixed(1.0),
        "parent",
        Access::default(),
        |_, _| Ok(()),
    )
    .unwrap_err();
    assert_eq!(
        refused,
        BoardError::new(RefusalKind::RunMissing, "coordination run missing")
    );
    assert_eq!(board.transactions(), [false]);
}

#[test]
fn an_unexpired_run_is_left_running_and_the_work_sees_it() {
    let board = MemoryBoard::with(running_board(100.0));
    let status = operation(
        &*board,
        &*SteppingClock::fixed(99.5),
        "parent",
        mutation(),
        |_, run| Ok(run.status.clone()),
    )
    .unwrap();
    assert_eq!(status, Some(RunState::RUNNING));
    assert!(board.snapshot().events.is_empty());
}

#[test]
fn end_records_stop_then_paused_reading_the_clock_in_python_order() {
    let board = MemoryBoard::with(running_board(100.0));
    let clock = SteppingClock::new(&[10.0, 20.0, 30.0]);
    atomic(&*board, false, |transaction| {
        end(transaction, &*clock, "parent", "failed", "lost")
    })
    .unwrap();
    let state = board.snapshot();
    let events: Vec<_> = state
        .events
        .iter()
        .map(|event| {
            (
                event.actor.as_str(),
                event.time,
                event.action.as_str(),
                event.detail.clone(),
            )
        })
        .collect();
    assert_eq!(
        events,
        [
            (
                "parent",
                10.0,
                "stop",
                json!({"status": "failed", "reason": "lost"})
            ),
            (
                "parent",
                30.0,
                "paused",
                json!({"reason": "lost", "started": 20.0, "outcome": "failed"})
            ),
        ]
    );
}

#[test]
fn a_refused_transaction_leaves_nothing_behind() {
    let board = MemoryBoard::with(running_board(100.0));
    let clock = SteppingClock::fixed(1.0);
    let refused: Result<(), _> = atomic(&*board, false, |transaction| {
        end(transaction, &*clock, "parent", "failed", "lost")?;
        Err(BoardError::new(RefusalKind::Invalid, "refused"))
    });
    assert_eq!(
        refused.unwrap_err(),
        BoardError::new(RefusalKind::Invalid, "refused")
    );
    let state = board.snapshot();
    assert_eq!(state.run.unwrap().record.status, Some(RunState::RUNNING));
    assert!(state.events.is_empty());
}

/// A repository that commits without running the work is caught.
#[test]
fn atomic_refuses_a_store_that_skipped_the_work() {
    struct Skipping;
    impl BoardRepository for Skipping {
        fn atomic(
            &self,
            _create: bool,
            _work: &mut crate::application::swarm::ports::BoardWork<'_>,
        ) -> Result<(), BoardError> {
            Ok(())
        }
    }
    let skipped = atomic(&Skipping, false, |_| Ok(1));
    assert_eq!(
        skipped.unwrap_err(),
        BoardError::new(
            RefusalKind::Internal,
            "coordination store committed without running its work"
        )
    );
}

/// A repository that runs the work twice is refused the second time.
#[test]
fn atomic_runs_the_work_at_most_once() {
    let board = MemoryBoard::with(running_board(100.0));
    struct Twice(std::sync::Arc<MemoryBoard>);
    impl BoardRepository for Twice {
        fn atomic(
            &self,
            create: bool,
            work: &mut crate::application::swarm::ports::BoardWork<'_>,
        ) -> Result<(), BoardError> {
            self.0.atomic(create, work)?;
            self.0.atomic(create, work)
        }
    }
    let twice = atomic(&Twice(board), false, |_| Ok(()));
    assert_eq!(
        twice.unwrap_err(),
        BoardError::new(
            RefusalKind::Internal,
            "coordination store ran a transaction's work twice"
        )
    );
}
