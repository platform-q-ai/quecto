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
use std::cell::RefCell;

use rusqlite::types::Value as SqlValue;
use rusqlite::{Connection, OptionalExtension, Row, params};
use serde_json::Value;

use super::binding;
use super::ledger;
use super::meter::Tally;
use super::py_json::{self, PyJson};
use super::store::{
    BoardStore, CONTENDED, StoreFailure, StoreRefusal, TransactionError, Undecodable, contended,
    failure,
};
use super::usage_sums::{KeptSums, UsageSums};
use crate::application::swarm::dto::{
    AmendedContract, BoardLocation, DictRow, LatestActivity, MemberRow, NewRun, RunContract,
    RunOwnerRow, RunRoles, RunStatusRow, ScopeObservation, StoredContract,
};
use crate::application::swarm::ports::{BoardEvents, BoardRepository, BoardRuns, BoardWork};
use crate::domain::swarm::{BoardError, RefusalKind, RunRecord, RunState};

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

/// The board file of one run. It measures nothing: a metered call (#2303)
/// runs its transactions through its own [`MeteredCall`] instead
/// (`meter.rs`), over the same [`atomic_on`].
///
/// [`MeteredCall`]: crate::application::swarm::ports::MeteredCall
///
/// Its clones, and the metered calls over it, share the request ledger's
/// sums the process keeps between transactions (#2340, [`usage_sums`]).
#[derive(Clone, Debug)]
pub struct SqliteBoardRepository {
    store: BoardStore,
    sums: KeptSums,
}

impl SqliteBoardRepository {
    pub fn new(location: &BoardLocation) -> Self {
        Self {
            store: BoardStore::new(&location.database),
            sums: KeptSums::default(),
        }
    }

    /// The ledger work its board calls have done (#2340, tests only).
    #[cfg(test)]
    pub(crate) fn ledger_work(&self) -> super::LedgerWork {
        self.sums.work()
    }

    /// The board file this repository opens, and the sums it keeps.
    pub(super) fn into_parts(self) -> (BoardStore, KeptSums) {
        (self.store, self.sums)
    }
}

impl BoardRepository for SqliteBoardRepository {
    fn atomic(&self, create: bool, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        atomic_on(&self.store, &self.sums, None, create, work)
    }

    fn read(&self, work: &mut BoardWork<'_>) -> Result<(), BoardError> {
        read_on(&self.store, &self.sums, None, work)
    }
}

/// `BoardRepository::atomic` on `store`, measured on `tally` when there is
/// one (#2303). A refusal keeps its kind: `work`'s own refusal is returned
/// as it was raised, and the store's is classified by why it failed. The
/// ledger's sums the transaction computed are kept in `sums` once it has
/// committed, and only then (#2340).
pub(super) fn atomic_on(
    store: &BoardStore,
    sums: &KeptSums,
    tally: Option<&Tally>,
    create: bool,
    work: &mut BoardWork<'_>,
) -> Result<(), BoardError> {
    on_store(sums, tally, work, |body| {
        store.measured(create, tally, body)
    })
}

/// `BoardRepository::read` on `store` (#2338): `work` in the store's read
/// transaction, measured on `tally` when there is one, its refusals kept
/// as [`atomic_on`] keeps them.
pub(super) fn read_on(
    store: &BoardStore,
    sums: &KeptSums,
    tally: Option<&Tally>,
    work: &mut BoardWork<'_>,
) -> Result<(), BoardError> {
    on_store(sums, tally, work, |body| store.measured_read(tally, body))
}

/// The body of a board transaction `begin` runs: the role ports over its
/// connection, and the run's id and roles noted for a metered call.
fn on_store(
    sums: &KeptSums,
    tally: Option<&Tally>,
    work: &mut BoardWork<'_>,
    begin: impl FnOnce(
        &mut dyn FnMut(&rusqlite::Transaction<'_>) -> Result<(), TransactionError>,
    ) -> Result<(), (StoreRefusal, Option<StoreFailure>)>,
) -> Result<(), BoardError> {
    let mut refused: Option<BoardError> = None;
    let staged = RefCell::new(None);
    let mut body = |transaction: &rusqlite::Transaction<'_>| {
        let board = SqliteBoard {
            connection: transaction,
            tally,
            usage_schema_created: std::cell::Cell::new(false),
            kept_sums: sums,
            staged_sums: &staged,
        };
        let done = work(&board);
        // Only a metered op that has not found the run row's id and
        // roles asks for them (#2313): one whose run rows named no
        // integrator (`run_status`'s) reads the roles here, so its
        // caller's role is still recorded.
        if let Some(tally) = tally.filter(|tally| tally.wants_run()) {
            board.run_read(tally);
        }
        done.map_err(|refusal| {
            refused = Some(refusal.clone());
            TransactionError::Board(refusal)
        })
    };
    let outcome = begin(&mut body).map_err(|(refusal, failure)| match (failure, refused) {
        (None, Some(refused)) => refused,
        (failure, _) => BoardError::new(store_kind(failure), refusal.0),
    });
    // The ledger's sums the transaction computed are kept once it has
    // ended well, and only then (#2340): a read's are of committed rows.
    if let (Ok(()), Some(summed)) = (&outcome, staged.into_inner()) {
        sums.keep(summed);
    }
    outcome
}

/// The kind of a refusal the store raised itself; `None` (a body refusal
/// that was not the work's) cannot happen, and is an internal fault.
fn store_kind(failure: Option<StoreFailure>) -> RefusalKind {
    match failure {
        Some(StoreFailure::Missing) => RefusalKind::StoreMissing,
        Some(StoreFailure::Busy) => RefusalKind::Contended,
        Some(StoreFailure::Failed) => RefusalKind::Store,
        None => RefusalKind::Internal,
    }
}

/// The role ports over one open transaction, and the metered call's
/// tally when there is one.
pub(super) struct SqliteBoard<'c> {
    pub(super) connection: &'c Connection,
    pub(super) tally: Option<&'c Tally>,
    /// Whether this transaction has run `_usage_schema`'s statements: they
    /// run once per transaction, as their `IF NOT EXISTS` makes every later
    /// run a no-op inside it.
    pub(super) usage_schema_created: std::cell::Cell<bool>,
    /// The ledger's sums the process kept from a committed transaction.
    pub(super) kept_sums: &'c KeptSums,
    /// The ledger's sums this transaction computed, kept once it commits.
    pub(super) staged_sums: &'c RefCell<Option<UsageSums>>,
}

impl BoardRuns for SqliteBoard<'_> {
    fn columns_current(&self) -> Result<bool, BoardError> {
        super::store::columns_current(self.connection).map_err(failed)
    }

    fn run(&self) -> Result<Option<RunRecord>, BoardError> {
        self.connection
            .query_row("SELECT * FROM run", [], |row| {
                self.seen(row);
                run_record(row)
            })
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
                self.seen(row);
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
                    self.seen(row);
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
        self.connection
            .query_row("SELECT coordinator FROM run", [], |row| {
                fetched(row)?;
                text(row, "coordinator")
            })
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

    fn clear_outcome(&self) -> Result<(), BoardError> {
        self.update_run(
            "UPDATE run SET outcome=NULL, outcome_reason=NULL",
            params![],
        )
    }

    fn run_row(&self) -> Result<Option<DictRow>, BoardError> {
        self.run_dict()
    }

    fn hold_failed(&self, reason: &str) -> Result<(), BoardError> {
        self.failed_hold(reason)
    }

    fn set_outcome(&self, status: &RunState) -> Result<(), BoardError> {
        self.update_run("UPDATE run SET status=?", [status.as_str()])
    }

    fn set_deadline(&self, deadline: f64) -> Result<(), BoardError> {
        self.update_run("UPDATE run SET deadline=?", [deadline])
    }

    fn pause_started(&self) -> Result<Option<Value>, BoardError> {
        super::repository_control::pause_started(self.connection)
    }

    fn run_criteria(&self) -> Result<Option<Value>, BoardError> {
        super::repository_evidence::run_criteria(self)
    }

    fn run_contract(&self) -> Result<Option<StoredContract>, BoardError> {
        super::repository_evidence::run_contract(self)
    }

    fn amend_contract(&self, contract: &AmendedContract) -> Result<(), BoardError> {
        super::repository_evidence::amend_contract(self, contract)
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
        let detail = PyJson::try_from(detail)
            .map_err(|error| BoardError::new(RefusalKind::Invalid, error.to_string()))?;
        ledger::event(self.connection, actor, time, action, &detail).map_err(refused)
    }

    fn scope_observations(&self) -> Result<Vec<ScopeObservation>, BoardError> {
        self.observations()
    }

    fn latest_activity(&self, actors: &[&str]) -> Result<Vec<LatestActivity>, BoardError> {
        self.latest_of(actors)
    }

    fn event_time(&self, id: i64) -> Result<Option<Value>, BoardError> {
        self.time_of(id)
    }

    fn event_page(&self, after: u64, limit: i64) -> Result<Vec<DictRow>, BoardError> {
        self.events_after(after, limit)
    }

    fn control_generation(&self) -> Result<i64, BoardError> {
        self.count("SELECT coalesce(max(id),0) FROM events WHERE action IN ('paused','resumed')")
    }

    /// A time the board did not write (text, NULL) is no creation time.
    fn created_at(&self) -> Result<Option<f64>, BoardError> {
        let time = self
            .connection
            .query_row(
                "SELECT time FROM events WHERE action='created' ORDER BY id DESC LIMIT 1",
                [],
                |row| row.get::<_, rusqlite::types::Value>(0),
            )
            .optional()
            .map_err(failed)?;
        Ok(match time {
            Some(rusqlite::types::Value::Real(time)) => Some(time),
            // An integer time is exact in f64 far past any clock reading.
            Some(rusqlite::types::Value::Integer(time)) => Some(time as f64),
            Some(
                rusqlite::types::Value::Null
                | rusqlite::types::Value::Text(_)
                | rusqlite::types::Value::Blob(_),
            )
            | None => None,
        })
    }
}

impl SqliteBoard<'_> {
    /// Notes the run id and roles of a run row the op read, for telemetry
    /// only (#2303), for a metered call that has found them not yet: no
    /// statement of its own.
    pub(super) fn seen(&self, row: &Row<'_>) {
        if let Some(tally) = self.tally.filter(|tally| tally.wants_run()) {
            run_noted(tally, row);
        }
    }

    /// Reads the run's id and roles into `tally`, for telemetry only
    /// (#2303): only while metered and the run id or roles are not found
    /// yet (an op that never read the run row, or read only some of it).
    /// No run, or a failed read, notes nothing.
    fn run_read(&self, tally: &Tally) {
        let _noted =
            self.connection
                .query_row("SELECT id, coordinator, integrator FROM run", [], |row| {
                    run_noted(tally, row);
                    Ok(())
                });
    }

    /// An `UPDATE` of the run row, which the operation gate has read in
    /// this transaction, so it changes that row (and any other a file
    /// edited outside the board holds, as Python's does).
    fn update_run(&self, sql: &str, parameters: impl rusqlite::Params) -> Result<(), BoardError> {
        debug_assert!(sql.starts_with("UPDATE run SET "), "a run update: {sql}");
        let changed = self.connection.execute(sql, parameters).map_err(failed)?;
        debug_assert!(changed >= 1, "{sql} changes the run row the gate read");
        Ok(())
    }

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
            BoardError::new(
                RefusalKind::Invalid,
                format!("{CONTENDED}: Error binding parameter {position}: {error}"),
            )
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
pub(super) fn text(row: &Row<'_>, name: &str) -> rusqlite::Result<Option<String>> {
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
pub(super) fn cell_at(row: &Row<'_>, index: usize) -> rusqlite::Result<Value> {
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
pub(super) fn encoded(value: &Value) -> Result<SqlValue, BoardError> {
    let invalid = |error: String| BoardError::new(RefusalKind::Invalid, error);
    let value = PyJson::try_from(value).map_err(|error| invalid(error.to_string()))?;
    py_json::encode(&value)
        .map(SqlValue::Text)
        .map_err(|error| invalid(error.to_string()))
}

/// Notes a run `row`'s id and roles in `tally` (#2303): a value that is
/// not text is none, and the roles only when the row holds both columns
/// (the op's `SELECT` named them).
fn run_noted(tally: &Tally, row: &Row<'_>) {
    let column = |name: &str| row.get::<_, Option<String>>(name);
    let roles = match (column("coordinator"), column("integrator")) {
        (Ok(coordinator), Ok(integrator)) => Some(RunRoles {
            coordinator,
            integrator,
        }),
        _ => None,
    };
    tally.run_seen(column("id").ok().flatten().as_deref(), roles);
}

/// A SQLite failure inside a transaction, contended when SQLite was busy.
pub(super) fn failed(error: rusqlite::Error) -> BoardError {
    let kind = store_kind(Some(failure(&error)));
    BoardError::new(kind, contended(&error).0)
}

/// A ledger failure (`ledger::event`, `ledger::retry`): a board refusal
/// keeps the kind it was raised under, the ledger's own or a request
/// action's (#2303).
pub(super) fn refused(error: TransactionError) -> BoardError {
    match error {
        TransactionError::Board(refusal) => refusal,
        TransactionError::Sqlite(error) => failed(error),
    }
}

#[cfg(test)]
#[path = "repository_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "repository_read_tests.rs"]
mod read_tests;
