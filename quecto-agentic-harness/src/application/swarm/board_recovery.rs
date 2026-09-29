//! What `recover` and `revoke` share (#2275): `Tasks._reopen` and
//! `Tasks._notify_revoked`. Capability-internal helpers, not a use case
//! and not a port.
use serde_json::Value;

use super::board_operation::detail;
use super::dto::TaskUpdate;
use super::ports::{
    BoardEncoding, BoardEvents, BoardFiles, BoardMembers, BoardMessages, BoardTasks, Clock,
};
use crate::domain::swarm::{BoardError, RefusalKind, status_is_alive};

/// The most unread messages an inbox holds.
const INBOX_CAPACITY: i64 = 100;

/// The revocation a previous owner is told of.
pub(crate) struct Notice<'a> {
    /// The coordinator revoking, who sends the message.
    pub(crate) actor: &'a str,
    /// Read when the `message_accepted` event is written, as Python's
    /// `store.event` reads `self.clock()` for each event.
    pub(crate) clock: &'a dyn Clock,
    /// The task's owner before the revocation, as stored.
    pub(crate) previous: &'a Value,
    /// The task id as the caller gave it.
    pub(crate) task_id: &'a Value,
    pub(crate) reason: &'a str,
}

/// `Tasks._reopen(db, task_id)`: every reservation of the task goes,
/// whichever claim made it, and the task is `ready` without owner, token,
/// blocker or evidence. The caller has read the task in this transaction.
///
/// # Errors
/// The store's.
pub(crate) fn reopen(
    transaction: &(impl BoardFiles + BoardTasks + ?Sized),
    task_id: &Value,
) -> Result<(), BoardError> {
    transaction.delete_task_files(task_id)?;
    transaction.update_task_status(task_id, &TaskUpdate::Reopen)
}

/// `Tasks._notify_revoked`: best effort inside the revocation's
/// transaction. A previous owner whose row is live or reserved (the
/// board's liveness allowlist), and whose inbox holds fewer than a hundred
/// unread messages, gets one accepted message naming the task and the
/// reason, recorded as `message_accepted{message,recipient,revision}`
/// with no revision; any other owner gets nothing, and the revocation
/// stands. Only the owner's status is read (Python's `SELECT status FROM
/// members WHERE id=?`), so what the board never writes in another column
/// of its row does not stop the revocation. Whether the message was
/// written.
///
/// # Errors
/// The store's, or a task id the board cannot write as Python's `str()`.
pub(crate) fn notify_revoked(
    transaction: &(impl BoardMembers + BoardMessages + BoardEvents + ?Sized),
    encoding: &dyn BoardEncoding,
    notice: &Notice<'_>,
) -> Result<bool, BoardError> {
    let reachable = transaction
        .member_status(notice.previous)?
        .is_some_and(|row| status_is_alive(row.status.as_deref()));
    if !reachable {
        return Ok(false);
    }
    if transaction.inbox_count(notice.previous)? >= INBOX_CAPACITY {
        return Ok(false);
    }
    let body = format!(
        "claim on task {} revoked by the coordinator: {}",
        python_str(notice.task_id, encoding)?,
        notice.reason
    );
    let message = transaction.insert_message(notice.actor, notice.previous, &body)?;
    debug_assert!(message > 0, "a message id is a positive rowid");
    transaction.event(
        notice.actor,
        notice.clock.now_seconds(),
        "message_accepted",
        &detail([
            ("message", Value::from(message)),
            ("recipient", notice.previous.clone()),
            ("revision", Value::Null),
        ]),
    )?;
    Ok(true)
}

/// Python's `str()` of a task id that found a task: text as it is, a
/// boolean as `True` or `False`, and a number as the board codec writes
/// it, which is Python's `str()` of it: an integer in decimal (`json.dumps`
/// of an int), a float as `repr` writes it (`1.0`, `1e+16`, `1e-05`).
fn python_str(value: &Value, encoding: &dyn BoardEncoding) -> Result<String, BoardError> {
    match value {
        Value::String(text) => Ok(text.clone()),
        Value::Bool(true) => Ok("True".to_owned()),
        Value::Bool(false) => Ok("False".to_owned()),
        Value::Number(_) => encoding.encode(value),
        Value::Null | Value::Array(_) | Value::Object(_) => Err(BoardError::new(
            RefusalKind::Internal,
            "a task id that finds a task is text or a number",
        )),
    }
}

#[cfg(test)]
#[path = "board_recovery_tests.rs"]
mod tests;
