//! The board ports over the SQLite store (#2270): `BoardRepository` opens
//! one [`BoardStore`] transaction per `atomic`, and the role ports run
//! Python's SQL (`swarm.py`, `swarm_repository.py`) on its connection.
//!
//! Every SQLite failure is `coordination store unavailable or contended:
//! {sqlite3_errmsg}`, as Python's `Store.transaction` reports it.
//!
//! Rows are fetched as Python's `sqlite3` fetches them where the board's
//! own writes or a file edited outside it leave loose values (#2270 review
//! L5, round-2 L2, round-3 N1/N5, round-4 L1): every row runs the same
//! query Python runs, and text that is not UTF-8 in any column of it is
//! refused with Python's `Could not decode to UTF-8` text ([`fetched`]). A
//! NULL run status or coordinator is `None`; `create` fetches the whole
//! run row and reads a status or coordinator that is not text as `None`,
//! which is never the setup placeholder, as Python's comparison finds; a
//! member row is every column the table has, in table order, as stored
//! (`dict(row)`); `_bootstrap` asks only whether a run exists; and
//! `_status` fetches only the columns it selects. A column `_snapshot` or
//! `_status` reads holding a type the board never writes, or a BLOB, is
//! refused the same way as a store failure, with the driver's conversion
//! error where Python would answer the value (the differential suite's
//! `outside_edited_columns` divergence).
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use super::binding;
use super::ledger;
use super::py_json::{self, PyJson};
use super::store::{BoardStore, CONTENDED, TransactionError, Undecodable, contended};
use crate::application::swarm::dto::{
    BoardLocation, MemberRow, NewRun, RunContract, RunOwnerRow, RunStatusRow,
};
use crate::application::swarm::ports::{BoardEvents, BoardRepository, BoardRuns, BoardWork};
use crate::domain::swarm::{BoardError, RunRecord, RunState};

/// `swarm_repository.ACTIVE_CLAIM`.
pub(super) const ACTIVE_CLAIM: &str = "('claimed','blocked','submitted')";

/// The parameters a refused member value is numbered by (#2270 review M1):
/// its position in Python's `_bootstrap` statement,
/// `INSERT INTO members(id,reservation,status,pid,started,socket)
/// VALUES(?,?,'live',?,?,?)`, where the status is a literal, so `pid`,
/// `started` and `socket` are parameters 3, 4 and 5. Python's `sqlite3`
/// names that position in its refusal; the Rust statement binds the status
/// and the launcher too, so its own positions would name another.
/// `repository_tests.rs` derives them from `swarm.py`'s statement.
///
/// The positions and the refusal texts built on them
/// (`Error binding parameter N: type 'list' is not supported`) are Python
/// 3.12 and later wording, which counts from 1; CI pins Python 3.13.
/// Earlier versions count from 0 and write `Error binding parameter N -
/// probably unsupported type.`
pub(super) const PYTHON_PID_PARAMETER: usize = 3;
pub(super) const PYTHON_STARTED_PARAMETER: usize = 4;
pub(super) const PYTHON_SOCKET_PARAMETER: usize = 5;

/// The board file of one run.
#[derive(Clone, Debug)]
pub struct SqliteBoardRepository {
    store: BoardStore,
}

impl SqliteBoardRepository {
    pub fn new(location: &BoardLocation) -> Self {
        Self {
            store: BoardStore::new(&location.database),
        }
    }
}

impl BoardRepository for SqliteBoardRepository {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        self.store
            .transaction(create, |transaction| {
                let board = SqliteBoard {
                    connection: transaction,
                };
                work(&board).map_err(|refusal| TransactionError::Board(refusal.0))
            })
            .map_err(|refusal| BoardError(refusal.0))
    }
}

/// The role ports over one open transaction.
pub(super) struct SqliteBoard<'c> {
    pub(super) connection: &'c Connection,
}

impl BoardRuns for SqliteBoard<'_> {
    fn run(&self) -> Result<Option<RunRecord>, BoardError> {
        self.connection
            .query_row("SELECT * FROM run", [], run_record)
            .optional()
            .map_err(failed)
    }

    fn run_exists(&self) -> Result<bool, BoardError> {
        self.connection
            .query_row("SELECT 1 FROM run", [], |_| Ok(()))
            .optional()
            .map(|found| found.is_some())
            .map_err(failed)
    }

    fn run_owner(&self) -> Result<Option<RunOwnerRow>, BoardError> {
        self.connection
            .query_row("SELECT * FROM run", [], |row| {
                fetched(row)?;
                Ok(RunOwnerRow {
                    status: text(row, "status")?,
                    coordinator: text(row, "coordinator")?,
                })
            })
            .optional()
            .map_err(failed)
    }

    fn run_status(&self) -> Result<Option<RunStatusRow>, BoardError> {
        self.connection
            .query_row(
                "SELECT id, status, deadline, coordinator, outcome FROM run",
                [],
                |row| {
                    fetched(row)?;
                    Ok(RunStatusRow {
                        id: row.get("id")?,
                        status: row.get("status")?,
                        deadline: cell(row, "deadline")?,
                        coordinator: row.get("coordinator")?,
                        outcome: row.get("outcome")?,
                    })
                },
            )
            .optional()
            .map_err(failed)
    }

    fn run_coordinator(&self) -> Result<Option<Option<String>>, BoardError> {
        Err(BoardError::new("not implemented (#2271)"))
    }

    fn insert_run(&self, run: &NewRun) -> Result<(), BoardError> {
        let contract = &run.contract;
        self.connection
            .execute(
                "INSERT INTO run(id,goal,constraints,criteria,coordinator,integrator,member_limit,deadline,status) VALUES(?,?,?,?,?,?,?,?,?)",
                params![
                    run.id,
                    contract.goal,
                    encoded(&contract.constraints)?,
                    encoded(&contract.criteria)?,
                    run.coordinator,
                    run.integrator,
                    contract.member_limit,
                    contract.deadline,
                    run.status.as_str(),
                ],
            )
            .map_err(failed)?;
        Ok(())
    }

    fn update_run_contract(&self, contract: &RunContract) -> Result<(), BoardError> {
        self.connection
            .execute(
                "UPDATE run SET goal=?,constraints=?,criteria=?,member_limit=?,deadline=?,status=?",
                params![
                    contract.goal,
                    encoded(&contract.constraints)?,
                    encoded(&contract.criteria)?,
                    contract.member_limit,
                    contract.deadline,
                    RunState::RUNNING.as_str(),
                ],
            )
            .map_err(failed)?;
        Ok(())
    }

    fn propose_outcome(&self, outcome: &str, reason: &str) -> Result<(), BoardError> {
        self.connection
            .execute(
                "UPDATE run SET status='paused', outcome=?, outcome_reason=?",
                params![outcome, reason],
            )
            .map_err(failed)?;
        Ok(())
    }
}

impl BoardEvents for SqliteBoard<'_> {
    fn event(
        &self,
        actor: &str,
        time: f64,
        action: &str,
        detail: &Value,
    ) -> Result<(), BoardError> {
        let detail =
            PyJson::try_from(detail).map_err(|error| BoardError::new(error.to_string()))?;
        ledger::event(self.connection, actor, time, action, &detail).map_err(refused)
    }

    fn control_generation(&self) -> Result<i64, BoardError> {
        self.count("SELECT coalesce(max(id),0) FROM events WHERE action IN ('paused','resumed')")
    }
}

impl SqliteBoard<'_> {
    pub(super) fn count(&self, sql: &str) -> Result<i64, BoardError> {
        self.connection
            .query_row(sql, [], |row| row.get(0))
            .map_err(failed)
    }
}

/// A member's JSON value bound as Python's `sqlite3` binds it (epic P3):
/// `True` as 1, a float as REAL, text as TEXT, and the column's affinity
/// decides what is stored. A value Python cannot bind is refused naming
/// Python's parameter `position`: a list or an object with Python's
/// `ProgrammingError` text, and an integer beyond i64, which Python raises
/// as `OverflowError` rather than refuses, with its text (the
/// `integer_beyond_i64_is_refused` divergence).
pub(super) fn loose(position: usize, value: &Value) -> Result<SqlValue, BoardError> {
    PyJson::try_from(value)
        .map_err(|error| error.to_string())
        .and_then(|value| binding::bind(&value).map_err(|error| error.to_string()))
        .map_err(|error| {
            BoardError(format!(
                "{CONTENDED}: Error binding parameter {position}: {error}"
            ))
        })
}

/// Python's fetch of `row`: every column is decoded, so the first TEXT
/// that is not UTF-8 is refused with Python's text ([`Undecodable`]).
pub(super) fn fetched(row: &Row<'_>) -> rusqlite::Result<()> {
    use rusqlite::types::ValueRef;
    for index in 0..row.as_ref().column_count() {
        match row.get_ref(index)? {
            ValueRef::Text(bytes) => decoded(row, index, bytes).map(|_| ())?,
            ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) | ValueRef::Blob(_) => {}
        }
    }
    Ok(())
}

/// A TEXT column's value, or Python's refusal of it.
fn decoded<'r>(row: &Row<'_>, index: usize, bytes: &'r [u8]) -> rusqlite::Result<&'r str> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(_) => Err(rusqlite::Error::FromSqlConversionFailure(
            index,
            rusqlite::types::Type::Text,
            Box::new(Undecodable::new(row.as_ref().column_name(index)?, bytes)),
        )),
    }
}

/// The text `name` holds, `None` for any other storage class: Python
/// compares it with a string, which a number, bytes or `None` never equals.
fn text(row: &Row<'_>, name: &str) -> rusqlite::Result<Option<String>> {
    use rusqlite::types::ValueRef;
    let index = row.as_ref().column_index(name)?;
    match row.get_ref(index)? {
        ValueRef::Text(bytes) => decoded(row, index, bytes).map(|text| Some(text.to_owned())),
        ValueRef::Null | ValueRef::Integer(_) | ValueRef::Real(_) | ValueRef::Blob(_) => Ok(None),
    }
}

/// A column as the JSON value of what it stores: NULL, an INTEGER, a
/// finite REAL or TEXT. A BLOB, or a REAL JSON cannot write, is a
/// conversion error; TEXT that is not UTF-8 is Python's refusal.
fn cell(row: &Row<'_>, name: &str) -> rusqlite::Result<Value> {
    cell_at(row, row.as_ref().column_index(name)?)
}

/// The column at `index` as [`cell`] reads it.
fn cell_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    use rusqlite::types::ValueRef;
    let name = row.as_ref().column_name(index)?;
    let invalid = |kind: rusqlite::types::Type| {
        rusqlite::Error::InvalidColumnType(index, name.to_owned(), kind)
    };
    match row.get_ref(index)? {
        ValueRef::Null => Ok(Value::Null),
        ValueRef::Integer(integer) => Ok(Value::from(integer)),
        ValueRef::Real(real) => serde_json::Number::from_f64(real)
            .map(Value::Number)
            .ok_or_else(|| invalid(rusqlite::types::Type::Real)),
        ValueRef::Text(text) => {
            decoded(row, index, text).map(|text| Value::String(text.to_owned()))
        }
        ValueRef::Blob(_) => Err(invalid(rusqlite::types::Type::Blob)),
    }
}

fn run_record(row: &Row<'_>) -> rusqlite::Result<RunRecord> {
    fetched(row)?;
    Ok(RunRecord {
        status: row.get::<_, Option<String>>("status")?.map(RunState::new),
        coordinator: row.get("coordinator")?,
        deadline: row.get("deadline")?,
        member_limit: row.get("member_limit")?,
        outcome: row.get("outcome")?,
        outcome_reason: row.get("outcome_reason")?,
    })
}

/// `dict(row)` of a `SELECT * FROM members` row: every column, in table
/// order, as stored.
pub(super) fn member_row(row: &Row<'_>) -> rusqlite::Result<MemberRow> {
    fetched(row)?;
    let columns = (0..row.as_ref().column_count())
        .map(|index| {
            let name = row.as_ref().column_name(index)?.to_owned();
            Ok((name, cell_at(row, index)?))
        })
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(MemberRow { columns })
}

/// The board's `encode(value)` of a JSON argument.
fn encoded(value: &Value) -> Result<SqlValue, BoardError> {
    let value = PyJson::try_from(value).map_err(|error| BoardError::new(error.to_string()))?;
    py_json::encode(&value)
        .map(SqlValue::Text)
        .map_err(|error| BoardError::new(error.to_string()))
}

pub(super) fn failed(error: rusqlite::Error) -> BoardError {
    BoardError(contended(&error).0)
}

fn refused(error: TransactionError) -> BoardError {
    match error {
        TransactionError::Board(message) => BoardError(message),
        TransactionError::Sqlite(error) => failed(error),
    }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;
