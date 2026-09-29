//! Run creation and run status (#2270): the requests and views of
//! `CreateRun`, `BootstrapRun`, `ReadRunStatus` and `ReadRunSnapshot`, and
//! the rows the board ports read and write for them.
use serde_json::Value;

use crate::domain::swarm::{MemberState, RunState};

/// `Workbench.create(goal, constraints, criteria, member_limit, deadline)`
/// as `member`. Every argument stays the JSON value the member passed:
/// Python type-checks them at run time, so `true` or `3.0` for the member
/// limit must reach the use case to be refused with Python's message.
#[derive(Clone, Debug, PartialEq)]
pub struct CreateRunRequest {
    pub member: String,
    pub goal: Value,
    pub constraints: Value,
    pub criteria: Value,
    pub member_limit: Value,
    pub deadline: Value,
}

/// Which of `create`'s two branches ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CreateBranch {
    /// No run existed: a fresh run and the creator's member row.
    Fresh,
    /// The setup placeholder's coordinator created over it.
    OverSetup,
}

/// A created run. Python's `create` goes on to return the summary, a read
/// model a later slice (S12) adds; this slice ports the transaction only.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CreatedRun {
    pub branch: CreateBranch,
}

/// The first half of `Workbench._bootstrap(pid, started, socket)` as
/// `member`: the container's placeholder run and the member's live row.
/// The three values stay the JSON the member passed: Python binds them to
/// the row as they are (`True` as 1, and the column's affinity decides the
/// rest, epic P3), so the store binds them the same way.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapRunRequest {
    pub member: String,
    pub pid: Value,
    pub started: Value,
    pub socket: Value,
}

/// Whether the bootstrap wrote the placeholder (`false`: a run existed).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Bootstrapped {
    pub created: bool,
}

/// `swarm_repository.member_claim_counts` (#1969).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemberClaimCounts {
    /// Live or reserved members, the coordinator excluded, holding no
    /// claimed, blocked or submitted task.
    pub members_without_claim: i64,
    /// Members whose death was confirmed.
    pub members_dead: i64,
}

/// The `run` columns `Workbench._status` selects (`id, status, deadline,
/// coordinator, outcome`), each as stored: a column the board never leaves
/// empty may still be NULL in a file edited outside it, and Python reads
/// that as `None`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunStatusRow {
    pub id: Option<String>,
    pub status: Option<String>,
    /// A REAL as the board writes it; any other storage class as stored.
    pub deadline: Value,
    pub coordinator: Option<String>,
    pub outcome: Option<String>,
}

/// The `run` columns `Workbench.create` reads of an existing run: its
/// status and coordinator, each the text stored. `None` is NULL or a value
/// that is not text (a BLOB, or a number in a table rebuilt without TEXT
/// affinity), which only a file edited outside the board holds: Python compares each with a string, which neither
/// equals, so neither is the setup placeholder.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunOwnerRow {
    pub status: Option<String>,
    pub coordinator: Option<String>,
}

/// `Workbench._status()`: membership-free, whether a run was created.
#[derive(Clone, Debug, PartialEq)]
pub struct RunStatusView {
    pub counts: MemberClaimCounts,
    pub id: Option<String>,
    /// `setup` when the store holds no run.
    pub status: Option<String>,
    /// The integer `0` when the store holds no run.
    pub deadline: Value,
    pub coordinator: Option<String>,
    pub outcome: Option<String>,
}

/// A whole `members` row as Python's `dict(row)` reads it (#2270 round-3
/// review N5): every column the table has, in table order, each as stored
/// (NULL, an INTEGER, a finite REAL or TEXT). The board's own rows hold
/// `id, reservation, status, pid, started, socket, launcher`; a file
/// edited outside the board may hold NULL where the board never writes it
/// (an id, a status) and columns the board never added, and those are
/// listed as they are. `_bootstrap` binds the pid the member passed, so it
/// may be an INTEGER, a REAL or TEXT (epic P3).
///
/// The row is keyed by column name for `dict(row)` fidelity (#2270 round-4
/// review N2): `_snapshot` and `_admit` answer it whole, in table order,
/// and the membership use cases (#2271) read its `status`, `reservation`,
/// `pid` and `started` by name, as Python's `row[...]` does, through
/// [`MemberRow::get`] and [`MemberRow::text`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub columns: Vec<(String, Value)>,
}

impl MemberRow {
    /// The value stored in `column`, when the row has that column.
    pub fn get(&self, column: &str) -> Option<&Value> {
        self.columns
            .iter()
            .find(|(name, _)| name == column)
            .map(|(_, value)| value)
    }

    /// The text stored in `column`: `None` for a missing column, NULL, or
    /// a value that is not text.
    pub fn text(&self, column: &str) -> Option<&str> {
        self.get(column).and_then(Value::as_str)
    }
}

/// A member's `status` alone, as `SELECT status FROM members WHERE id=?`
/// reads it (#2275): the row exists, and `status` is its text, or `None`
/// for NULL or a value that is not text (a number or bytes, which Python
/// never finds equal to a status name).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberStatusRow {
    pub status: Option<String>,
}

/// `Workbench._snapshot()`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSnapshotView {
    /// `None` for a NULL `run.status`, which Python answers as `null`.
    pub status: Option<String>,
    pub coordinator: Option<String>,
    pub outcome: Option<String>,
    /// The id of the latest `paused` or `resumed` event, `0` for none.
    pub control_generation: i64,
    pub deadline: f64,
    pub members: Vec<MemberRow>,
}

/// The run contract `create` stores. `constraints` and `criteria` are the
/// values the member gave, stored with the board's `encode()`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunContract {
    pub goal: String,
    pub constraints: Value,
    pub criteria: Value,
    /// 1 through 25.
    pub member_limit: i64,
    /// Unix seconds (`run.deadline REAL`).
    pub deadline: f64,
}

/// A new `run` row.
#[derive(Clone, Debug, PartialEq)]
pub struct NewRun {
    pub id: String,
    pub contract: RunContract,
    pub coordinator: String,
    pub integrator: String,
    pub status: RunState,
}

/// A new `members` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NewMember {
    pub id: String,
    pub reservation: String,
    pub status: MemberState,
    /// The JSON values bound as Python's `sqlite3` binds them (NULL for
    /// `create`'s row; `_bootstrap`'s as the member passed them).
    pub pid: Value,
    pub started: Value,
    pub socket: Value,
    /// The harness that reserved the member (#1961); `None` for a member
    /// that joined by creating or bootstrapping the run.
    pub launcher: Option<String>,
}
