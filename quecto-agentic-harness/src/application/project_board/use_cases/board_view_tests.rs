use super::*;
use crate::application::project_board::dto::IdUser;
use crate::application::project_board::ports::{BoardHead, QuarantinedFile};
use crate::application::project_board::use_cases::fixtures_tests::{fields, slug};
use crate::domain::project_board::entities::task::Task;
use crate::infrastructure::test_support::board_artifacts::{change, discussion, plan, review, run};

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
    }
}

fn set_aside(key: Option<ArtifactKey>, path: &str) -> QuarantinedFile {
    QuarantinedFile {
        path: path.into(),
        // A file of a known artifact is carried; a stray is not.
        carried: key.is_some(),
        key,
        reason: "malformed".into(),
        object: "0".repeat(40),
    }
}

/// The file of `key`, by the plan's names.
fn path_of(key: &ArtifactKey) -> String {
    match key {
        ArtifactKey::Task(id) => format!("tasks/{}.md", id.as_str()),
        ArtifactKey::Plan(id, n) => format!("plans/{}.{n}.md", id.as_str()),
        ArtifactKey::Run(id, n) => format!("runs/{}.{n}.md", id.as_str()),
        ArtifactKey::Change(id, n) => format!("changes/{}.{n}.md", id.as_str()),
        ArtifactKey::Review(id, n) => format!("reviews/{}.{n}.md", id.as_str()),
        ArtifactKey::Discussion(id) => format!("discussions/{}.md", id.as_str()),
    }
}

/// The file of `key` set aside.
fn keyed(key: ArtifactKey) -> QuarantinedFile {
    let path = path_of(&key);
    set_aside(Some(key), &path)
}

fn refused(key: ArtifactKey) -> ProjectBoardError {
    let path = path_of(&key);
    ProjectBoardError::Quarantined { key, path }
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
    let board = board(vec![keyed(ArtifactKey::Task(slug("a")))]);
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
    let board = board(vec![keyed(ArtifactKey::Run(slug("a"), 3))]);
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
        keyed(ArtifactKey::Run(slug("b"), 1)),
        keyed(ArtifactKey::Plan(slug("b"), 1)),
        keyed(ArtifactKey::Change(slug("b"), 1)),
        keyed(ArtifactKey::Review(slug("b"), 1)),
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
        keyed(ArtifactKey::Run(slug("a"), 9)),
        keyed(ArtifactKey::Change(slug("b"), 9)),
        keyed(ArtifactKey::Change(slug("b"), 4)),
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
        set_aside(None, "runs/a.x.md"),
        set_aside(None, "changes/a.1.json"),
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
    let board = board(vec![keyed(ArtifactKey::Discussion(slug("a")))]);
    let a = slug("a");
    assert!(task(&board, &a).is_ok());
    assert!(plans_of(&board, &a).is_ok());
    assert!(runs_of(&board, &a).is_ok());
    assert!(changes_of(&board, &a).is_ok());
    assert!(all_changes(&board).is_ok());
    assert!(review_of(&board, &a, 1).is_ok());
}

/// No store hands over a task both read and set aside: the check every
/// store's read makes stops one in a debug build.
#[test]
#[cfg(debug_assertions)]
#[should_panic(expected = "a task is set aside and read at once")]
fn a_store_never_reads_a_task_it_set_aside() {
    let _ = board(vec![keyed(ArtifactKey::Task(slug("a")))]).asserted();
}

/// The board with one more copy of an artifact of `kind` that it holds
/// already: what a store that broke its contract (one artifact per key)
/// would hand over.
fn twice(kind: &str) -> (BoardSnapshot, ArtifactKey) {
    let mut board = board(vec![]);
    let key = match kind {
        "task" => {
            board.tasks.push(a_task("a"));
            ArtifactKey::Task(slug("a"))
        }
        "plan" => {
            board.plans.push(plan("a", 2));
            ArtifactKey::Plan(slug("a"), 2)
        }
        "run" => {
            board.runs.push(run("a", 2));
            ArtifactKey::Run(slug("a"), 2)
        }
        "change" => {
            board.changes.push(change("a", 2));
            ArtifactKey::Change(slug("a"), 2)
        }
        "review" => {
            board.reviews.push(review("a", 1));
            ArtifactKey::Review(slug("a"), 1)
        }
        _ => {
            board.discussions = vec![discussion("d"), discussion("d")];
            ArtifactKey::Discussion(slug("d"))
        }
    };
    (board, key)
}

const KINDS: [&str; 6] = ["task", "plan", "run", "change", "review", "discussion"];

/// No store hands over an artifact twice, of any kind: the check every
/// store's read makes stops one in a debug build.
#[test]
#[cfg(debug_assertions)]
fn a_store_never_reads_an_artifact_twice() {
    assert!(std::panic::catch_unwind(|| board(vec![]).asserted()).is_ok());
    for kind in KINDS {
        let (board, _) = twice(kind);
        let stopped = std::panic::catch_unwind(|| board.asserted());
        let message = stopped
            .expect_err(kind)
            .downcast::<&str>()
            .map(|message| message.to_string())
            .unwrap_or_default();
        assert_eq!(message, "the board holds an artifact twice", "{kind}");
    }
}

/// A view that finds an artifact twice refuses, naming it, rather than
/// pick one or panic; another task's view is not refused.
#[test]
fn a_view_refuses_an_artifact_found_twice() {
    let a = slug("a");
    let duplicate = |key: ArtifactKey| Err(ProjectBoardError::Duplicate { key });
    let (board, key) = twice("task");
    assert_eq!(task(&board, &a).map(|_| ()), duplicate(key));
    assert!(task(&board, &slug("b")).is_ok());
    let (board, key) = twice("plan");
    assert_eq!(plans_of(&board, &a).map(|_| ()), duplicate(key));
    assert!(plans_of(&board, &slug("b")).is_ok());
    let (board, key) = twice("run");
    assert_eq!(runs_of(&board, &a).map(|_| ()), duplicate(key));
    assert!(runs_of(&board, &slug("b")).is_ok());
    let (board, key) = twice("change");
    assert_eq!(changes_of(&board, &a).map(|_| ()), duplicate(key.clone()));
    assert_eq!(all_changes(&board).map(|_| ()), duplicate(key));
    assert!(changes_of(&board, &slug("b")).is_ok());
    let (board, key) = twice("review");
    assert_eq!(review_of(&board, &a, 1).map(|_| ()), duplicate(key));
    assert!(review_of(&board, &slug("b"), 1).is_ok());
}

/// What names a task's id is the least such artifact, whatever order the
/// store hands them over in.
#[test]
fn naming_names_the_least_artifact_naming_the_id() {
    let mut board = board(vec![]);
    board.tasks.retain(|task| task.fields().id != slug("a"));
    board.plans.reverse();
    board.discussions = vec![discussion("chat")];
    assert_eq!(naming(&board, &slug("a")), Ok(Some(IdUser::Plan(1))));
    assert_eq!(naming(&board, &slug("z")), Ok(None));
    let set_aside = keyed(ArtifactKey::Discussion(slug("old")));
    let mut blocked = board.clone();
    blocked.quarantined = vec![set_aside];
    assert_eq!(
        naming(&blocked, &slug("z")),
        Err(refused(ArtifactKey::Discussion(slug("old"))))
    );
}

/// Reviews come in the store's order, unsorted: of reviews 3 and 1 of the
/// id's task, the least, review 1, is named.
#[test]
fn naming_names_the_least_review_whatever_the_store_order() {
    let mut board = board(vec![]);
    board.tasks.clear();
    board.plans.clear();
    board.runs.clear();
    board.changes.clear();
    board.reviews = vec![review("a", 3), review("a", 1)];
    assert_eq!(naming(&board, &slug("a")), Ok(Some(IdUser::Review(1))));
}
