//! `BoardTasks`, `BoardRequests` and `BoardFiles` over the SQLite store
//! (#2272): the `tasks` rows, the request ledger and a claim's file
//! reservations, by Python's SQL (`swarm_tasks.py`, `swarm_store.py`). A
//! task id, a claim token or a file's task is the caller's value, bound as
//! Python's `sqlite3` binds it ([`loose`]), so the column affinity finds
//! rows as Python's board does.
//!
//! A task row is fetched as Python fetches it (every column decoded, text
//! that is not UTF-8 refused with Python's text), and its `acceptance`,
//! `dependencies` and `evidence` are loaded with the Python-compatible
//! codec. A JSON column the board never writes (not text, or not JSON) is
//! refused as a store failure, where Python raises a `TypeError` or a
//! `JSONDecodeError`: the `outside_edited_task_columns` divergence.
use rusqlite::types::{Type, Value as SqlValue};
use rusqlite::{Row, params};
use serde_json::Value;

use super::binding;
use super::ledger;
use super::py_json::{self, PyJson};
use super::repository::{SqliteBoard, cell_at, encoded, failed, fetched, loose, refused, text};
use super::store::TransactionError;
use crate::application::swarm::dto::{NewTask, TaskRow, TaskUpdate};
use crate::application::swarm::ports::{BoardFiles, BoardRequests, BoardTasks, RequestAction};
use crate::domain::swarm::{BoardError, RefusalKind};

/// The task columns `_task` loads from JSON.
const JSON_COLUMNS: [&str; 3] = ["acceptance", "dependencies", "evidence"];

impl BoardTasks for SqliteBoard<'_> {
    fn task(&self, id: &Value) -> Result<Option<TaskRow>, BoardError> {
        let parameters = [loose(1, id)?];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT * FROM tasks WHERE id=?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        rows.next()
            .and_then(|row| row.map(task_row).transpose())
            .map_err(failed)
    }

    fn task_status(&self, id: &Value) -> Result<Option<String>, BoardError> {
        let parameters = [loose(1, id)?];
        let mut statement = binding::bound_statement(
            self.connection,
            "SELECT status FROM tasks WHERE id=?",
            &parameters,
        )
        .map_err(failed)?;
        let mut rows = statement.raw_query();
        let status = rows
            .next()
            .and_then(|row| {
                row.map(|row| {
                    fetched(row)?;
                    text(row, "status")
                })
                .transpose()
            })
            .map_err(failed)?;
        Ok(status.flatten())
    }

    fn all_task_dependencies(&self) -> Result<Vec<(i64, Value)>, BoardError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM tasks")
            .map_err(failed)?;
        let rows = statement
            .query_map([], |row| {
                fetched(row)?;
                let index = row.as_ref().column_index("dependencies")?;
                Ok((row.get("id")?, loaded(row, index)?))
            })
            .map_err(failed)?;
        rows.collect::<rusqlite::Result<_>>().map_err(failed)
    }

    fn task_count(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM tasks")
    }

    fn insert_task(&self, task: &NewTask) -> Result<i64, BoardError> {
        let inserted = self
            .connection
            .execute(
                "INSERT INTO tasks(title,acceptance,dependencies,status,evidence) VALUES(?,?,?,'ready','[]')",
                params![
                    task.title,
                    encoded(&task.acceptance)?,
                    encoded(&task.dependencies)?,
                ],
            )
            .map_err(failed)?;
        debug_assert_eq!(inserted, 1, "one VALUES row inserts one task");
        Ok(self.connection.last_insert_rowid())
    }

    fn set_task_dependencies(&self, id: &Value, dependencies: &Value) -> Result<(), BoardError> {
        self.run_on_task(
            "UPDATE tasks SET dependencies=? WHERE id=?",
            &[encoded(dependencies)?, loose(2, id)?],
        )
    }

    fn update_task_claim(&self, id: &Value, owner: &str, token: &str) -> Result<(), BoardError> {
        self.run_on_task(
            "UPDATE tasks SET status='claimed',owner=?,token=? WHERE id=?",
            &[
                SqlValue::Text(owner.to_owned()),
                SqlValue::Text(token.to_owned()),
                loose(3, id)?,
            ],
        )
    }

    fn update_task_status(&self, id: &Value, update: &TaskUpdate) -> Result<(), BoardError> {
        match update {
            TaskUpdate::Release => self.run_on_task(
                "UPDATE tasks SET status='ready',owner=NULL,token=NULL,blocker=NULL WHERE id=?",
                &[loose(1, id)?],
            ),
            TaskUpdate::Block { reason } => self.run_on_task(
                "UPDATE tasks SET status='blocked',blocker=? WHERE id=?",
                &[SqlValue::Text(reason.clone()), loose(2, id)?],
            ),
            TaskUpdate::Unblock => self.run_on_task(
                "UPDATE tasks SET status='claimed',blocker=NULL WHERE id=?",
                &[loose(1, id)?],
            ),
            TaskUpdate::Submit { evidence } => self.run_on_task(
                "UPDATE tasks SET status='submitted',evidence=?,blocker=NULL WHERE id=?",
                &[encoded(evidence)?, loose(2, id)?],
            ),
            TaskUpdate::Complete => self.run_on_task(
                "UPDATE tasks SET status='completed' WHERE id=?",
                &[loose(1, id)?],
            ),
        }
    }
}

impl BoardRequests for SqliteBoard<'_> {
    fn retry(
        &self,
        actor: &str,
        request: &str,
        payload: &Value,
        action: &mut RequestAction<'_>,
    ) -> Result<Value, BoardError> {
        let payload = PyJson::try_from(payload)
            .map_err(|error| BoardError::new(RefusalKind::Invalid, error.to_string()))?;
        let answer = ledger::retry(self.connection, actor, request, &payload, || {
            let result = action().map_err(TransactionError::Board)?;
            PyJson::try_from(&result).map_err(|error| {
                TransactionError::Board(BoardError::new(RefusalKind::Invalid, error.to_string()))
            })
        })
        .map_err(refused)?;
        answer
            .to_value()
            .map_err(|stored| BoardError::new(RefusalKind::Store, stored.to_string()))
    }
}

impl BoardFiles for SqliteBoard<'_> {
    fn delete_claim_files(&self, task: &Value, claim: &Value) -> Result<(), BoardError> {
        // Any number of reservations, none included.
        self.run(
            "DELETE FROM files WHERE task=? AND claim=?",
            &[loose(1, task)?, loose(2, claim)?],
        )
        .map(|_| ())
    }
}

impl SqliteBoard<'_> {
    /// One statement with its parameters already bound as Python binds
    /// them: the number of rows it changed.
    pub(super) fn run(&self, sql: &str, parameters: &[SqlValue]) -> Result<usize, BoardError> {
        binding::bound_statement(self.connection, sql, parameters)
            .and_then(|mut statement| statement.raw_execute())
            .map_err(failed)
    }

    /// An `UPDATE` of a task id the use case has just read in this
    /// transaction. A hand-edited table without a primary key may hold
    /// duplicate ids, so an update can change multiple rows.
    pub(super) fn run_on_task(&self, sql: &str, parameters: &[SqlValue]) -> Result<(), BoardError> {
        debug_assert!(sql.starts_with("UPDATE tasks SET "), "a task update: {sql}");
        let changed = self.run(sql, parameters)?;
        debug_assert!(changed >= 1, "{sql} changes a task read before it");
        Ok(())
    }
}

/// `dict(row)` of a `SELECT * FROM tasks` row, its JSON columns loaded.
pub(super) fn task_row(row: &Row<'_>) -> rusqlite::Result<TaskRow> {
    fetched(row)?;
    let columns = (0..row.as_ref().column_count())
        .map(|index| {
            let name = row.as_ref().column_name(index)?.to_owned();
            let value = if JSON_COLUMNS.contains(&name.as_str()) {
                loaded(row, index)?
            } else {
                cell_at(row, index)?
            };
            Ok((name, value))
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(TaskRow { columns })
}

/// `json.loads(row[column])` of the column at `index`, which the board
/// writes as JSON text.
pub(super) fn loaded(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    let stored = row.get_ref(index)?;
    let text = stored.as_str().map_err(|_| {
        rusqlite::Error::InvalidColumnType(
            index,
            row.as_ref().column_name(index).unwrap_or("?").to_owned(),
            stored.data_type(),
        )
    })?;
    py_json::decode(text)
        .and_then(|value| value.to_value())
        .map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(index, Type::Text, Box::new(error))
        })
}
