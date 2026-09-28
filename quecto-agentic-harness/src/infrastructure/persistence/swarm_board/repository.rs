//! The board ports over the SQLite store (#2270): `BoardRepository` opens
//! one [`BoardStore`] transaction per `atomic`, and the role ports run
//! Python's SQL (`swarm.py`, `swarm_repository.py`) on its connection.
//!
//! Every SQLite failure is `coordination store unavailable or contended:
//! {sqlite3_errmsg}`, as Python's `Store.transaction` reports it.
//!
//! Rows are read as Python reads them where the board's own writes or a
//! file edited outside it leave loose values (#2270 review L5, round-2 L2,
//! round-3 N1/N5): a NULL run status or coordinator is `None`, a member row
//! is every column the table has, in table order, as stored (`dict(row)`),
//! `_bootstrap` asks only whether a run exists, `create` reads only the
//! run's status and coordinator, and `_status` decodes only the columns it
//! selects. Any other column holding a type the board never writes, or a
//! BLOB, is refused the same way as a store failure, with the driver's
//! conversion error where Python would answer the value (the differential
//! suite's `outside_edited_columns` divergence).
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use super::binding;
use super::ledger;
use super::py_json::{self, PyJson};
use super::store::{BoardStore, CONTENDED, TransactionError, contended};
use crate::application::swarm::dto::{
    BoardLocation, MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract, RunOwnerRow,
    RunStatusRow,
};
use crate::application::swarm::ports::{
    BoardEvents, BoardMembers, BoardRepository, BoardRuns, BoardWork,
};
use crate::domain::swarm::{BoardError, MemberRecord, MemberState, RunRecord, RunState};

/// `swarm_repository.ACTIVE_CLAIM`.
const ACTIVE_CLAIM: &str = "('claimed','blocked','submitted')";

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
const PYTHON_PID_PARAMETER: usize = 3;
const PYTHON_STARTED_PARAMETER: usize = 4;
const PYTHON_SOCKET_PARAMETER: usize = 5;

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
struct SqliteBoard<'c> {
    connection: &'c Connection,
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
            .query_row("SELECT status, coordinator FROM run", [], |row| {
                Ok(RunOwnerRow {
                    status: row.get("status")?,
                    coordinator: row.get("coordinator")?,
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

impl BoardMembers for SqliteBoard<'_> {
    fn member(&self, id: &str) -> Result<Option<MemberRecord>, BoardError> {
        self.connection
            .query_row("SELECT * FROM members WHERE id=?", [id], |row| {
                Ok(MemberRecord {
                    id: row.get("id")?,
                    status: row
                        .get::<_, Option<String>>("status")?
                        .map(MemberState::new),
                    reservation: row.get("reservation")?,
                })
            })
            .optional()
            .map_err(failed)
    }

    fn members(&self) -> Result<Vec<MemberRow>, BoardError> {
        let mut statement = self
            .connection
            .prepare("SELECT * FROM members")
            .map_err(failed)?;
        let rows = statement.query_map([], member_row).map_err(failed)?;
        rows.collect::<rusqlite::Result<_>>().map_err(failed)
    }

    fn usage(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM members WHERE status IN ('live','reserved')")
    }

    fn not_dead(&self) -> Result<i64, BoardError> {
        self.count("SELECT count(*) FROM members WHERE status!='dead'")
    }

    fn claim_counts(&self, coordinator: Option<&str>) -> Result<MemberClaimCounts, BoardError> {
        let members_without_claim = self
            .connection
            .query_row(
                &format!(
                    "SELECT count(*) FROM members m WHERE m.status IN ('live','reserved') AND m.id IS NOT ? \
                     AND NOT EXISTS (SELECT 1 FROM tasks t WHERE t.owner=m.id AND t.status IN {ACTIVE_CLAIM})"
                ),
                [coordinator],
                |row| row.get(0),
            )
            .map_err(failed)?;
        let members_dead = self.count("SELECT count(*) FROM members WHERE status='dead'")?;
        Ok(MemberClaimCounts {
            members_without_claim,
            members_dead,
        })
    }

    fn insert_member(&self, member: &NewMember) -> Result<(), BoardError> {
        let parameters = [
            SqlValue::Text(member.id.clone()),
            SqlValue::Text(member.reservation.clone()),
            SqlValue::Text(member.status.as_str().to_owned()),
            loose(PYTHON_PID_PARAMETER, &member.pid)?,
            loose(PYTHON_STARTED_PARAMETER, &member.started)?,
            loose(PYTHON_SOCKET_PARAMETER, &member.socket)?,
            member
                .launcher
                .clone()
                .map_or(SqlValue::Null, SqlValue::Text),
        ];
        binding::bound_statement(
            self.connection,
            "INSERT INTO members(id,reservation,status,pid,started,socket,launcher) VALUES(?,?,?,?,?,?,?)",
            &parameters,
        )
        .and_then(|mut statement| statement.raw_execute())
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
    fn count(&self, sql: &str) -> Result<i64, BoardError> {
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
fn loose(position: usize, value: &Value) -> Result<SqlValue, BoardError> {
    PyJson::try_from(value)
        .map_err(|error| error.to_string())
        .and_then(|value| binding::bind(&value).map_err(|error| error.to_string()))
        .map_err(|error| {
            BoardError(format!(
                "{CONTENDED}: Error binding parameter {position}: {error}"
            ))
        })
}

/// A column as the JSON value of what it stores: NULL, an INTEGER, a
/// finite REAL or TEXT. A BLOB, or a REAL JSON cannot write, is a
/// conversion error.
fn cell(row: &Row<'_>, name: &str) -> rusqlite::Result<Value> {
    cell_at(row, row.as_ref().column_index(name)?)
}

/// The column at `index` as [`cell`] reads it.
fn cell_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
    use rusqlite::types::{FromSqlError, ValueRef};
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
        ValueRef::Text(text) => std::str::from_utf8(text)
            .map(|text| Value::String(text.to_owned()))
            .map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    index,
                    rusqlite::types::Type::Text,
                    Box::new(FromSqlError::Other(Box::new(error))),
                )
            }),
        ValueRef::Blob(_) => Err(invalid(rusqlite::types::Type::Blob)),
    }
}

fn run_record(row: &Row<'_>) -> rusqlite::Result<RunRecord> {
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
fn member_row(row: &Row<'_>) -> rusqlite::Result<MemberRow> {
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

fn failed(error: rusqlite::Error) -> BoardError {
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
