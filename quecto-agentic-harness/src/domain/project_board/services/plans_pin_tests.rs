//! #2481 M4c review round 1: an acceptance is pinned to the plan the
//! caller saw accepted, so a stale reader never supersedes an acceptance
//! it did not see.
use super::*;
use crate::domain::project_board::services::fixtures_tests::task;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus::Ready;

/// Task `board-store`, naming `accepted` as its plan.
fn naming(accepted: Option<u32>) -> Task {
    let mut fields = task(Ready).into_fields();
    fields.accepted_plan = accepted;
    Task::new(fields).unwrap()
}

#[test]
fn the_accepted_plan_must_be_the_one_the_caller_saw() {
    assert_eq!(as_accepted(&naming(None), None), Ok(()));
    assert_eq!(as_accepted(&naming(Some(3)), Some(3)), Ok(()));
    assert_eq!(
        as_accepted(&naming(Some(3)), Some(2)),
        Err(BoardRuleError::AcceptedPlanChanged { accepted: Some(3) })
    );
    assert_eq!(
        as_accepted(&naming(Some(3)), None),
        Err(BoardRuleError::AcceptedPlanChanged { accepted: Some(3) })
    );
    assert_eq!(
        as_accepted(&naming(None), Some(1)),
        Err(BoardRuleError::AcceptedPlanChanged { accepted: None })
    );
}

#[test]
fn a_changed_acceptance_names_what_the_task_accepts_now() {
    let now = BoardRuleError::AcceptedPlanChanged { accepted: Some(3) };
    assert_eq!(
        now.to_string(),
        "the task's accepted plan changed since it was read (it is now version 3); read it again"
    );
    let none = BoardRuleError::AcceptedPlanChanged { accepted: None };
    assert_eq!(
        none.to_string(),
        "the task's accepted plan changed since it was read (it is now none); read it again"
    );
}
