//! The board ports over the SQLite store (#2270): `BoardRepository` opens
//! one [`BoardStore`] transaction per `atomic`, and the role ports run
//! Python's SQL (`swarm.py`, `swarm_repository.py`) on its connection.
//!
//! Every SQLite failure is `coordination store unavailable or contended:
//! {sqlite3_errmsg}`, as Python's `Store.transaction` reports it. A row
//! whose columns do not have the types the board writes (only a file edited
//! outside the board holds one) is refused the same way, with the driver's
//! conversion error where Python would fail later with its own.
use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use super::ledger;
use super::py_json::{self, PyJson};
use super::store::{BoardStore, TransactionError, contended};
use crate::application::swarm::dto::{
    BoardLocation, MemberClaimCounts, MemberRow, NewMember, NewRun, RunContract,
};
use crate::application::swarm::ports::{
    BoardEvents, BoardMembers, BoardRepository, BoardRuns, BoardWork,
};
use crate::domain::swarm::{BoardError, MemberRecord, MemberState, RunRecord, RunState};

/// `swarm_repository.ACTIVE_CLAIM`.
const ACTIVE_CLAIM: &str = "('claimed','blocked','submitted')";

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

    fn run_id(&self) -> Result<Option<String>, BoardError> {
        self.connection
            .query_row("SELECT id FROM run", [], |row| row.get(0))
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
                    status: MemberState::new(row.get::<_, String>("status")?),
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
        self.connection
            .execute(
                "INSERT INTO members(id,reservation,status,pid,started,socket,launcher) VALUES(?,?,?,?,?,?,?)",
                params![
                    member.id,
                    member.reservation,
                    member.status.as_str(),
                    member.pid,
                    member.started,
                    member.socket,
                    member.launcher,
                ],
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
    fn count(&self, sql: &str) -> Result<i64, BoardError> {
        self.connection
            .query_row(sql, [], |row| row.get(0))
            .map_err(failed)
    }
}

fn run_record(row: &Row<'_>) -> rusqlite::Result<RunRecord> {
    Ok(RunRecord {
        status: RunState::new(row.get::<_, String>("status")?),
        coordinator: row.get("coordinator")?,
        deadline: row.get("deadline")?,
        member_limit: row.get("member_limit")?,
        outcome: row.get("outcome")?,
        outcome_reason: row.get("outcome_reason")?,
    })
}

fn member_row(row: &Row<'_>) -> rusqlite::Result<MemberRow> {
    Ok(MemberRow {
        id: row.get("id")?,
        reservation: row.get("reservation")?,
        status: row.get("status")?,
        pid: row.get("pid")?,
        started: row.get("started")?,
        socket: row.get("socket")?,
        launcher: row.get("launcher")?,
    })
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
