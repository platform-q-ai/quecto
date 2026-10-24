//! The board's listings from the local index, kept in step with the store:
//! each listing refreshes the index first (rebuilding it when the board
//! moved), and an index that cannot be read is discarded and rebuilt once.
use super::index::{BoardIndex, BoardRevisions, IndexError};
use crate::application::project_board::dto::BoardListing;
use crate::application::project_board::ports::{BoardListings, BoardStoreError, ProjectBoardStore};
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::TaskStatus;
use std::sync::{Arc, OnceLock};

/// Names the index where it is first needed.
type NameIndex = Box<dyn Fn() -> Result<BoardIndex, IndexError> + Send + Sync>;

/// Where the index is: named as the listings were built, or on the first
/// listing (#2705 B1), so building them asks nothing of git.
enum IndexAt {
    Named(BoardIndex),
    Deferred {
        name: NameIndex,
        named: OnceLock<BoardIndex>,
    },
}

impl IndexAt {
    /// The index, named now if it was deferred and is not yet; a naming
    /// that fails is tried again by the next listing.
    fn index(&self) -> Result<&BoardIndex, IndexError> {
        match self {
            Self::Named(index) => Ok(index),
            Self::Deferred { name, named } => match named.get() {
                Some(index) => Ok(index),
                None => {
                    let index = name()?;
                    Ok(named.get_or_init(|| index))
                }
            },
        }
    }
}

pub struct IndexedBoardListings {
    index: IndexAt,
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
            index: IndexAt::Named(index),
            store,
            revisions,
        }
    }

    /// Listings whose index `name` names on the first listing, not now:
    /// for a front end that builds its board without asking git (#2705
    /// B1).
    pub fn named_when_listed(
        name: impl Fn() -> Result<BoardIndex, IndexError> + Send + Sync + 'static,
        store: Arc<dyn ProjectBoardStore>,
        revisions: Arc<dyn BoardRevisions>,
    ) -> Self {
        Self {
            index: IndexAt::Deferred {
                name: Box::new(name),
                named: OnceLock::new(),
            },
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
        let index = self.index.index().map_err(store_error)?;
        let refresh = || index.refresh(&*self.store, &*self.revisions);
        refresh().map_err(store_error)?;
        match query(index) {
            Ok(listing) => Ok(listing),
            Err(IndexError::NeedsRebuild(_) | IndexError::Corrupt(_)) => {
                index.discard().map_err(store_error)?;
                refresh().map_err(store_error)?;
                query(index).map_err(store_error)
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
}
