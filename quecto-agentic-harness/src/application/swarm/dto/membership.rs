//! Membership admission, activation and launch records (#2271): the
//! requests and outcomes of `AdmitMember`, `ActivateMember`,
//! `RecordMemberLaunch`, `ReleaseUnlaunchedMember`, `RegisterMemberSocket`
//! and `JoinRun`.
//!
//! `actor` is the member the call acts as (Python's `store.actor`): the
//! events' actor, and the `launcher` an admission records. Every other
//! argument stays the JSON value the caller passed (#2271 round-1 review
//! M1): Python binds each to its SQL untyped, the column affinity decides
//! what is stored, and the board compares stored values with the argument
//! by Python's `==` (`domain::swarm::python_equal`).
use serde_json::Value;

use super::MemberRow;

/// A launched member's process as the caller passed it: `members.pid`
/// (INTEGER affinity) and `members.started` (TEXT affinity) are bound as
/// given, and a recorded process is this one only when both compare equal
/// by Python's `==`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchIdentity {
    pub pid: Value,
    pub started: Value,
}

/// `Workbench._admit(member, reservation)` as `actor`. `member` stays the
/// JSON value passed: `bounded(member, 'member', 128)` refuses any value
/// that is not text with Python's message.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AdmitMemberRequest {
    pub actor: String,
    pub member: Value,
    pub reservation: Value,
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
/// `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ActivateMemberRequest {
    pub actor: String,
    pub member: Value,
    pub reservation: Value,
    pub launch: LaunchIdentity,
    pub socket: Value,
}

/// `Workbench._record_launch(member, reservation, pid, started)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecordMemberLaunchRequest {
    pub actor: String,
    pub member: Value,
    pub reservation: Value,
    pub launch: LaunchIdentity,
}

/// `Workbench._release_unlaunched(member)` as `actor`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReleaseUnlaunchedMemberRequest {
    pub actor: String,
    pub member: Value,
}

/// `Workbench._socket(socket)` as `actor`: the actor's own endpoint.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegisterMemberSocketRequest {
    pub actor: String,
    pub socket: Value,
}

/// `join_process(context, reservation, pid, started, socket)` for the
/// joining `member` (the second half of `_bootstrap`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JoinRunRequest {
    pub member: String,
    pub reservation: Value,
    pub launch: LaunchIdentity,
    pub socket: Value,
}

/// Which of the join's branches ran. Python answers every branch with the
/// coordinator's `summary()` (`JoinMember`, `BootstrapMember`, #2277).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Joined {
    /// A new identity: the coordinator admitted it, then activated it.
    Admitted,
    /// This process is already the member's live one: nothing was written.
    /// Python still answers `coordinator.summary()`, whose gate can refuse
    /// (a coordinator that is nobody, or dead), so the branch carries the
    /// coordinator the join read, for the summary to run that gate as
    /// without reading the run again. `None` is a run whose coordinator is NULL or
    /// not text, which Python's `Workbench` would act as.
    AlreadyLive { coordinator: Option<String> },
    /// A known identity under its own reservation: the coordinator
    /// activated it.
    Reactivated,
}

impl Joined {
    /// Whether the branch, once it ran to its end, wrote to the board:
    /// the admission and activation, or the activation.
    #[must_use]
    pub fn wrote(&self) -> bool {
        matches!(self, Self::Admitted | Self::Reactivated)
    }
}
