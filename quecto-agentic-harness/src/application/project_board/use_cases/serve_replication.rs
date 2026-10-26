//! A primary's batches for its followers (epic #2705 B11): its store's
//! history after a follower's place, with every admin row (its device
//! tokens' digests), so a token it revokes is revoked on each follower.
use std::sync::Arc;

use crate::application::project_board::dto::ProjectBoardError;
use crate::application::project_board::ports::feed::BatchLimit;
use crate::application::project_board::ports::replication::{
    AdminRows, ReplicaBatch, ReplicationLog,
};
use crate::application::project_board::ports::{BoardStoreError, ServeError};

/// A primary's batches for its followers.
pub struct ServeReplication {
    log: Arc<dyn ReplicationLog>,
    admin: Arc<dyn AdminRows>,
}

impl ServeReplication {
    pub fn new(log: Arc<dyn ReplicationLog>, admin: Arc<dyn AdminRows>) -> Self {
        Self { log, admin }
    }

    /// The batch after revision `after` and object `objects_after`, with
    /// every admin row.
    ///
    /// # Errors
    /// The store or the admin rows could not be read.
    pub fn batch(
        &self,
        after: u64,
        objects_after: u64,
        limit: BatchLimit,
    ) -> Result<ReplicaBatch, ServeError> {
        let failed = |error| ServeError::Board(ProjectBoardError::Store(error));
        let mut batch = self
            .log
            .replica_batch(after, objects_after, limit)
            .map_err(failed)?;
        let tokens = self.admin.export();
        batch.tokens =
            tokens.map_err(|error| failed(BoardStoreError::Unavailable(error.to_string())))?;
        Ok(batch)
    }
}

#[cfg(test)]
#[path = "serve_replication_tests.rs"]
mod tests;
