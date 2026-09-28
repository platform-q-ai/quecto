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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BootstrapRunRequest {
    pub member: String,
    pub pid: Option<i64>,
    pub started: Option<String>,
    pub socket: Option<String>,
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

/// `_status`'s deadline: the run's, or the placeholder Python reports as
/// the integer `0` when the store holds no run.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum StatusDeadline {
    NoRun,
    Stored(f64),
}

/// `Workbench._status()`: membership-free, whether a run was created.
#[derive(Clone, Debug, PartialEq)]
pub struct RunStatusView {
    pub counts: MemberClaimCounts,
    pub id: Option<String>,
    /// `setup` when the store holds no run.
    pub status: String,
    pub deadline: StatusDeadline,
    pub coordinator: Option<String>,
    pub outcome: Option<String>,
}

/// A whole `members` row, in column order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRow {
    pub id: String,
    pub reservation: Option<String>,
    pub status: String,
    pub pid: Option<i64>,
    pub started: Option<String>,
    pub socket: Option<String>,
    pub launcher: Option<String>,
}

/// `Workbench._snapshot()`.
#[derive(Clone, Debug, PartialEq)]
pub struct RunSnapshotView {
    pub status: String,
    pub coordinator: String,
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
    pub pid: Option<i64>,
    pub started: Option<String>,
    pub socket: Option<String>,
    /// The harness that reserved the member (#1961); `None` for a member
    /// that joined by creating or bootstrapping the run.
    pub launcher: Option<String>,
}
