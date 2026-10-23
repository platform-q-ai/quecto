//! #2661: a ready task is started only once every task it depends on is
//! done or archived, as `list_ready` lists it; the refusal names them.
use super::{confirmed, plan, starter};
use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::{BoardArtifact, ProjectBoardStore};
use crate::application::project_board::use_cases::fixtures_tests::{at, board, fields, slug};
use crate::domain::project_board::entities::task::{Task, TaskFields};
use crate::domain::project_board::value_objects::rule_error::BoardRuleError;
use crate::domain::project_board::value_objects::vocabulary::{PlanState, Resolution, TaskStatus};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// Task `a` (ready, its plan 1 accepted) depending on task `dep` in
/// `status`.
fn depending_on(status: TaskStatus) -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let task = |fields: TaskFields| Task::new(fields).unwrap();
    let a = task(TaskFields {
        accepted_plan: Some(1),
        depends_on: vec![slug("dep")],
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields("a")
    });
    let resolution = match status {
        TaskStatus::Done | TaskStatus::Archived => Some(Resolution::Completed),
        _ => None,
    };
    let dep = task(TaskFields {
        status,
        resolution,
        created: at("09:00:00"),
        updated: at("09:00:00"),
        ..fields("dep")
    });
    let artifacts = [
        BoardArtifact::Task(a),
        BoardArtifact::Task(dep),
        BoardArtifact::Plan(plan(1, PlanState::Accepted)),
    ];
    store.write(&head, &artifacts, "seed").unwrap();
    store
}

#[test]
fn a_task_whose_dependency_is_not_done_is_not_started() {
    let store = depending_on(TaskStatus::Ready);
    let writes = store.writes();
    let refused = starter(&store, Some(super::Launcher::launching())).execute(&confirmed(1));
    assert_eq!(
        refused,
        Err(ProjectBoardError::Rule(BoardRuleError::DependencyNotDone {
            undone: vec![slug("dep")],
            missing: vec![],
        }))
    );
    assert_eq!(store.writes(), writes, "nothing is written");
    for done in [TaskStatus::Done, TaskStatus::Archived] {
        let store = depending_on(done);
        let started = starter(&store, Some(super::Launcher::launching())).execute(&confirmed(1));
        assert!(started.is_ok(), "{done:?}: {started:?}");
    }
}
