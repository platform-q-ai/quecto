//! Completion, task revalidation, contract amendment and criterion
//! evidence (#2273): the requests of `CompleteRun`, `RevalidateTask`,
//! `AmendRunContract` and `RecordEvidence`, and the rows `BoardEvidence`
//! and `BoardRuns` read and write for them.
//!
//! As in [`super::tasks`], `actor` is the member the call acts as and every
//! other argument stays the JSON value the caller passed: Python checks a
//! revision, a goal, criteria and evidence at run time, and binds a task id
//! or a criterion to its SQL untyped.
use serde_json::Value;

use super::TaskRow;

/// `Workbench.complete(revision)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompleteRunRequest {
    pub actor: String,
    pub revision: Value,
}

/// `Workbench.revalidate_task(task_id, revision, evidence)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevalidateTaskRequest {
    pub actor: String,
    pub task_id: Value,
    pub revision: Value,
    pub evidence: Value,
}

/// `Workbench.amend(goal, constraints, criteria, reason)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmendRunContractRequest {
    pub actor: String,
    pub goal: Value,
    pub constraints: Value,
    pub criteria: Value,
    pub reason: Value,
}

/// `Workbench.evidence(criterion, artifact, revision, kind, passed)` as
/// `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordEvidenceRequest {
    pub actor: String,
    pub criterion: Value,
    pub artifact: Value,
    pub revision: Value,
    pub kind: Value,
    pub passed: Value,
}

/// Whether `evidence` wrote its row or found the same row already there.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EvidenceTransition {
    Recorded,
    Unchanged,
}

/// What `evidence` answered (#2394): whether it wrote its row, the
/// evidence as recorded (the row's columns, `accepted` as the board
/// decided it), so the member sees whether its pass was accepted, and the
/// criterion as the row stores it (the caller's value under the column's
/// TEXT affinity).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordedEvidence {
    pub transition: EvidenceTransition,
    pub evidence: NewEvidence,
    pub criterion: Value,
}

/// One `evidence` row as stored (`SELECT * FROM evidence`): the board
/// writes text for the first four columns and `0` or `1` for `accepted`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceEntry {
    pub criterion: Value,
    pub artifact: Value,
    pub revision: Value,
    pub kind: Value,
    pub accepted: Value,
}

/// `SELECT criterion,artifact,revision,kind,accepted FROM evidence WHERE
/// criterion=? AND actor=?`: the row `evidence` compares a new record with,
/// and (#2394) the criterion as the row stores it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PriorEvidence {
    pub criterion: Value,
    pub artifact: Value,
    pub revision: Value,
    pub kind: Value,
    pub accepted: Value,
}

/// `INSERT OR REPLACE INTO evidence VALUES(?,?,?,?,?,?)`. The criterion is
/// the caller's value, bound as Python's `sqlite3` binds it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewEvidence {
    pub criterion: Value,
    pub artifact: String,
    pub revision: String,
    pub kind: String,
    pub actor: String,
    pub accepted: bool,
}

/// `Transaction.completion_state()`.
#[derive(Clone, Debug, PartialEq)]
pub struct CompletionState {
    /// `json.loads(run['criteria'])`.
    pub criteria: Value,
    /// Every `evidence` row, in store order.
    pub evidence: Vec<EvidenceEntry>,
    /// Every task as `Transaction.task` reads it (no derived status), by id.
    pub tasks: Vec<TaskRow>,
    /// Whether any file reservation is left.
    pub has_reservations: bool,
}

/// The contract `amend` reads of the run before it changes it: the goal
/// as stored, the constraints and criteria as `json.loads` reads them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StoredContract {
    pub goal: Value,
    pub constraints: Value,
    pub criteria: Value,
}

/// `amend`'s `UPDATE run SET goal=?,constraints=?,criteria=?`: the two
/// lists are the caller's values, stored with the board's `encode()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AmendedContract {
    pub goal: String,
    pub constraints: Value,
    pub criteria: Value,
}
