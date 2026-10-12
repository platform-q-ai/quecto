//! [`BoardListings`] over `QueryBoard` for tests (#2477): the index's
//! meaning (its contract holds it to `QueryBoard`), on any store, such as
//! the in-memory one; and [`assembled`], the operations over a store as
//! composition assembles them.
use std::sync::Arc;

use super::{BoardOperationParts, NoPullRequestHost};
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::ports::{
    BoardClock, BoardListings, BoardStoreError, ProjectBoardStore, PullRequestStates,
};
use crate::application::project_board::use_cases::{
    BootstrapProjectBoard, ChangeBoardTask, ClearSetAside, CompleteMergedTask, CreateBoardTask,
    QueryBoard, RunBoardOperation, UpdateDraftTask,
};
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
