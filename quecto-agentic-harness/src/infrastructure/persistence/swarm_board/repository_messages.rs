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

use super::binding;
use super::repository::{SqliteBoard, cell_at, failed, fetched, loose};
use crate::application::swarm::dto::{MessageRow, MessageTally, NewMessage};
use crate::application::swarm::ports::BoardMessages;
use crate::domain::swarm::BoardError;

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

    fn message(&self, id: &Value) -> Result<Option<MessageRow>, BoardError> {
        let rows = self.message_rows("SELECT * FROM messages WHERE id=?", &[loose(1, id)?])?;
        Ok(rows.into_iter().next())
    }

    fn addressed_message(
        &self,
        id: &Value,
        recipient: &str,
    ) -> Result<Option<MessageRow>, BoardError> {
        let rows = self.message_rows(
            "SELECT * FROM messages WHERE id=? AND recipient=?",
            &[loose(1, id)?, SqlValue::Text(recipient.to_owned())],
        )?;
        Ok(rows.into_iter().next())
    }

    fn set_message_status(&self, id: &Value, status: &str) -> Result<(), BoardError> {
        let changed = self.run(
            "UPDATE messages SET status=? WHERE id=?",
            &[SqlValue::Text(status.to_owned()), loose(2, id)?],
        )?;
        debug_assert!(changed >= 1, "the message was read in this transaction");
        Ok(())
    }

    fn message_tally(&self) -> Result<MessageTally, BoardError> {
        self.connection
            .query_row(
                "SELECT count(*), coalesce(sum(status='consumed'),0), coalesce(sum(status='withdrawn'),0) FROM messages",
                [],
                |row| {
                    Ok(MessageTally {
                        sent: row.get(0)?,
                        consumed: row.get(1)?,
                        withdrawn: row.get(2)?,
                    })
                },
            )
            .map_err(failed)
    }

    fn send_message(&self, message: &NewMessage) -> Result<i64, BoardError> {
        let supersedes = match &message.supersedes {
            Some(id) => loose(5, id)?,
            None => SqlValue::Null,
        };
        let inserted = self.run(
            "INSERT INTO messages(sender,recipient,body,status,revision,supersedes) VALUES(?,?,?,'accepted',?,?)",
            &[
                SqlValue::Text(message.sender.clone()),
                loose(2, &message.recipient)?,
                SqlValue::Text(message.body.clone()),
                message.revision.clone().map_or(SqlValue::Null, SqlValue::Text),
                supersedes,
            ],
        )?;
        debug_assert_eq!(inserted, 1, "one VALUES row inserts one message");
        Ok(self.connection.last_insert_rowid())
    }

    fn set_superseded_by(&self, id: &Value, successor: i64) -> Result<(), BoardError> {
        let changed = self.run(
            "UPDATE messages SET superseded_by=? WHERE id=?",
            &[SqlValue::Integer(successor), loose(2, id)?],
        )?;
        debug_assert!(changed >= 1, "the message was retired in this transaction");
        Ok(())
    }

    fn inbox(
        &self,
        recipient: &str,
        include_consumed: &Value,
    ) -> Result<Vec<MessageRow>, BoardError> {
        let rows = self.message_rows(
            "SELECT * FROM messages WHERE recipient=? AND (status='accepted' OR ?) ORDER BY id LIMIT 100",
            &[
                SqlValue::Text(recipient.to_owned()),
                loose(2, include_consumed)?,
            ],
        )?;
        debug_assert!(rows.len() <= 100, "an inbox holds at most a hundred");
        Ok(rows)
    }
}

impl SqliteBoard<'_> {
    /// Each `messages` row `sql` selects, with its parameters already bound
    /// as Python binds them, as `dict(row)`: every column, in table order.
    fn message_rows(
        &self,
        sql: &str,
        parameters: &[SqlValue],
    ) -> Result<Vec<MessageRow>, BoardError> {
        debug_assert!(
            sql.starts_with("SELECT * FROM messages "),
            "a message read: {sql}"
        );
        let mut statement =
            binding::bound_statement(self.connection, sql, parameters).map_err(failed)?;
        let mut rows = statement.raw_query();
        let mut found = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            fetched(row).map_err(failed)?;
            let columns = (0..row.as_ref().column_count())
                .map(|index| {
                    Ok((
                        row.as_ref().column_name(index)?.to_owned(),
                        cell_at(row, index)?,
                    ))
                })
                .collect::<rusqlite::Result<Vec<_>>>()
                .map_err(failed)?;
            found.push(MessageRow { columns });
        }
        Ok(found)
    }
}
