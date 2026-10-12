use super::ProjectBoardError;
use crate::application::project_board::ports::{BoardClockError, BoardStoreError};

/// A store's failure says which kind it is, not only the store's words.
#[test]
fn a_store_failure_names_its_kind() {
    let shown = |error: BoardStoreError| ProjectBoardError::Store(error).to_string();
    assert_eq!(
        shown(BoardStoreError::Clash("refs/heads/quecto exists".into())),
        "the board clashes with the repository: refs/heads/quecto exists"
    );
    assert_eq!(
        shown(BoardStoreError::InvalidWrite(
            "a write holds each task once".into()
        )),
        "the board refused an invalid write: a write holds each task once"
    );
    assert_eq!(
        shown(BoardStoreError::Unavailable("the remote hung up".into())),
        "the board store is unavailable: the remote hung up"
    );
    assert_eq!(
        shown(BoardStoreError::NoBoard),
        "the project has no board yet"
    );
}

#[test]
fn a_clock_that_cannot_tell_the_time_is_reported() {
    let error: ProjectBoardError =
        BoardClockError::OutOfRange("1969-12-31T23:59:59Z".into()).into();
    assert_eq!(
        error.to_string(),
        "this clock reads 1969-12-31T23:59:59Z, a time the board cannot spell; fix the clock"
    );
}

/// A board with these files set aside, holding `plans` and `runs`.
fn snapshot(
    quarantined: Vec<crate::application::project_board::ports::QuarantinedFile>,
) -> crate::application::project_board::ports::BoardSnapshot {
    use crate::application::project_board::ports::{BoardHead, BoardSnapshot};
    BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![],
        plans: vec![],
        runs: vec![],
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined,
    }
}

fn slug(text: &str) -> crate::domain::project_board::value_objects::slug::Slug {
    crate::domain::project_board::value_objects::slug::Slug::parse(text).unwrap()
}

/// A listing names the files set aside that may hold one of its items, so
/// a caller can tell it may be incomplete: a plan's file for the task's
/// plans, any run of the task for one plan's runs (a run's name gives no
/// plan version), the change's own review, any discussion (its name gives
/// no task), and in every scope a file whose name spells no key (it may
/// hold anything); never another task's file or another kind's.
#[test]
fn a_listing_names_the_set_aside_files_in_its_scope() {
    use super::{ArtifactListing, ArtifactScope};
    use crate::application::project_board::ports::{ArtifactKey, QuarantinedFile};
    let file = |key: Option<ArtifactKey>, path: &str| QuarantinedFile {
        path: path.into(),
        carried: key.is_some(),
        key,
        reason: "malformed".into(),
        object: "0".repeat(40),
    };
    let board = snapshot(vec![
        file(Some(ArtifactKey::Task(slug("a"))), "tasks/a.md"),
        file(Some(ArtifactKey::Plan(slug("a"), 2)), "plans/a.2.md"),
        file(Some(ArtifactKey::Plan(slug("b"), 1)), "plans/b.1.md"),
        file(Some(ArtifactKey::Run(slug("a"), 7)), "runs/a.7.md"),
        file(Some(ArtifactKey::Change(slug("a"), 3)), "changes/a.3.md"),
        file(Some(ArtifactKey::Review(slug("a"), 3)), "reviews/a.3.md"),
        file(Some(ArtifactKey::Discussion(slug("d"))), "discussions/d.md"),
        file(None, "plans/notes.txt"),
    ]);
    let a = slug("a");
    let paths = |files: Vec<&QuarantinedFile>| -> Vec<String> {
        files.into_iter().map(|file| file.path.clone()).collect()
    };
    let plans = ArtifactListing::plans_of(&board, &a).unwrap();
    assert_eq!(plans.scope, ArtifactScope::Plans { task: a.clone() });
    assert_eq!(
        paths(plans.set_aside_in_scope()),
        ["plans/a.2.md", "plans/notes.txt"]
    );
    let runs = ArtifactListing::runs_of_plan(&board, &a, 1).unwrap();
    assert_eq!(
        paths(runs.set_aside_in_scope()),
        ["runs/a.7.md", "plans/notes.txt"]
    );
    let changes = ArtifactListing::changes_of(&board, &a).unwrap();
    assert_eq!(
        paths(changes.set_aside_in_scope()),
        ["changes/a.3.md", "plans/notes.txt"]
    );
    let review = ArtifactListing::review_of(&board, &a, 3).unwrap();
    assert_eq!(
        paths(review.set_aside_in_scope()),
        ["reviews/a.3.md", "plans/notes.txt"]
    );
    let other_review = ArtifactListing::review_of(&board, &a, 1).unwrap();
    assert_eq!(
        paths(other_review.set_aside_in_scope()),
        ["plans/notes.txt"]
    );
    let discussions = ArtifactListing::discussions_of(&board, &a).unwrap();
    assert_eq!(
        paths(discussions.set_aside_in_scope()),
        ["discussions/d.md", "plans/notes.txt"]
    );
    let elsewhere = ArtifactListing::changes_of(&board, &slug("c")).unwrap();
    assert_eq!(paths(elsewhere.set_aside_in_scope()), ["plans/notes.txt"]);
    assert_eq!(
        elsewhere.quarantined.len(),
        8,
        "every file is still carried"
    );
}

/// A listing that finds an artifact twice, of any kind, refuses, naming
/// it, as a board view does: what a store that broke its contract holds
/// is unknown.
#[test]
fn a_listing_refuses_an_artifact_found_twice() {
    use super::{ArtifactListing, ProjectBoardError};
    use crate::application::project_board::ports::ArtifactKey;
    use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields};
    use crate::infrastructure::test_support::board_artifacts::{
        change, discussion, plan, review, run,
    };
    let a = slug("a");
    let duplicate = |key: ArtifactKey| Err(ProjectBoardError::Duplicate { key });
    let mut board = snapshot(vec![]);
    board.plans = vec![plan("a", 1), plan("a", 2), plan("a", 1)];
    board.runs = vec![run("a", 3), run("a", 3)];
    board.changes = vec![change("a", 2), change("a", 2)];
    board.reviews = vec![review("a", 1), review("a", 1)];
    let on_a = Discussion::new(DiscussionFields {
        task: Some(a.clone()),
        ..discussion("d").into_fields()
    })
    .unwrap();
    board.discussions = vec![on_a.clone(), on_a];
    assert_eq!(
        ArtifactListing::plans_of(&board, &a).map(|_| ()),
        duplicate(ArtifactKey::Plan(a.clone(), 1))
    );
    assert_eq!(
        ArtifactListing::runs_of_plan(&board, &a, 1).map(|_| ()),
        duplicate(ArtifactKey::Run(a.clone(), 3))
    );
    assert_eq!(
        ArtifactListing::changes_of(&board, &a).map(|_| ()),
        duplicate(ArtifactKey::Change(a.clone(), 2))
    );
    assert_eq!(
        ArtifactListing::review_of(&board, &a, 1).map(|_| ()),
        duplicate(ArtifactKey::Review(a.clone(), 1))
    );
    assert_eq!(
        ArtifactListing::discussions_of(&board, &a).map(|_| ()),
        duplicate(ArtifactKey::Discussion(slug("d")))
    );
    let b = slug("b");
    assert!(
        ArtifactListing::plans_of(&board, &b).is_ok(),
        "another task's"
    );
    assert!(ArtifactListing::review_of(&board, &a, 2).is_ok());
}
