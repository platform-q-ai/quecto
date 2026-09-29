//! `BoardWakes` over the SQLite store (#2276): the notification and wake
//! cursors and what the wake policy reads, by Python's SQL
//! (`swarm_repository.py`) verbatim. `wake_cursors` is not part of the
//! schema: [`BoardWakes::create_wake_cursors`] creates it where Python's
//! `claim_wake_events` does, inside the claim's transaction.
//!
//! An event detail is loaded as Python's `json.loads` loads it; a detail
//! that is not JSON text, or a wake cursor that is not an integer (only a
//! hand edit writes either), is refused as a store failure (the
//! `outside_edited_wake_records` divergence). Python raises for such a
//! detail and for a TEXT or NULL wake cursor, but compares and binds a
//! REAL one as it is, and claims past it. A notification cursor is bound
//! back as it is stored, as Python binds `cursor[0]`.
use std::collections::BTreeSet;

use rusqlite::types::Value as SqlValue;
use rusqlite::{OptionalExtension, Row, params};
use serde_json::Value;

use super::binding;
use super::repository::{SqliteBoard, failed, fetched, text};
use super::repository_tasks::loaded;
use crate::application::swarm::dto::NotificationCursor;
use crate::application::swarm::ports::BoardWakes;
use crate::domain::swarm::{
    BoardError, NotificationEvent, NotificationState, TaskState, TaskSummary,
};

/// `claim_wake_events`' statement, verbatim.
const WAKE_CURSORS: &str =
    "CREATE TABLE IF NOT EXISTS wake_cursors (actor TEXT PRIMARY KEY, event INTEGER)";

impl BoardWakes for SqliteBoard<'_> {
    fn notification_events(&self, actor: &str) -> Result<Vec<NotificationEvent>, BoardError> {
        let cursor = self
            .connection
            .query_row(
                "SELECT event FROM notification_cursors WHERE actor=?",
                [actor],
                |row| row.get::<_, SqlValue>(0),
            )
            .optional()
            .map_err(failed)?
            .unwrap_or(SqlValue::Integer(0));
        let mut events = self.events(
            "SELECT action,detail FROM events WHERE actor=? AND id>? ORDER BY id",
            &[SqlValue::Text(actor.to_owned()), cursor],
        )?;
        for event in &mut events {
            event.actor = Some(actor.to_owned());
        }
        Ok(events)
    }

    fn advance_notifications(&self, actor: &str) -> Result<NotificationCursor, BoardError> {
        let previous = self
            .connection
            .query_row(
                "SELECT event FROM notification_cursors WHERE actor=?",
                [actor],
                |row| row.get::<_, SqlValue>(0),
            )
            .optional()
            .map_err(failed)?;
        let written = self
            .connection
            .execute(
                "INSERT OR REPLACE INTO notification_cursors VALUES(?,(SELECT coalesce(max(id),0) FROM events))",
                [actor],
            )
            .map_err(failed)?;
        debug_assert_eq!(written, 1, "one cursor row is written");
        let current = self
            .connection
            .query_row(
                "SELECT event FROM notification_cursors WHERE actor=?",
                [actor],
                |row| row.get::<_, i64>(0),
            )
            .map_err(failed)?;
        Ok(NotificationCursor {
            previous: previous.and_then(|value| match value {
                SqlValue::Integer(event) => Some(event),
                SqlValue::Null | SqlValue::Real(_) | SqlValue::Text(_) | SqlValue::Blob(_) => None,
            }),
            current,
        })
    }

    fn notification_state(&self) -> Result<NotificationState, BoardError> {
        let mut statement = self
            .connection
            .prepare("SELECT id,status,dependencies,owner FROM tasks")
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                fetched(row)?;
                let dependencies = task_ids(&loaded(row, 2)?);
                // A status that is not text (only a hand edit writes one)
                // equals no status, as Python's does: the policy judges
                // such a task as one the board does not hold.
                let Some(status) = text(row, "status")? else {
                    return Ok(None);
                };
                Ok(Some(TaskSummary {
                    id: row.get(0)?,
                    status: TaskState::new(status),
                    dependencies,
                    owner: text(row, "owner")?,
                }))
            })
            .map_err(failed)?;
        let tasks = rows
            .filter_map(Result::transpose)
            .collect::<rusqlite::Result<Vec<_>>>()
            .map_err(failed)?;
        let unread = self
            .connection
            .prepare("SELECT id,recipient FROM messages WHERE status='accepted'")
            .and_then(|mut statement| {
                statement
                    .query_map([], |row| {
                        fetched(row)?;
                        row.get::<_, i64>(0)
                    })?
                    .collect::<rusqlite::Result<BTreeSet<i64>>>()
            })
            .map_err(failed)?;
        Ok(NotificationState { tasks, unread })
    }

    fn create_wake_cursors(&self) -> Result<(), BoardError> {
        self.connection
            .execute(WAKE_CURSORS, [])
            .map(|_| ())
            .map_err(failed)
    }

    fn event_generation(&self) -> Result<i64, BoardError> {
        self.count("SELECT coalesce(max(id),0) FROM events")
    }

    /// The caller's wake cursor: an integer, or refused as a store failure
    /// (`Invalid column type`) where Python raises on TEXT or NULL and
    /// uses a REAL as it is (`outside_edited_wake_records`).
    fn wake_cursor(&self, actor: &str) -> Result<Option<i64>, BoardError> {
        self.connection
            .query_row(
                "SELECT event FROM wake_cursors WHERE actor=?",
                [actor],
                |row| row.get::<_, i64>(0),
            )
            .optional()
            .map_err(failed)
    }

    fn wake_events(
        &self,
        previous: i64,
        generation: i64,
        actor: &str,
    ) -> Result<Vec<NotificationEvent>, BoardError> {
        debug_assert!(previous >= 0, "a cursor is an event id or 0");
        self.events(
            "SELECT action,detail,actor FROM events WHERE id>? AND id<=? AND actor<>? ORDER BY id",
            &[
                SqlValue::Integer(previous),
                SqlValue::Integer(generation),
                SqlValue::Text(actor.to_owned()),
            ],
        )
    }

    fn set_wake_cursor(&self, actor: &str, generation: i64) -> Result<(), BoardError> {
        let written = self
            .connection
            .execute(
                "INSERT OR REPLACE INTO wake_cursors VALUES(?,?)",
                params![actor, generation],
            )
            .map_err(failed)?;
        debug_assert_eq!(written, 1, "one cursor row is written");
        Ok(())
    }
}

impl SqliteBoard<'_> {
    /// The events `sql` selects (`action`, `detail` and, when selected,
    /// `actor`), each detail as `json.loads` reads it.
    fn events(
        &self,
        sql: &str,
        parameters: &[SqlValue],
    ) -> Result<Vec<NotificationEvent>, BoardError> {
        debug_assert!(
            sql.starts_with("SELECT action,detail"),
            "an event read: {sql}"
        );
        let mut statement =
            binding::bound_statement(self.connection, sql, parameters).map_err(failed)?;
        let mut rows = statement.raw_query();
        let mut events = Vec::new();
        while let Some(row) = rows.next().map_err(failed)? {
            events.push(event(row).map_err(failed)?);
        }
        Ok(events)
    }
}

/// One selected event row: an action that is not text is none (it equals
/// no action the policy names, as Python's `None` does), and an actor that
/// is not text is no actor.
fn event(row: &Row<'_>) -> rusqlite::Result<NotificationEvent> {
    fetched(row)?;
    let detail = loaded(row, 1)?;
    let actor = match row.as_ref().column_count() {
        3 => text(row, "actor")?,
        _ => None,
    };
    Ok(NotificationEvent {
        action: text(row, "action")?.unwrap_or_default(),
        detail,
        actor,
    })
}

/// A loaded dependency list's task ids: its integer entries (the board
/// writes only those); anything else names no task.
fn task_ids(dependencies: &Value) -> Vec<i64> {
    dependencies
        .as_array()
        .map(|entries| entries.iter().filter_map(Value::as_i64).collect())
        .unwrap_or_default()
}
