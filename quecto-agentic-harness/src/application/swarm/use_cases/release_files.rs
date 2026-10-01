//! `Tasks.release_files(task_id, token, reservation)` (#2275): the owner
//! of a claim gives one reservation set back.
use std::sync::Arc;

use super::OverRepository;

use crate::application::swarm::board_operation::{detail, operation};
use crate::application::swarm::board_tasks::{owned, stored_id};
use crate::application::swarm::dto::{ReleaseFilesRequest, ReleasedFiles};
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
    ///
    /// The task's id as its row holds it (#2303), for the call's record,
    /// and how many files the reservation released (#2394).
    pub fn execute(&self, request: ReleaseFilesRequest) -> Result<ReleasedFiles, BoardError> {
        let actor = request.actor.as_str();
        operation(
            &*self.repository,
            &*self.clock,
            actor,
            Access::default(),
            |transaction, _| {
                let task = owned(transaction, &request.task_id, &request.token, actor)?;
                let released = transaction.delete_reservation(
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
                )?;
                Ok(ReleasedFiles {
                    task_id: stored_id(&task),
                    released,
                })
            },
        )
    }
}

impl OverRepository for ReleaseFiles {
    fn over(&self, repository: Arc<dyn BoardRepository>) -> Self {
        Self {
            repository,
            clock: self.clock.clone(),
        }
    }
}

#[cfg(test)]
#[path = "release_files_tests.rs"]
mod tests;
