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

/// A listing names the files set aside that may hold one of its items, so
/// a caller can tell it may be incomplete: a plan's file for the task's
/// plans, any run of the task for one plan's runs (a run's name gives no
/// plan version), the change's own review, and any discussion (its name
/// gives no task); never another task's file, another kind's, or a stray.
#[test]
fn a_listing_names_the_set_aside_files_in_its_scope() {
    use super::{ArtifactListing, ArtifactScope};
    use crate::application::project_board::ports::{
        ArtifactKey, BoardHead, BoardSnapshot, QuarantinedFile,
    };
    use crate::domain::project_board::value_objects::slug::Slug;
    let slug = |text: &str| Slug::parse(text).unwrap();
    let file = |key: Option<ArtifactKey>, path: &str| QuarantinedFile {
        path: path.into(),
        carried: key.is_some(),
        key,
        reason: "malformed".into(),
        object: "0".repeat(40),
    };
    let board = BoardSnapshot {
        head: BoardHead("h".into()),
        tasks: vec![],
        plans: vec![],
        runs: vec![],
        changes: vec![],
        reviews: vec![],
        discussions: vec![],
        quarantined: vec![
            file(Some(ArtifactKey::Task(slug("a"))), "tasks/a.md"),
            file(Some(ArtifactKey::Plan(slug("a"), 2)), "plans/a.2.md"),
            file(Some(ArtifactKey::Plan(slug("b"), 1)), "plans/b.1.md"),
            file(Some(ArtifactKey::Run(slug("a"), 7)), "runs/a.7.md"),
            file(Some(ArtifactKey::Change(slug("a"), 3)), "changes/a.3.md"),
            file(Some(ArtifactKey::Review(slug("a"), 3)), "reviews/a.3.md"),
            file(Some(ArtifactKey::Discussion(slug("d"))), "discussions/d.md"),
            file(None, "plans/notes.txt"),
        ],
    };
    let a = slug("a");
    let paths = |files: Vec<&QuarantinedFile>| -> Vec<String> {
        files.into_iter().map(|file| file.path.clone()).collect()
    };
    let plans = ArtifactListing::plans_of(&board, &a);
    assert_eq!(plans.scope, ArtifactScope::Plans { task: a.clone() });
    assert_eq!(paths(plans.set_aside_in_scope()), ["plans/a.2.md"]);
    let runs = ArtifactListing::runs_of_plan(&board, &a, 1);
    assert_eq!(paths(runs.set_aside_in_scope()), ["runs/a.7.md"]);
    let changes = ArtifactListing::changes_of(&board, &a);
    assert_eq!(paths(changes.set_aside_in_scope()), ["changes/a.3.md"]);
    let review = ArtifactListing::review_of(&board, &a, 3);
    assert_eq!(paths(review.set_aside_in_scope()), ["reviews/a.3.md"]);
    assert!(
        ArtifactListing::review_of(&board, &a, 1)
            .set_aside_in_scope()
            .is_empty()
    );
    let discussions = ArtifactListing::discussions_of(&board, &a);
    assert_eq!(
        paths(discussions.set_aside_in_scope()),
        ["discussions/d.md"]
    );
    let elsewhere = ArtifactListing::changes_of(&board, &slug("c"));
    assert!(elsewhere.set_aside_in_scope().is_empty());
    assert_eq!(
        elsewhere.quarantined.len(),
        8,
        "every file is still carried"
    );
}
