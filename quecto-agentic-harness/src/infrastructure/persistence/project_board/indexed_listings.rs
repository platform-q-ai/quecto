//! The board's listings from the local index, kept in step with the store:
//! each listing refreshes the index first (rebuilding it when the board
//! moved), and an index that cannot be read is discarded and rebuilt once.
use super::index::{BoardIndex, BoardRevisions, IndexError};
use crate::application::project_board::artifact_listings;
use crate::application::project_board::dto::{ArtifactListing, BoardListing};
use crate::application::project_board::ports::{BoardListings, BoardStoreError, ProjectBoardStore};
use crate::domain::project_board::entities::change::Change;
use crate::domain::project_board::entities::discussion::Discussion;
use crate::domain::project_board::entities::plan::Plan;
use crate::domain::project_board::entities::review::Review;
use crate::domain::project_board::entities::run::Run;
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use std::sync::Arc;

pub struct IndexedBoardListings {
    index: BoardIndex,
    store: Arc<dyn ProjectBoardStore>,
    revisions: Arc<dyn BoardRevisions>,
}

impl IndexedBoardListings {
    pub fn new(
        index: BoardIndex,
        store: Arc<dyn ProjectBoardStore>,
        revisions: Arc<dyn BoardRevisions>,
    ) -> Self {
        Self {
            index,
            store,
            revisions,
        }
    }

    /// `query` against an index refreshed from the board; an index that
    /// cannot be read is discarded and rebuilt once.
    fn listed(
        &self,
        query: impl Fn(&BoardIndex) -> Result<BoardListing, IndexError>,
    ) -> Result<BoardListing, BoardStoreError> {
        let refresh = || self.index.refresh(&*self.store, &*self.revisions);
        refresh().map_err(store_error)?;
        match query(&self.index) {
            Ok(listing) => Ok(listing),
            Err(IndexError::NeedsRebuild(_) | IndexError::Corrupt(_)) => {
                self.index.discard().map_err(store_error)?;
                refresh().map_err(store_error)?;
                query(&self.index).map_err(store_error)
            }
            Err(error) => Err(store_error(error)),
        }
    }
}

fn store_error(error: IndexError) -> BoardStoreError {
    match error {
        IndexError::Store(error) => error,
        IndexError::NeedsRebuild(_)
        | IndexError::Corrupt(_)
        | IndexError::Sqlite(_)
        | IndexError::Io(_) => BoardStoreError::Unavailable(format!("the board index: {error}")),
    }
}

impl BoardListings for IndexedBoardListings {
    fn ready(&self) -> Result<BoardListing, BoardStoreError> {
        self.listed(BoardIndex::ready)
    }

    fn takeable(&self, now: &Timestamp) -> Result<BoardListing, BoardStoreError> {
        self.listed(|index| index.takeable(now))
    }

    fn all(&self) -> Result<BoardListing, BoardStoreError> {
        self.listed(BoardIndex::all)
    }

    fn by_status(&self, status: TaskStatus) -> Result<BoardListing, BoardStoreError> {
        self.listed(|index| index.by_status(status))
    }

    fn children(&self, parent: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.listed(|index| index.children(parent))
    }

    fn dependents(&self, task: &Slug) -> Result<BoardListing, BoardStoreError> {
        self.listed(|index| index.dependents(task))
    }

    // The other kinds are read from the store's snapshot until the index
    // keeps a table per kind (S12).
    fn plans_of(&self, task: &Slug) -> Result<ArtifactListing<Plan>, BoardStoreError> {
        Ok(artifact_listings::plans_of(self.store.read()?, task))
    }

    fn runs_of_plan(
        &self,
        task: &Slug,
        version: u32,
    ) -> Result<ArtifactListing<Run>, BoardStoreError> {
        Ok(artifact_listings::runs_of_plan(
            self.store.read()?,
            task,
            version,
        ))
    }

    fn changes_of(&self, task: &Slug) -> Result<ArtifactListing<Change>, BoardStoreError> {
        Ok(artifact_listings::changes_of(self.store.read()?, task))
    }

    fn review_of(
        &self,
        task: &Slug,
        change: u32,
    ) -> Result<ArtifactListing<Review>, BoardStoreError> {
        Ok(artifact_listings::review_of(
            self.store.read()?,
            task,
            change,
        ))
    }

    fn discussions_of(&self, task: &Slug) -> Result<ArtifactListing<Discussion>, BoardStoreError> {
        Ok(artifact_listings::discussions_of(self.store.read()?, task))
    }
}
