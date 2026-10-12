//! [`BoardListings`] over `QueryBoard` for tests (#2477): the index's
//! meaning (its contract holds it to `QueryBoard`), on any store, such as
//! the in-memory one; and [`assembled`], the operations over a store as
//! composition assembles them.
use std::sync::Arc;

use super::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::dto::{ArtifactListing, BoardListing, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardClock, BoardListings, BoardStoreError, ProjectBoardStore, PullRequestStates,
};
use crate::application::project_board::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    QueryBoard, RunBoardOperation, UpdateDraftTask,
};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;

pub struct QueryListings(pub QueryBoard);

fn store_error(error: ProjectBoardError) -> BoardStoreError {
    match error {
        ProjectBoardError::Store(error) => error,
        other => BoardStoreError::Unavailable(other.to_string()),
    }
}

impl BoardListings for QueryListings {
    fn ready(&self) -> Result<BoardListing, BoardStoreError> {
        self.0.ready().map_err(store_error)
    }

    fn takeable(&self, now: &Timestamp) -> Result<BoardListing, BoardStoreError> {
        self.0.takeable(now).map_err(store_error)
    }

    fn all(&self) -> Result<BoardListing, BoardStoreError> {
        self.0.all().map_err(store_error)
    }

    fn by_status(&self, status: TaskStatus) -> Result<BoardListing, BoardStoreError> {
        self.0.by_status(status).map_err(store_error)
    }

    fn children(&self, parent: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.0.children(parent).map_err(store_error)
    }

    fn dependents(&self, task: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.0.dependents(task).map_err(store_error)
    }

    fn plans_of(&self, task: &Slug) -> Result<ArtifactListing<Plan>, BoardStoreError> {
        self.0.plans_of(task).map_err(store_error)
    }

    fn runs_of_plan(
        &self,
        task: &Slug,
        version: u32,
    ) -> Result<ArtifactListing<Run>, BoardStoreError> {
        self.0.runs_of_plan(task, version).map_err(store_error)
    }

    fn changes_of(&self, task: &Slug) -> Result<ArtifactListing<Change>, BoardStoreError> {
        self.0.changes_of(task).map_err(store_error)
    }

    fn review_of(
        &self,
        task: &Slug,
        change: u32,
    ) -> Result<ArtifactListing<Review>, BoardStoreError> {
        self.0.review_of(task, change).map_err(store_error)
    }

    fn discussions_of(&self, task: &Slug) -> Result<ArtifactListing<Discussion>, BoardStoreError> {
        self.0.discussions_of(task).map_err(store_error)
    }
}

/// The operations over `store`, its `listings` and `clock`, with pull
/// requests read from `prs` (or none, and why), as composition builds
/// them.
pub fn assembled(
    store: Arc<dyn ProjectBoardStore>,
    clock: Arc<dyn BoardClock>,
    listings: Arc<dyn BoardListings>,
    prs: Result<Arc<dyn PullRequestStates>, NoPullRequestHost>,
) -> Result<RunBoardOperation, ProjectBoardError> {
    let complete = match prs {
        Ok(prs) => Ok(CompleteMergedTask::new(
            Arc::clone(&store),
            Arc::clone(&clock),
            prs,
        )?),
        Err(why) => Err(why),
    };
    Ok(RunBoardOperation::new(BoardOperationParts {
        bootstrap: BootstrapProjectBoard::new(Arc::clone(&store)),
        create: CreateBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?,
        update: UpdateDraftTask::new(Arc::clone(&store), Arc::clone(&clock))?,
        change: ChangeBoardTask::new(Arc::clone(&store), Arc::clone(&clock))?,
        clear: ClearSetAside::new(Arc::clone(&store), Arc::clone(&clock))?,
        complete,
        clock,
        listings,
    }))
}

/// Over the in-memory store, which keeps every kind, each artifact
/// listing [`QueryListings`] answers is `QueryBoard`'s, items and files
/// set aside alike, and a store's refusal stays the store's.
#[test]
fn query_listings_list_every_kind_as_query_board_does() {
    use crate::application::project_board::ports::{ArtifactKey, BoardArtifact, BoardWrite};
    use crate::domain::project_board::entities::discussion::DiscussionFields;
    use crate::infrastructure::test_support::board_artifacts::{
        change, discussion, plan, review, run,
    };
    use crate::infrastructure::test_support::memory_board_store::MemoryBoardStore;

    let store = MemoryBoardStore::default();
    let listings = QueryListings(QueryBoard::new(Arc::new(store.client())));
    let a = Slug::parse("a").unwrap();
    assert_eq!(listings.plans_of(&a), Err(BoardStoreError::NoBoard));
    store.bootstrap().unwrap();
    let linked = Discussion::new(DiscussionFields {
        task: Some(a.clone()),
        ..discussion("d").into_fields()
    })
    .unwrap();
    let head = store.read().unwrap().head;
    let artifacts = [
        BoardArtifact::Plan(plan("a", 1)),
        BoardArtifact::Run(run("a", 1)),
        BoardArtifact::Change(change("a", 1)),
        BoardArtifact::Review(review("a", 1)),
        BoardArtifact::Discussion(linked),
    ];
    let written = store.write(&head, &artifacts, "board: seed");
    assert!(matches!(written, Ok(BoardWrite::Written(_))), "{written:?}");
    store.quarantine_key(
        ArtifactKey::Change(a.clone(), 2),
        "changes/a.2.md",
        "malformed",
    );
    let query = QueryBoard::new(Arc::new(store.client()));
    let listed = listings.plans_of(&a).unwrap();
    assert!(!listed.items.is_empty() && !listed.quarantined.is_empty());
    assert_eq!(listed, query.plans_of(&a).unwrap());
    let listed = listings.runs_of_plan(&a, 1).unwrap();
    assert!(!listed.items.is_empty());
    assert_eq!(listed, query.runs_of_plan(&a, 1).unwrap());
    let listed = listings.changes_of(&a).unwrap();
    assert!(!listed.items.is_empty());
    assert_eq!(listed, query.changes_of(&a).unwrap());
    let listed = listings.review_of(&a, 1).unwrap();
    assert!(!listed.items.is_empty());
    assert_eq!(listed, query.review_of(&a, 1).unwrap());
    let listed = listings.discussions_of(&a).unwrap();
    assert!(!listed.items.is_empty());
    assert_eq!(listed, query.discussions_of(&a).unwrap());
}
