//! `Workbench._socket` (#2271): a member registers the endpoint its harness
//! listens on.
use std::sync::Arc;

use super::OverRepository;
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::RegisterMemberSocketRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (`active=False`): the actor's own row takes
/// the socket. No event.
pub struct RegisterMemberSocket {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RegisterMemberSocket {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, or the store's.
    pub fn execute(&self, request: RegisterMemberSocketRequest) -> Result<(), BoardError> {
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _run| transaction.set_socket(actor, &request.socket),
        )
    }
}

impl OverRepository for RegisterMemberSocket {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "register_member_socket_tests.rs"]
mod tests;
