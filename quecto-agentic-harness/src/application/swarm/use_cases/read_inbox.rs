//! `Workbench.inbox(include_consumed=False)` (#2276): the caller's
//! messages.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::{MessageRow, ReadInboxRequest};
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// A read-only operation (a dead member may still read): the caller's
/// unread messages, or with `include_consumed` every message to it (the
/// audit of consumed, superseded and withdrawn ones), in id order, at most
/// a hundred. The flag is Python's argument bound into the query untyped,
/// so SQLite's truth of it decides.
pub struct ReadInbox {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReadInbox {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, a flag the store cannot bind, or the
    /// store's.
    pub fn execute(&self, request: ReadInboxRequest) -> Result<Vec<MessageRow>, BoardError> {
        let actor = request.actor.as_str();
        let reading = Access {
            read_only: true,
            ..Access::default()
        };
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            reading,
            |transaction, _| transaction.inbox(actor, &request.include_consumed),
        )
    }
}

impl OverRepository for ReadInbox {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "read_inbox_tests.rs"]
mod tests;
