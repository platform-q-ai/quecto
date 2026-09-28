//! Membership admission, activation and launch records (#2271): the
//! requests and outcomes of `AdmitMember`, `ActivateMember`,
//! `RecordMemberLaunch`, `ReleaseUnlaunchedMember`, `RegisterMemberSocket`
//! and `JoinRun`.
//!
//! `actor` is the member the call acts as (Python's `store.actor`): the
//! events' actor, and the `launcher` an admission records. Each is a
//! harness-internal method, so the launch identity is typed as the harness
//! passes it (`ProcessIdentity`: an integer pid and the start-time text).
use serde_json::Value;

use super::MemberRow;

/// A launched member's process: `members.pid` INTEGER and `members.started`
/// TEXT, as the Rust callers bind them (`swarm_coordination.rs`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub pid: i64,
    pub started: String,
}

/// `Workbench._admit(member, reservation)` as `actor`. `member` stays the
/// JSON value passed: `bounded(member, 'member', 128)` refuses any value
/// that is not text with Python's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmitMemberRequest {
    pub actor: String,
    pub member: Value,
    pub reservation: String,
}

/// Which of admission's answers was taken.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdmissionDecision {
    /// A new `reserved` row, launched by the actor.
    Reserved,
    /// The member already holds this reservation alive: an idempotent retry.
    Retry,
}

/// The member's row after admission (`dict(row)`, every column in table
/// order) and the decision taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmittedMember {
    pub row: MemberRow,
    pub decision: AdmissionDecision,
}

/// `Workbench._activate(member, reservation, pid, started, socket)` as
/// `actor`. `reservation` is `None` only when a join passes on a NULL
/// stored reservation (Python compares `None != None` as equal).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivateMemberRequest {
    pub actor: String,
    pub member: String,
    pub reservation: Option<String>,
    pub launch: LaunchIdentity,
    pub socket: Option<String>,
}

/// `Workbench._record_launch(member, reservation, pid, started)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordMemberLaunchRequest {
    pub actor: String,
    pub member: String,
    pub reservation: String,
    pub launch: LaunchIdentity,
}

/// `Workbench._release_unlaunched(member)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseUnlaunchedMemberRequest {
    pub actor: String,
    pub member: String,
}

/// `Workbench._socket(socket)` as `actor`: the actor's own endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterMemberSocketRequest {
    pub actor: String,
    pub socket: Option<String>,
}

/// `join_process(context, reservation, pid, started, socket)` for the
/// joining `member` (the second half of `_bootstrap`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRunRequest {
    pub member: String,
    pub reservation: Option<String>,
    pub launch: LaunchIdentity,
    pub socket: Option<String>,
}

/// Which of the join's branches ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Joined {
    /// A new identity: the coordinator admitted it, then activated it.
    Admitted,
    /// This process is already the member's live one: nothing was written.
    AlreadyLive,
    /// A known identity under its own reservation: the coordinator
    /// activated it.
    Reactivated,
}
