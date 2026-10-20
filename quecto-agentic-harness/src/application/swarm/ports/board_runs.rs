//! The coordination board's `run` row (#2270).

use crate::application::swarm::dto::{
    AmendedContract, DictRow, NewRun, RunContract, RunOwnerRow, RunStatusRow, StoredContract,
};
use crate::domain::swarm::entities::records::{RunRecord, RunState};
use crate::domain::swarm::value_objects::board_error::BoardError;
use serde_json::Value;

/// The `run` row.
pub trait BoardRuns {
    /// The run, when the store holds one.
    fn run(&self) -> Result<Option<RunRecord>, BoardError>;
    /// Whether the board holds every column a write transaction adds to an
    /// older board (`ensure_columns`, #2338 final review): a read
    /// transaction, which adds none, answers only from a board that does,
    /// so it answers and leaves the board as the full transaction would.
    fn columns_current(&self) -> Result<bool, BoardError>;
    /// Whether the store holds a run (`_bootstrap`'s `SELECT 1 FROM run`):
    /// no column of it is read.
    fn run_exists(&self) -> Result<bool, BoardError>;
    /// The two columns `create` reads of an existing run (status and
    /// coordinator), as stored, when the store holds one: only those, so a
    /// column it does not read is never decoded.
    fn run_owner(&self) -> Result<Option<RunOwnerRow>, BoardError>;
    /// The columns `_status` reads, as stored, when the store holds a run:
    /// only those, so a column it does not read is never decoded.
    fn run_status(&self) -> Result<Option<RunStatusRow>, BoardError>;
    /// The run's coordinator column alone (`join_process`'s `SELECT
    /// coordinator FROM run`): `None` when the store holds no run, and
    /// `Some(None)` for a coordinator that is NULL or not text.
    fn run_coordinator(&self) -> Result<Option<Option<String>>, BoardError>;
    fn insert_run(&self, run: &NewRun) -> Result<(), BoardError>;
    /// `create` over the setup placeholder: the new contract, and the run
    /// is running.
    fn update_run_contract(&self, contract: &RunContract) -> Result<(), BoardError>;
    /// The run pauses holding `outcome` for the supervisor (#1729).
    fn propose_outcome(&self, outcome: &str, reason: &str) -> Result<(), BoardError>;
    /// The run holds no outcome and no reason (#2273).
    fn clear_outcome(&self) -> Result<(), BoardError>;
    /// `SELECT * FROM run` as `dict(row)` (#2277), every column in table
    /// order, with `constraints` and `criteria` loaded from their JSON, as
    /// `summary` answers it; `None` when the store holds no run.
    fn run_row(&self) -> Result<Option<DictRow>, BoardError>;
    /// `_end_by_loss` on a pause holding no outcome (#2277): `UPDATE run
    /// SET outcome='failed', outcome_reason=?`; the status stays paused.
    fn hold_failed(&self, reason: &str) -> Result<(), BoardError>;
    /// `Transaction.set_outcome(status)`: the run's status alone becomes
    /// `status`; its outcome and reason are left as they are.
    fn set_outcome(&self, status: &RunState) -> Result<(), BoardError>;
    /// The run's deadline, in Unix seconds (a REAL).
    fn set_deadline(&self, deadline: f64) -> Result<(), BoardError>;
    /// `Transaction.pause_started`'s read: the `started` value of the
    /// latest `paused` event's detail as stored (NULL when the detail has
    /// none), `None` when no `paused` event exists.
    fn pause_started(&self) -> Result<Option<Value>, BoardError>;
    /// `json.loads(run['criteria'])` (#2273), when the store holds a run.
    fn run_criteria(&self) -> Result<Option<Value>, BoardError>;
    /// The contract `amend` reads before it changes it (#2273), when the
    /// store holds a run.
    fn run_contract(&self) -> Result<Option<StoredContract>, BoardError>;
    /// `amend`'s `UPDATE run SET goal=?,constraints=?,criteria=?`, the two
    /// lists stored with the board's `encode()` (#2273).
    fn amend_contract(&self, contract: &AmendedContract) -> Result<(), BoardError>;
}
