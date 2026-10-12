//! `QueryBoard`'s listings of a task's other artifacts: each in its order,
//! an item pointing at what the board does not hold still listed, an
//! unknown argument listing nothing, and every file set aside carried.
use super::QueryBoard;
use crate::application::project_board::dto::ArtifactListing;
use crate::application::project_board::ports::{
    ArtifactKey, BoardArtifact, BoardWrite, ProjectBoardStore, QuarantinedFile,
};
use crate::application::project_board::use_cases::fixtures_tests::{board, fields, slug};
use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields};
use crate::domain::project_board::entities::run::{Run, RunFields};
use crate::domain::project_board::entities::task::Task;
use crate::infrastructure::test_support::board_artifacts::{change, discussion, plan, review, run};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;
use std::sync::Arc;

/// Run `number` of `task`'s plan `version`.
fn run_of(task: &str, number: u32, version: u32) -> Run {
    Run::new(RunFields {
        plan_version: version,
        ..run(task, number).into_fields()
    })
    .unwrap()
}

/// Discussion `id` linked to `task`.
fn discussion_on(id: &str, task: &str) -> Discussion {
    Discussion::new(DiscussionFields {
        task: Some(slug(task)),
        ..discussion(id).into_fields()
    })
    .unwrap()
}

/// Tasks `a` and `b`, written out of order with artifacts of `a`, of `b`
/// and of `gone` (a task the board does not hold); run 5 of `a` is set
/// aside, and so is a stray.
fn seeded() -> MemoryBoardStore {
    let store = board();
    let head = store.read().unwrap().head;
    let artifacts = vec![
        BoardArtifact::Task(Task::new(fields("b")).unwrap()),
        BoardArtifact::Task(Task::new(fields("a")).unwrap()),
        BoardArtifact::Plan(plan("a", 2)),
        BoardArtifact::Plan(plan("b", 1)),
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Plan(plan("gone", 1)),
        BoardArtifact::Run(run_of("a", 3, 1)),
        BoardArtifact::Run(run_of("a", 2, 2)),
        BoardArtifact::Run(run_of("a", 1, 1)),
        BoardArtifact::Run(run_of("a", 4, 9)),
        BoardArtifact::Run(run_of("b", 1, 1)),
        BoardArtifact::Change(change("a", 2)),
        BoardArtifact::Change(change("b", 1)),
        BoardArtifact::Change(change("a", 1)),
        BoardArtifact::Review(review("a", 2)),
        BoardArtifact::Review(review("a", 1)),
        BoardArtifact::Review(review("a", 7)),
        BoardArtifact::Discussion(discussion_on("z-last", "a")),
        BoardArtifact::Discussion(discussion_on("m-first", "a")),
        BoardArtifact::Discussion(discussion_on("on-b", "b")),
        BoardArtifact::Discussion(discussion_on("on-gone", "gone")),
        BoardArtifact::Discussion(discussion("unlinked")),
    ];
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store.quarantine_key(ArtifactKey::Run(slug("a"), 5), "runs/a.5.md", "malformed");
    store.set_aside_stray("notes.txt", "not a board file");
    store
}

fn query(store: &MemoryBoardStore) -> QueryBoard {
    QueryBoard::new(Arc::new(store.client()))
}

/// The listing's items by `key`, after checking it carries the board's
/// head and every file set aside.
fn keys<T, K>(
    store: &MemoryBoardStore,
    listing: ArtifactListing<T>,
    key: impl Fn(&T) -> K,
) -> Vec<K> {
    let board = store.read().unwrap();
    assert_eq!(listing.head, board.head);
    let paths = |files: &[QuarantinedFile]| -> Vec<String> {
        files.iter().map(|file| file.path.clone()).collect()
    };
    assert_eq!(paths(&listing.quarantined), ["runs/a.5.md", "notes.txt"]);
    assert_eq!(listing.quarantined, board.quarantined);
    listing.items.iter().map(key).collect()
}

#[test]
fn plans_of_lists_a_tasks_plans_by_version() {
    let store = seeded();
    let query = query(&store);
    let versions = |task: &str| {
        let listing = query.plans_of(&slug(task)).unwrap();
        keys(&store, listing, |plan| {
            (
                plan.fields().task.as_str().to_owned(),
                plan.fields().version,
            )
        })
    };
    assert_eq!(versions("a"), [("a".into(), 1), ("a".into(), 2)]);
    // A plan of a task the board does not hold is still listed.
    assert_eq!(versions("gone"), [("gone".into(), 1)]);
    assert_eq!(versions("unknown"), []);
}

#[test]
fn runs_of_plan_lists_the_runs_of_one_plan_by_number() {
    let store = seeded();
    let query = query(&store);
    let numbers = |task: &str, version: u32| {
        let listing = query.runs_of_plan(&slug(task), version).unwrap();
        keys(&store, listing, |run| {
            let fields = run.fields();
            (
                fields.task.as_str().to_owned(),
                fields.plan_version,
                fields.number,
            )
        })
    };
    assert_eq!(numbers("a", 1), [("a".into(), 1, 1), ("a".into(), 1, 3)]);
    assert_eq!(numbers("a", 2), [("a".into(), 2, 2)]);
    // A run of a plan the board does not hold is still listed.
    assert_eq!(numbers("a", 9), [("a".into(), 9, 4)]);
    assert_eq!(numbers("a", 7), []);
    assert_eq!(numbers("unknown", 1), []);
}

#[test]
fn changes_of_lists_a_tasks_changes_by_number() {
    let store = seeded();
    let query = query(&store);
    let numbers = |task: &str| {
        let listing = query.changes_of(&slug(task)).unwrap();
        keys(&store, listing, |change| {
            (
                change.fields().task.as_str().to_owned(),
                change.fields().number,
            )
        })
    };
    assert_eq!(numbers("a"), [("a".into(), 1), ("a".into(), 2)]);
    assert_eq!(numbers("b"), [("b".into(), 1)]);
    assert_eq!(numbers("unknown"), []);
}

#[test]
fn review_of_lists_none_or_the_one_review_of_a_change() {
    let store = seeded();
    let query = query(&store);
    let found = |task: &str, change: u32| {
        let listing = query.review_of(&slug(task), change).unwrap();
        keys(&store, listing, |review| {
            (
                review.fields().task.as_str().to_owned(),
                review.fields().change,
            )
        })
    };
    assert_eq!(found("a", 1), [("a".into(), 1)]);
    assert_eq!(found("a", 2), [("a".into(), 2)]);
    // A review of a change the board does not hold is still listed.
    assert_eq!(found("a", 7), [("a".into(), 7)]);
    assert_eq!(found("a", 3), []);
    assert_eq!(found("unknown", 1), []);
}

#[test]
fn discussions_of_lists_a_tasks_discussions_by_id() {
    let store = seeded();
    let query = query(&store);
    let ids = |task: &str| {
        let listing = query.discussions_of(&slug(task)).unwrap();
        keys(&store, listing, |discussion| {
            discussion.fields().id.as_str().to_owned()
        })
    };
    assert_eq!(ids("a"), ["m-first", "z-last"]);
    assert_eq!(ids("b"), ["on-b"]);
    // A discussion linked to a task the board does not hold is listed.
    assert_eq!(ids("gone"), ["on-gone"]);
    assert_eq!(ids("unknown"), Vec::<String>::new());
}

#[test]
fn every_artifact_listing_answers_no_board_before_the_bootstrap() {
    use crate::application::project_board::dto::ProjectBoardError;
    use crate::application::project_board::ports::BoardStoreError;
    let query = QueryBoard::new(Arc::new(MemoryBoardStore::default()));
    let a = slug("a");
    let no_board = Err(ProjectBoardError::Store(BoardStoreError::NoBoard));
    assert_eq!(query.plans_of(&a).map(|_| ()), no_board);
    assert_eq!(query.runs_of_plan(&a, 1).map(|_| ()), no_board);
    assert_eq!(query.changes_of(&a).map(|_| ()), no_board);
    assert_eq!(query.review_of(&a, 1).map(|_| ()), no_board);
    assert_eq!(query.discussions_of(&a).map(|_| ()), no_board);
}
