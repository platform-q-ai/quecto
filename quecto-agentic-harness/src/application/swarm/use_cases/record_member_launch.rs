//! `Workbench._record_launch` (#2271): the launcher records the process it
//! started for a member, before the member activates itself.
use std::sync::Arc;

use crate::application::swarm::board_membership::{alive, launched_elsewhere};
use crate::application::swarm::board_operation::operation;
use crate::application::swarm::dto::RecordMemberLaunchRequest;
use crate::application::swarm::ports::{BoardRepository, Clock};
use crate::domain::swarm::{Access, BoardError};

/// Through the operation gate (`active=False`): the member's row under the
/// reservation must be alive (`unknown_member_status_is_not_alive`,
/// #2295), and a process already recorded must be this one. No event. The
/// row is found in SQL with the caller's member and reservation bound as
/// given (a NULL reservation finds nothing), and the process is compared
/// by Python's `==`.
pub struct RecordMemberLaunch {
    repository: Arc<dyn BoardRepository>,
    clock: Arc<dyn Clock + Send + Sync>,
}

impl RecordMemberLaunch {
    pub fn new(repository: Arc<dyn BoardRepository>, clock: Arc<dyn Clock + Send + Sync>) -> Self {
        Self { repository, clock }
    }

    /// # Errors
    /// An authorisation refusal, `stale launch reservation`, `conflicting
    /// launch identity`, or the store's.
    pub fn execute(&self, request: RecordMemberLaunchRequest) -> Result<(), BoardError> {
        let member = &request.member;
        operation(
            &*self.repository,
            &*self.clock,
            &request.actor,
            Access::default(),
            |transaction, _run| {
                let row = transaction.member_row(member, Some(&request.reservation))?;
                let Some(row) = row.filter(alive) else {
                    return Err(BoardError::new("stale launch reservation"));
                };
                if launched_elsewhere(&row, &request.launch) {
                    return Err(BoardError::new("conflicting launch identity"));
                }
                transaction.record_launch(member, &request.launch)
            },
        )
    }
}

#[cfg(test)]
#[path = "record_member_launch_tests.rs"]
mod tests;
