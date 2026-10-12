use super::*;
use crate::application::project_board::ports::{BoardHead, QuarantinedFile};
use crate::application::project_board::use_cases::fixtures_tests::{fields, slug};
use crate::domain::project_board::entities::task::Task;
use crate::infrastructure::test_support::board_artifacts::{change, plan, review, run};

fn a_task(id: &str) -> Task {
    Task::new(fields(id)).unwrap()
}

/// A board of tasks `a` and `b`, each with plans 1 and 2, runs 1 and 2,
/// changes 1 and 2 and a review of change 1, written out of order, and
/// these files set aside.
fn board(set_aside: Vec<QuarantinedFile>) -> BoardSnapshot {
    BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![a_task("a"), a_task("b")],
        plans: ["b", "a"]
            .into_iter()
            .flat_map(|task| [plan(task, 2), plan(task, 1)])
            .collect(),
        runs: ["b", "a"]
            .into_iter()
            .flat_map(|task| [run(task, 2), run(task, 1)])
            .collect(),
        changes: ["b", "a"]
            .into_iter()
            .flat_map(|task| [change(task, 2), change(task, 1)])
            .collect(),
        reviews: vec![review("b", 1), review("a", 1)],
        discussions: vec![],
        quarantined: set_aside,
        kept: Default::default(),
    }
}

fn set_aside(key: Option<ArtifactKey>, path: &str) -> QuarantinedFile {
    QuarantinedFile {
        path: path.into(),
        key,
        reason: "malformed".into(),
        object: "0".repeat(40),
    }
}

fn refused(key: ArtifactKey) -> ProjectBoardError {
    ProjectBoardError::Quarantined(key)
}

fn numbers<T>(items: &[&T], number: impl Fn(&T) -> (String, u32)) -> Vec<(String, u32)> {
    items.iter().map(|item| number(item)).collect()
}

#[test]
fn every_view_answers_its_own_scope_in_order() {
    let board = board(vec![]);
    let a = slug("a");
    assert_eq!(task(&board, &a).unwrap().fields().id, a);
    assert_eq!(
        task(&board, &slug("c")),
        Err(ProjectBoardError::NotFound(slug("c")))
    );
    let plans = plans_of(&board, &a).unwrap();
    assert_eq!(
        numbers(&plans, |p| (
            p.fields().task.as_str().into(),
            p.fields().version
        )),
        [("a".into(), 1), ("a".into(), 2)]
    );
    let runs = runs_of(&board, &a).unwrap();
    assert_eq!(
        numbers(&runs, |r| (
            r.fields().task.as_str().into(),
            r.fields().number
        )),
        [("a".into(), 1), ("a".into(), 2)]
    );
    let changes = changes_of(&board, &a).unwrap();
    assert_eq!(
        numbers(&changes, |c| (
            c.fields().task.as_str().into(),
            c.fields().number
        )),
        [("a".into(), 1), ("a".into(), 2)]
    );
    let every = all_changes(&board).unwrap();
    assert_eq!(
        numbers(&every, |c| (
            c.fields().task.as_str().into(),
            c.fields().number
        )),
        [
            ("a".into(), 1),
            ("a".into(), 2),
            ("b".into(), 1),
            ("b".into(), 2)
        ]
    );
    assert_eq!(review_of(&board, &a, 1).unwrap(), Some(&review("a", 1)));
    assert_eq!(review_of(&board, &a, 2).unwrap(), None);
}

#[test]
fn a_task_set_aside_is_refused_even_with_a_copy_read() {
    let board = board(vec![set_aside(
        Some(ArtifactKey::Task(slug("a"))),
        "tasks/a.json",
    )]);
    assert_eq!(
        task(&board, &slug("a")),
        Err(refused(ArtifactKey::Task(slug("a"))))
    );
    assert!(task(&board, &slug("b")).is_ok());
    // A task's own file blocks no view of its other artifacts.
    assert!(plans_of(&board, &slug("a")).is_ok());
    assert!(runs_of(&board, &slug("a")).is_ok());
    assert!(changes_of(&board, &slug("a")).is_ok());
    assert!(all_changes(&board).is_ok());
    assert!(review_of(&board, &slug("a"), 1).is_ok());
}

#[test]
fn runs_of_refuses_while_a_run_of_the_task_is_set_aside() {
    let board = board(vec![set_aside(
        Some(ArtifactKey::Run(slug("a"), 3)),
        "runs/a/3.md",
    )]);
    assert_eq!(
        runs_of(&board, &slug("a")),
        Err(refused(ArtifactKey::Run(slug("a"), 3)))
    );
    // Only runs are in its scope.
    assert!(task(&board, &slug("a")).is_ok());
    assert!(plans_of(&board, &slug("a")).is_ok());
    assert!(changes_of(&board, &slug("a")).is_ok());
    assert!(review_of(&board, &slug("a"), 3).is_ok());
}

#[test]
fn another_tasks_set_aside_run_does_not_block() {
    let board = board(vec![
        set_aside(Some(ArtifactKey::Run(slug("b"), 1)), "runs/b/1.md"),
        set_aside(Some(ArtifactKey::Plan(slug("b"), 1)), "plans/b/1.md"),
        set_aside(Some(ArtifactKey::Change(slug("b"), 1)), "changes/b/1.md"),
        set_aside(Some(ArtifactKey::Review(slug("b"), 1)), "reviews/b/1.md"),
    ]);
    let a = slug("a");
    assert_eq!(runs_of(&board, &a).unwrap().len(), 2);
    assert_eq!(plans_of(&board, &a).unwrap().len(), 2);
    assert_eq!(changes_of(&board, &a).unwrap().len(), 2);
    assert!(review_of(&board, &a, 1).unwrap().is_some());
    assert_eq!(
        runs_of(&board, &slug("b")),
        Err(refused(ArtifactKey::Run(slug("b"), 1)))
    );
    assert_eq!(
        plans_of(&board, &slug("b")),
        Err(refused(ArtifactKey::Plan(slug("b"), 1)))
    );
    assert_eq!(
        changes_of(&board, &slug("b")),
        Err(refused(ArtifactKey::Change(slug("b"), 1)))
    );
    assert_eq!(
        review_of(&board, &slug("b"), 1),
        Err(refused(ArtifactKey::Review(slug("b"), 1)))
    );
    // The review of another change of the same task is not blocked.
    assert_eq!(review_of(&board, &slug("b"), 2), Ok(None));
}

#[test]
fn all_changes_refuses_while_any_change_is_set_aside() {
    let board = board(vec![
        set_aside(Some(ArtifactKey::Run(slug("a"), 9)), "runs/a/9.md"),
        set_aside(Some(ArtifactKey::Change(slug("b"), 9)), "changes/b/9.md"),
        set_aside(Some(ArtifactKey::Change(slug("b"), 4)), "changes/b/4.md"),
    ]);
    // The least key in scope names the refusal, whatever the store's order.
    assert_eq!(
        all_changes(&board),
        Err(refused(ArtifactKey::Change(slug("b"), 4)))
    );
    assert!(changes_of(&board, &slug("a")).is_ok());
}

#[test]
fn a_stray_blocks_nothing() {
    let board = board(vec![
        set_aside(None, "runs/a/x.md"),
        set_aside(None, "changes/a/1.json"),
        set_aside(None, "stray.txt"),
    ]);
    let a = slug("a");
    assert!(task(&board, &a).is_ok());
    assert!(plans_of(&board, &a).is_ok());
    assert!(runs_of(&board, &a).is_ok());
    assert!(changes_of(&board, &a).is_ok());
    assert!(all_changes(&board).is_ok());
    assert!(review_of(&board, &a, 1).is_ok());
}

#[test]
fn a_set_aside_discussion_blocks_no_task_view() {
    let board = board(vec![set_aside(
        Some(ArtifactKey::Discussion(slug("a"))),
        "discussions/a.md",
    )]);
    let a = slug("a");
    assert!(task(&board, &a).is_ok());
    assert!(plans_of(&board, &a).is_ok());
    assert!(runs_of(&board, &a).is_ok());
    assert!(all_changes(&board).is_ok());
    assert!(review_of(&board, &a, 1).is_ok());
}
