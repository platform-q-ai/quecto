//! The index's listings of a task's other artifacts, over a store that
//! holds them (the in-memory one: the git store keeps tasks only until
//! S10): each answers its own task's, plan's and change's artifacts.
use super::*;
use crate::application::project_board::ports::{BoardArtifact, BoardWrite};
use crate::domain::project_board::entities::discussion::{Discussion, DiscussionFields};
use crate::domain::project_board::entities::run::RunFields;
use crate::domain::project_board::value_objects::vocabulary::ReviewVerdict;
use crate::infrastructure::test_support::board_artifacts::{change, discussion, plan, review, run};
use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

/// No remote: the listings of other kinds never ask it.
struct NoRemote;

impl BoardRevisions for NoRemote {
    fn remote_head(&self) -> Result<Option<String>, BoardStoreError> {
        Ok(None)
    }

    fn is_ancestor(&self, _: &str, _: &str) -> Result<bool, BoardStoreError> {
        Ok(false)
    }
}

fn slug(text: &str) -> Slug {
    Slug::parse(text).unwrap()
}

/// Task `a`'s plans 1 and 2, runs 1 and 3 of plan 1 and run 2 of plan 2,
/// changes 1 and 2 each with a review (change 2's asks for changes), and
/// a discussion on it; and task `b`'s run 1 of plan 2 and review of
/// change 2.
fn board() -> Arc<MemoryBoardStore> {
    let store = MemoryBoardStore::default();
    store.bootstrap().unwrap();
    let run_of = |task: &str, number: u32, version: u32| {
        BoardArtifact::Run(
            Run::new(RunFields {
                plan_version: version,
                ..run(task, number).into_fields()
            })
            .unwrap(),
        )
    };
    let mut asked = review("a", 2).into_fields();
    asked.rounds[0].verdict = ReviewVerdict::ChangesRequested;
    let on_a = Discussion::new(DiscussionFields {
        task: Some(slug("a")),
        ..discussion("locks").into_fields()
    })
    .unwrap();
    let artifacts = vec![
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Plan(plan("a", 2)),
        run_of("a", 1, 1),
        run_of("a", 2, 2),
        run_of("a", 3, 1),
        run_of("b", 1, 2),
        BoardArtifact::Change(change("a", 1)),
        BoardArtifact::Change(change("a", 2)),
        BoardArtifact::Review(review("a", 1)),
        BoardArtifact::Review(Review::new(asked).unwrap()),
        BoardArtifact::Review(review("b", 2)),
        BoardArtifact::Discussion(on_a),
    ];
    let head = store.read().unwrap().head;
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    Arc::new(store)
}

fn listings(store: Arc<MemoryBoardStore>) -> (tempfile::TempDir, IndexedBoardListings) {
    let dir = tempfile::tempdir().unwrap();
    let index = BoardIndex::at(dir.path().join("board.sqlite"));
    (
        dir,
        IndexedBoardListings::new(index, store, Arc::new(NoRemote)),
    )
}

#[test]
fn the_index_lists_each_plans_runs_and_each_changes_review() {
    let (_dir, listings) = listings(board());
    let a = slug("a");
    let runs = |version: u32| -> Vec<(String, u32, u32)> {
        let listed = listings.runs_of_plan(&a, version).unwrap();
        listed
            .items
            .iter()
            .map(|run| {
                let fields = run.fields();
                (
                    fields.task.as_str().into(),
                    fields.plan_version,
                    fields.number,
                )
            })
            .collect()
    };
    assert_eq!(runs(1), [("a".into(), 1, 1), ("a".into(), 1, 3)]);
    assert_eq!(runs(2), [("a".into(), 2, 2)]);
    assert_eq!(runs(3), []);
    let review = |change: u32| -> Vec<(String, u32, ReviewVerdict)> {
        let listed = listings.review_of(&a, change).unwrap();
        listed
            .items
            .iter()
            .map(|review| {
                let fields = review.fields();
                (fields.task.as_str().into(), fields.change, review.state())
            })
            .collect()
    };
    assert_eq!(review(1), [("a".into(), 1, ReviewVerdict::Approved)]);
    assert_eq!(
        review(2),
        [("a".into(), 2, ReviewVerdict::ChangesRequested)]
    );
    assert_eq!(review(3), []);
    let versions: Vec<u32> = listings
        .plans_of(&a)
        .unwrap()
        .items
        .iter()
        .map(|plan| plan.fields().version)
        .collect();
    assert_eq!(versions, [1, 2]);
    let changes: Vec<u32> = listings
        .changes_of(&a)
        .unwrap()
        .items
        .iter()
        .map(|change| change.fields().number)
        .collect();
    assert_eq!(changes, [1, 2]);
    let discussions = listings.discussions_of(&a).unwrap();
    assert_eq!(discussions.items.len(), 1);
    assert!(
        listings
            .discussions_of(&slug("b"))
            .unwrap()
            .items
            .is_empty()
    );
}

/// Every artifact listing the index answers is `QueryBoard`'s over the
/// same store, items, head and files set aside alike, for a task the
/// board holds artifacts of, another, and none; with a run's file set
/// aside and a stray beside them.
#[test]
fn the_index_lists_every_kind_as_query_board_does() {
    use crate::application::project_board::ports::ArtifactKey;
    use crate::application::project_board::use_cases::QueryBoard;
    let store = board();
    store.quarantine_key(ArtifactKey::Run(slug("a"), 9), "runs/a.9.md", "malformed");
    store.set_aside_stray("notes.txt", "not a board file");
    let (_dir, listings) = listings(Arc::clone(&store));
    let query = QueryBoard::new(store);
    let mut listed = 0;
    for task in ["a", "b", "missing"].map(slug) {
        let plans = listings.plans_of(&task).unwrap();
        assert_eq!(plans.quarantined.len(), 2, "both files carried");
        listed += plans.items.len();
        assert_eq!(plans, query.plans_of(&task).unwrap(), "plans of {task:?}");
        let changes = listings.changes_of(&task).unwrap();
        listed += changes.items.len();
        assert_eq!(changes, query.changes_of(&task).unwrap());
        let discussions = listings.discussions_of(&task).unwrap();
        listed += discussions.items.len();
        assert_eq!(discussions, query.discussions_of(&task).unwrap());
        for number in 1..=3 {
            let runs = listings.runs_of_plan(&task, number).unwrap();
            listed += runs.items.len();
            assert_eq!(runs, query.runs_of_plan(&task, number).unwrap());
            let review = listings.review_of(&task, number).unwrap();
            listed += review.items.len();
            assert_eq!(review, query.review_of(&task, number).unwrap());
        }
    }
    assert_eq!(listed, 12, "every seeded artifact is listed once");
}
