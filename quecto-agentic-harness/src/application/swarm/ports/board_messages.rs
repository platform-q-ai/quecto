//! The coordination board's messages.

use crate::application::swarm::dto::{MessageRow, MessageTally, NewMessage};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `messages` rows (#2275, #2276): those revocation writes, and the
/// durable messages `send`, `withdraw`, `inbox` and `ack` read and write.
/// A recipient, a message id or the inbox flag is the caller's (or the
/// board's) value, bound as Python binds it; a value it cannot bind is
/// refused naming its parameter.
pub trait BoardMessages {
    /// `SELECT count(*) FROM messages WHERE recipient=? AND
    /// status='accepted'`: the recipient's unread messages.
    fn inbox_count(&self, recipient: &Value) -> Result<i64, BoardError>;
    /// An `accepted` message from `sender`: its id.
    fn insert_message(
        &self,
        sender: &str,
        recipient: &Value,
        body: &str,
    ) -> Result<i64, BoardError>;
    /// `SELECT * FROM messages WHERE id=?` as `dict(row)`.
    fn message(&self, id: &Value) -> Result<Option<MessageRow>, BoardError>;
    /// `SELECT * FROM messages WHERE id=? AND recipient=?` as `dict(row)`:
    /// the message when it is addressed to `recipient`.
    fn addressed_message(
        &self,
        id: &Value,
        recipient: &str,
    ) -> Result<Option<MessageRow>, BoardError>;
    /// `UPDATE messages SET status=? WHERE id=?`, for a message the caller
    /// has read in this transaction.
    fn set_message_status(&self, id: &Value, status: &str) -> Result<(), BoardError>;
    /// The messages by what became of them (#2313 review M1): `count(*)`,
    /// and the rows whose status is `consumed` or `withdrawn`.
    fn message_tally(&self) -> Result<MessageTally, BoardError>;
    /// `send`'s `INSERT INTO messages(sender,recipient,body,status,
    /// revision,supersedes) VALUES(?,?,?,'accepted',?,?)`: its id.
    fn send_message(&self, message: &NewMessage) -> Result<i64, BoardError>;
    /// `UPDATE messages SET superseded_by=? WHERE id=?`: the message `id`
    /// the caller has just retired is superseded by `successor`.
    fn set_superseded_by(&self, id: &Value, successor: i64) -> Result<(), BoardError>;
    /// `SELECT * FROM messages WHERE recipient=? AND (status='accepted' OR
    /// ?) ORDER BY id LIMIT 100`, each row as `dict(row)`: the flag is
    /// bound as given, so SQLite's truth of it decides.
    fn inbox(
        &self,
        recipient: &str,
        include_consumed: &Value,
    ) -> Result<Vec<MessageRow>, BoardError>;
}
