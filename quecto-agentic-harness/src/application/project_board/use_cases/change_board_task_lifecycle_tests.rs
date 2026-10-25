//! #2705 T4 through the change use case: `move` refuses what the
//! lifecycle makes, back to ready from review takes a reason (the CLI's
//! unasked one aside; it is normalised as every entered line is).
use super::tests::{change, held_by_ada, seed};
use crate::application::project_board::dto::{BoardOp, ProjectBoardError};
use crate::application::project_board::ports::ProjectBoardStore;
use crate::application::project_board::use_cases::fixtures_tests::{board, slug};
use crate::domain::project_board::entities::task::TaskFields;
use crate::domain::project_board::services::transitions::MoveReason;
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{LifecycleAction, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

fn current(store: &MemoryBoardStore) -> TaskFields {
    store.read().unwrap().tasks[0].fields().clone()
}

fn refused(error: BoardRuleError) -> Result<TaskStatus, ProjectBoardError> {
    Err(ProjectBoardError::Rule(error))
}

#[test]
fn move_refuses_what_the_lifecycle_makes_and_writes_nothing() {
    let store = board();
    seed(&store, held_by_ada(TaskStatus::InProgress));
    let before = current(&store);
    let a = slug("a");
    for (to, action) in [
        (TaskStatus::Review, LifecycleAction::EndRun),
        (TaskStatus::Blocked, LifecycleAction::EndRun),
    ] {
        let moved = change(&store, "Ada", "10:00:00")
            .execute(&a, &BoardOp::Move(to, MoveReason::Absent))
            .map(|task| task.fields().status);
        assert_eq!(
            moved,
            refused(BoardRuleError::UseLifecycle {
                from: TaskStatus::InProgress,
                to,
                action
            })
        );
    }
    assert_eq!(current(&store), before, "nothing is written");
}

#[test]
fn back_to_ready_from_review_takes_a_reason_but_for_the_clis_unasked_one() {
    let a = slug("a");
    let store = board();
    seed(&store, held_by_ada(TaskStatus::Review));
    let bare = change(&store, "Ada", "10:00:00")
        .execute(&a, &BoardOp::Move(TaskStatus::Ready, MoveReason::Absent))
        .map(|task| task.fields().status);
    assert_eq!(
        bare,
        refused(BoardRuleError::ReasonRequired {
            from: TaskStatus::Review,
            to: TaskStatus::Ready
        })
    );
    // A reason is normalised as it enters: CRLF and chat spaces go.
    let typed = MoveReason::Given("Plan\u{a0}changed: new approach".into());
    change(&store, "Ada", "10:00:00")
        .execute(&a, &BoardOp::Move(TaskStatus::Ready, typed))
        .unwrap();
    let back = current(&store);
    assert_eq!(
        (back.status, back.reason.as_deref()),
        (TaskStatus::Ready, Some("Plan changed: new approach"))
    );
    // The Pocket relay's move back, through the CLI with no reason.
    let relayed = board();
    seed(&relayed, held_by_ada(TaskStatus::Review));
    change(&relayed, "Ada", "10:00:00")
        .execute(&a, &BoardOp::Move(TaskStatus::Ready, MoveReason::Unasked))
        .unwrap();
    let back = current(&relayed);
    assert_eq!((back.status, back.reason), (TaskStatus::Ready, None));
}
