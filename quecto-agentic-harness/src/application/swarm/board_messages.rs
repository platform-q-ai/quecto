//! What `send`, `withdraw` and `ack` share (#2276): `Workbench._message_id`
//! and `Workbench._retire`. Capability-internal helpers, not a use case and
//! not a port.
use serde_json::Value;

use super::ports::{BoardEncoding, BoardMessages};
use crate::domain::swarm::{BoardError, RefusalKind, python_equal};

/// The most unread messages an inbox holds (`send` and `_notify_revoked`).
pub(crate) const INBOX_CAPACITY: i64 = 100;

/// A message's status once its recipient has read it.
pub(crate) const CONSUMED: &str = "consumed";
/// An unread message's status.
pub(crate) const ACCEPTED: &str = "accepted";

/// Python's `type(value) is int and value >= 1`: a JSON integer of at
/// least 1, never a boolean or a float.
pub(crate) fn is_message_id(value: &Value) -> bool {
    match value {
        Value::Number(number) => {
            number.as_u64().is_some_and(|id| id >= 1) || number.as_i64().is_some_and(|id| id >= 1)
        }
        Value::Null | Value::Bool(_) | Value::String(_) | Value::Array(_) | Value::Object(_) => {
            false
        }
    }
}

/// `Workbench._message_id(value)`: the id, checked before the board is
/// read.
///
/// # Errors
/// `message id must be a positive integer`.
pub(crate) fn message_id(value: &Value) -> Result<&Value, BoardError> {
    if is_message_id(value) {
        return Ok(value);
    }
    Err(BoardError::new(
        RefusalKind::Invalid,
        "message id must be a positive integer",
    ))
}

/// Which retirement `_retire` makes, and the status it writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Retirement {
    /// `send(..., supersedes=id)`: the caller's earlier message to the
    /// same recipient is `superseded`.
    Superseded,
    /// `withdraw(id)`: the caller's message is `withdrawn`.
    Withdrawn,
}

impl Retirement {
    fn status(self) -> &'static str {
        match self {
            Self::Superseded => "superseded",
            Self::Withdrawn => "withdrawn",
        }
    }
}

/// `Workbench._retire(db, message_id, recipient, status)` for `actor`:
/// the caller's own message (addressed to `recipient`, when one is given,
/// by Python's `==`) moves from `accepted` to the retirement's status.
/// Whether it changed: a withdrawal leaves a message already withdrawn as
/// it is.
///
/// # Errors
/// `only your own message can be {status}`, `only a message to the same
/// recipient can be {status}`, `message {id} is already {status}` (the
/// stored status as Python's `str()` writes it), or the store's.
pub(crate) fn retire(
    transaction: &(impl BoardMessages + ?Sized),
    encoding: &dyn BoardEncoding,
    actor: &str,
    id: &Value,
    recipient: Option<&Value>,
    retirement: Retirement,
) -> Result<bool, BoardError> {
    debug_assert!(is_message_id(id), "the id was checked before the board");
    let status = retirement.status();
    let row = transaction.message(id)?;
    let Some(row) = row.filter(|row| row.get("sender").and_then(Value::as_str) == Some(actor))
    else {
        return Err(BoardError::new(
            RefusalKind::NotOwner,
            format!("only your own message can be {status}"),
        ));
    };
    let addressed = |recipient: &Value| {
        row.get("recipient")
            .is_some_and(|stored| python_equal(stored, recipient))
    };
    if recipient.is_some_and(|recipient| !addressed(recipient)) {
        return Err(BoardError::new(
            RefusalKind::WrongState,
            format!("only a message to the same recipient can be {status}"),
        ));
    }
    let stored = row.get("status").cloned().unwrap_or(Value::Null);
    match (stored.as_str(), retirement) {
        (Some("withdrawn"), Retirement::Withdrawn) => return Ok(false),
        (Some(ACCEPTED), _) => {}
        _ => {
            return Err(BoardError::new(
                RefusalKind::WrongState,
                format!("message {id} is already {}", python_str(&stored, encoding)?),
            ));
        }
    }
    transaction.set_message_status(id, status)?;
    Ok(true)
}

/// Python's `str()` of a stored column value: `None` for NULL, text as it
/// is, and a number as the board codec writes it (`repr`'s text). A cell
/// holds nothing else.
fn python_str(value: &Value, encoding: &dyn BoardEncoding) -> Result<String, BoardError> {
    debug_assert!(
        !matches!(value, Value::Bool(_) | Value::Array(_) | Value::Object(_)),
        "a stored cell is NULL, text or a number"
    );
    match value {
        Value::Null => Ok("None".to_owned()),
        Value::String(text) => Ok(text.clone()),
        Value::Number(_) | Value::Bool(_) | Value::Array(_) | Value::Object(_) => {
            encoding.encode(value)
        }
    }
}

#[cfg(test)]
#[path = "board_messages_tests.rs"]
mod tests;
