use super::{plan_as_read, task_as_read};
use crate::application::project_board::dto::{ProjectBoardError, ReadPin};
use crate::application::project_board::ports::{BoardHead, ProjectBoardStore};
use crate::application::project_board::use_cases::fixtures_tests::{at, board, fields, slug};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

fn task(id: &str, title: &str) -> Task {
    Task::new(TaskFields {
        title: title.into(),
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields(id)
    })
    .unwrap()
}

/// Writes `tasks` on the board as it stands, and the head written.
fn written(store: &MemoryBoardStore, tasks: &[Task]) -> BoardHead {
    let head = store.read().unwrap().head;
    match store.write_tasks(&head, tasks, "seed").unwrap() {
        crate::application::project_board::ports::BoardWrite::Written(head) => head,
        other => panic!("{other:?}"),
    }
}

fn rev(head: &BoardHead) -> ReadPin {
    ReadPin::Rev(head.clone())
}

#[test]
fn a_rev_pin_holds_while_its_task_is_as_read_whatever_else_moved() {
    let store = board();
    let read = written(&store, &[task("a", "A")]);
    written(&store, &[task("b", "B")]);
    let now = store.read().unwrap();
    assert_ne!(now.head, read, "the board moved since the read");
    let a = &now
        .tasks
        .iter()
        .find(|t| t.fields().id == slug("a"))
        .unwrap();
    assert_eq!(task_as_read(&store, &now, a, Some(&rev(&read))), Ok(()));
    assert_eq!(task_as_read(&store, &now, a, Some(&rev(&now.head))), Ok(()));
    assert_eq!(task_as_read(&store, &now, a, None), Ok(()));
}

#[test]
fn a_rev_pin_from_before_a_change_or_of_no_revision_is_stale() {
    let store = board();
    let read = written(&store, &[task("a", "A")]);
    written(&store, &[task("a", "A2")]);
    let now = store.read().unwrap();
    let a = &now.tasks[0];
    for pin in [
        rev(&read),
        rev(&BoardHead("r999".into())),
        rev(&BoardHead("x".into())),
    ] {
        assert_eq!(
            task_as_read(&store, &now, a, Some(&pin)),
            Err(ProjectBoardError::Stale {
                id: slug("a"),
                expected: pin.clone(),
                updated: at("09:00:00"),
            }),
            "{pin:?}"
        );
    }
}

#[test]
fn a_deprecated_updated_pin_is_judged_by_the_stamp() {
    let store = board();
    written(&store, &[task("a", "A")]);
    let now = store.read().unwrap();
    let a = &now.tasks[0];
    let same = ReadPin::Updated(at("09:00:00"));
    assert_eq!(task_as_read(&store, &now, a, Some(&same)), Ok(()));
    let other = ReadPin::Updated(at("08:00:00"));
    assert!(matches!(
        task_as_read(&store, &now, a, Some(&other)),
        Err(ProjectBoardError::Stale { .. })
    ));
}

#[test]
fn a_plan_pinned_by_a_stale_rev_changed_and_a_deprecated_pin_is_the_rules() {
    use crate::application::project_board::use_cases::fixtures_tests::{holding_plans, plan_of};
    use crate::domain::project_board::value_objects::vocabulary::{PlanState, TaskStatus};
    let store = holding_plans(
        "a",
        TaskStatus::Ready,
        vec![plan_of("a", 1, PlanState::Draft)],
    );
    let read = store.read().unwrap();
    let plan = read.plans[0].clone();
    assert_eq!(
        plan_as_read(&store, &read, &plan, Some(&rev(&read.head))),
        Ok(())
    );
    let stale = rev(&BoardHead("r999".into()));
    assert_eq!(
        plan_as_read(&store, &read, &plan, Some(&stale)),
        Err(ProjectBoardError::Rule(BoardRuleError::PlanChanged {
            version: plan.fields().version,
            updated: plan.fields().updated.clone(),
        }))
    );
    let deprecated = ReadPin::Updated(at("01:00:00"));
    assert_eq!(
        plan_as_read(&store, &read, &plan, Some(&deprecated)),
        Ok(())
    );
}
