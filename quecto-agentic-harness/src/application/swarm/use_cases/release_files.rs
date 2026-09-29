//! `Tasks.release_files(task_id, token, reservation)` (#2275): the owner
//! of a claim gives one reservation set back.
use std::sync::Arc;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::owned;
use crate::application::swarm::dto::ReleaseFilesRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (`operation(active=False)`: the run need not
/// be running), only the claim's owner with its current token releases.
/// The rows of that task, owner, claim and ownership token go (none, when
/// the reservation is not there), and the event
/// `files_released{task,token}` names the task id and the reservation as
/// given.
pub struct ReleaseFiles {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl ReleaseFiles {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `unknown task`, `stale or unowned claim`,
    /// or the store's (a reservation it cannot bind).
    pub fn execute(&self, request: ReleaseFilesRequest) -> Result<(), BoardError> {
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _| {
                owned(transaction, &request.task_id, &request.token, actor)?;
                transaction.delete_reservation(
                    &request.task_id,
                    actor,
                    &request.token,
                    &request.reservation,
                )?;
                transaction.event(
                    actor,
                    self.clock.now_seconds(),
                    "files_released",
                    &detail([
                        ("task", request.task_id.clone()),
                        ("token", request.reservation.clone()),
                    ]),
                )
            },
        )
    }
}

#[cfg(test)]
#[path = "release_files_tests.rs"]
mod tests;
