//! `BoardMessages` over the SQLite store (#2275, #2276): the messages
//! revocation writes and the durable messages `send`, `withdraw`, `inbox`
//! and `ack` read and write, by Python's SQL (`swarm.py`, `swarm_tasks.py`)
//! verbatim. A recipient, a message id or the inbox flag is the caller's
//! (or the board's) value, bound as Python's `sqlite3` binds it ([`loose`])
//! and numbered by its position in Python's statement, so the column
//! affinity finds and stores rows as Python's board does, and a value
//! Python cannot bind is refused with its text.
use rusqlite::types::Value as SqlValue;
use serde_json::Value;

use super::repository::{SqliteBoard, loose};
use crate::application::swarm::dto::{MessageRow, NewMessage};
use crate::application::swarm::ports::BoardMessages;
use crate::domain::swarm::{BoardError, RefusalKind};

impl BoardMessages for SqliteBoard<'_> {
    fn inbox_count(&self, recipient: &Value) -> Result<i64, BoardError> {
        self.bound_count(
            "SELECT count(*) FROM messages WHERE recipient=? AND status='accepted'",
            &[loose(1, recipient)?],
        )
    }

    fn insert_message(
        &self,
        sender: &str,
        recipient: &Value,
        body: &str,
    ) -> Result<i64, BoardError> {
        let inserted = self.run(
            "INSERT INTO messages(sender,recipient,body,status) VALUES(?,?,?,'accepted')",
            &[
                SqlValue::Text(sender.to_owned()),
                loose(2, recipient)?,
                SqlValue::Text(body.to_owned()),
            ],
        )?;
        debug_assert_eq!(inserted, 1, "one VALUES row inserts one message");
        Ok(self.connection.last_insert_rowid())
    }

    fn message(&self, _id: &Value) -> Result<Option<MessageRow>, BoardError> {
        Err(unported())
    }

    fn addressed_message(
        &self,
        _id: &Value,
        _recipient: &str,
    ) -> Result<Option<MessageRow>, BoardError> {
        Err(unported())
    }

    fn set_message_status(&self, _id: &Value, _status: &str) -> Result<(), BoardError> {
        Err(unported())
    }

    fn send_message(&self, _message: &NewMessage) -> Result<i64, BoardError> {
        Err(unported())
    }

    fn set_superseded_by(&self, _id: &Value, _successor: i64) -> Result<(), BoardError> {
        Err(unported())
    }

    fn inbox(
        &self,
        _recipient: &str,
        _include_consumed: &Value,
    ) -> Result<Vec<MessageRow>, BoardError> {
        Err(unported())
    }
}

fn unported() -> BoardError {
    BoardError::new(RefusalKind::Internal, "not yet ported (#2276)")
}
