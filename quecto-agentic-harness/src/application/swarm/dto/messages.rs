//! Durable messages (#2276): the requests and answers of `SendMessage`,
//! `WithdrawMessage`, `ReadInbox` and `AcknowledgeMessage`, and the rows
//! the board ports read and write for them.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and every
//! other argument stays the JSON value the caller passed: Python binds a
//! recipient and an inbox flag untyped and type-checks the rest at run
//! time.
use serde_json::{Map, Value};

/// `Workbench.send(request, recipient, body, revision=None,
/// supersedes=None)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SendMessageRequest {
    pub actor: String,
    pub request: Value,
    pub recipient: Value,
    pub body: Value,
    pub revision: Value,
    pub supersedes: Value,
}

/// What `send` answered: the receipt the ledger holds (`{id, status}` as
/// first written, or as replayed), and whether this call wrote it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SentMessage {
    pub receipt: Value,
    pub sent: bool,
}

/// `Workbench.withdraw(message_id)` or `Workbench.ack(message_id)` as
/// `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageIdRequest {
    pub actor: String,
    pub message_id: Value,
}

/// What `withdraw` or `ack` did to the message `message_id` (a positive
/// integer): whether it changed its status.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Settled {
    pub message_id: Value,
    pub changed: bool,
}

/// `Workbench.inbox(include_consumed=False)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReadInboxRequest {
    pub actor: String,
    pub include_consumed: Value,
}

/// A `messages` row as Python's `dict(row)`: every column, in table order
/// (the migrated `revision`, `supersedes` and `superseded_by` included).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MessageRow {
    pub columns: Vec<(String, Value)>,
}

impl MessageRow {
    /// The value stored in `column`, when the row has that column.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    /// The row as the JSON object Python's `dict(row)` renders.
    pub fn into_value(self) -> Value {
        Value::Object(self.columns.into_iter().collect::<Map<String, Value>>())
    }
}

/// An `accepted` message `send` inserts: the recipient as the caller gave
/// it, the revision when one is named, and the id of the caller's own
/// message it supersedes, as the caller gave it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMessage {
    pub sender: String,
    pub recipient: Value,
    pub body: String,
    pub revision: Option<String>,
    pub supersedes: Option<Value>,
}
