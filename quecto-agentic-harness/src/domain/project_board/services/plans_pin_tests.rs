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

/// Review round 2 (LOW-3): the pin is the acceptance's own, judged after
/// the task is open to plans and the version is a draft, beside its
/// `updated`: a stale reader of a done task hears it is done, and one who
/// saw another acceptance hears that, never building on it.
#[test]
fn an_acceptance_judges_its_pin_beside_its_read() {
    use crate::domain::project_board::services::fixtures_tests::{at, person, plan};
    use crate::domain::project_board::value_objects::vocabulary::PlanState;
    use crate::domain::project_board::value_objects::vocabulary::TaskStatus::Done;
    let versions = [plan(1, PlanState::Accepted), plan(2, PlanState::Draft)];
    let updated = at("08:30:00");
    let accepting = |task: &Task, seen: Option<u32>| {
        accept(
            &TaskPlans {
                task,
                versions: &versions,
                runs: &[],
            },
            2,
            Some(AsRead {
                updated: &updated,
                accepted: seen,
            }),
            &person("Ada"),
            &Moment {
                now: at("10:00:00"),
                latest: None,
            },
        )
        .map(|accepted| accepted.plan.fields().version)
    };
    let mut done = task(Done).into_fields();
    done.accepted_plan = Some(1);
    let done = Task::new(done).unwrap();
    assert_eq!(
        accepting(&done, None),
        Err(BoardRuleError::TaskFinished(Done)),
        "a done task is said done before a stale pin"
    );
    assert_eq!(
        accepting(&naming(Some(1)), None),
        Err(BoardRuleError::AcceptedPlanChanged { accepted: Some(1) })
    );
    assert_eq!(accepting(&naming(Some(1)), Some(1)), Ok(2));
}
