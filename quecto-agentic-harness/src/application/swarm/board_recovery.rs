//! What `recover` and `revoke` share (#2275): `Tasks._reopen` and
//! `Tasks._notify_revoked`. Capability-internal helpers, not a use case
//! and not a port.
#![allow(dead_code)] // The red phase: #2275's use cases call these once served.
use serde_json::Value;

use super::ports::{
    BoardEncoding, BoardEvents, BoardFiles, BoardMembers, BoardMessages, BoardTasks,
};
use crate::domain::swarm::BoardError;

/// The revocation a previous owner is told of.
pub(crate) struct Notice<'a> {
    /// The coordinator revoking, who sends the message.
    pub(crate) actor: &'a str,
    pub(crate) now: f64,
    /// The task's owner before the revocation, as stored.
    pub(crate) previous: &'a Value,
    /// The task id as the caller gave it.
    pub(crate) task_id: &'a Value,
    pub(crate) reason: &'a str,
}

pub(crate) fn reopen(
    transaction: &(impl BoardFiles + BoardTasks + ?Sized),
    task_id: &Value,
) -> Result<(), BoardError> {
    let _ = (transaction, task_id);
    Err(BoardError::new("reopen is not served yet"))
}

pub(crate) fn notify_revoked(
    transaction: &(impl BoardMembers + BoardMessages + BoardEvents + ?Sized),
    encoding: &dyn BoardEncoding,
    notice: &Notice<'_>,
) -> Result<bool, BoardError> {
    let _ = (
        transaction,
        encoding,
        notice.actor,
        notice.now,
        notice.previous,
    );
    let _ = (notice.task_id, notice.reason);
    Err(BoardError::new("notify_revoked is not served yet"))
}

#[cfg(test)]
#[path = "board_recovery_tests.rs"]
mod tests;
