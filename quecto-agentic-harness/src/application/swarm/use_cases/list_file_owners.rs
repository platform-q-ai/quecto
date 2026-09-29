//! `Tasks.file_owners(offset=0, limit=50)` (#2275): a page of the file
//! reservations.
use std::sync::Arc;

use super::OverRepository;

use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::{FileRow, ListFileOwnersRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError, RefusalKind};

/// The largest page.
const PAGE_MAX: i64 = 100;

/// The offset must be an integer of at least 0 and the limit an integer
/// from 1 to 100 (Python's `type(x) is int`: no boolean, no float),
/// checked before the gate. Through the read-only gate (a dead member
/// may read, the run need not be running), the rows ordered by path.
pub struct ListFileOwners {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ListFileOwners {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// The page's bounds, an authorisation refusal, or the store's.
    pub fn execute(&self, request: ListFileOwnersRequest) -> Result<Vec<FileRow>, BoardError> {
        // `as_u64` and `as_i64` answer only a JSON integer: never a
        // boolean, a float or a numeric text.
        let offset = request.offset.as_u64();
        let limit = request
            .limit
            .as_i64()
            .filter(|limit| (1..=PAGE_MAX).contains(limit));
        let (Some(offset), Some(limit)) = (offset, limit) else {
            return Err(BoardError::new(
                RefusalKind::Invalid,
                "file page requires nonnegative offset and limit 1 through 100",
            ));
        };
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            &request.actor,
            reading,
            |transaction, _| transaction.file_page(offset, limit),
        )
    }
}

impl OverRepository for ListFileOwners {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "list_file_owners_tests.rs"]
mod tests;
