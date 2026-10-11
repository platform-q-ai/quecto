//! [`BoardListings`] over `QueryBoard` for tests (#2477): the index's
//! meaning (its contract holds it to `QueryBoard`), on any store, such as
//! the in-memory one.
use crate::application::project_board::dto::{BoardListing, ProjectBoardError};
use crate::application::project_board::ports::{BoardListings, BoardStoreError};
use crate::application::project_board::use_cases::QueryBoard;
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
