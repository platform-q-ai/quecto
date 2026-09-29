//! Board op telemetry (#2303, epic #2265): what one board op did, as the
//! event log records it (`swarm_op`). Pure values: ids, kinds, durations
//! and sizes only. No board text (titles, bodies, evidence, reasons,
//! paths) is ever a field, and the two caller-chosen ids are [`Redacted`].
use serde::{Deserialize, Serialize};

use crate::domain::redaction::Redacted;

/// Why the board refused an op: a stable, allowlisted kind for every
/// refusal the board raises. The message text is for the member, never
/// telemetry. `BoardError` cannot be built without one, so a new refusal
/// without a kind does not compile.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    /// The board holds no run.
    RunMissing,
    /// A coordinator-only op by another member.
    NotCoordinator,
    /// The caller is no member, or its death was confirmed.
    NotMember,
    /// The run is not running (paused, stopped, or not yet created).
    NotRunning,
    /// The run's deadline has passed.
    BudgetExhausted,
    /// The run's member limit is reached, or would be exceeded.
    MemberLimit,
    /// A member identity already used.
    IdentityTaken,
    /// `create` over a run that is no longer the setup placeholder.
    RunExists,
    /// Completion without the evidence or settlement it needs.
    CompletionUnmet,
    /// Evidence for a revision that is no longer current.
    StaleRevision,
    /// Submitted evidence, which cannot be revised.
    Immutable,
    /// A task in a state the op does not apply to.
    WrongState,
    /// A task owned by another member.
    NotOwner,
    /// A reservation or claim token that is no longer current.
    StaleToken,
    /// A file reserved by another member.
    ReservedByOther,
    /// A dependency that would close a cycle.
    DependencyCycle,
    /// An argument the board refuses (shape, type, bound).
    Invalid,
    /// A call that names no board method or does not bind to its signature.
    Calling,
    /// The store's write lock stayed busy past its timeout.
    Contended,
    /// The board file is gone.
    StoreMissing,
    /// Any other store failure (I/O, a corrupt or outside-edited value, a
    /// build without the board's SQLite options).
    Store,
    /// A broken invariant of the board's own code.
    Internal,
}

impl RefusalKind {
    /// The kind as it is serialized.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::RunMissing => "run_missing",
            Self::NotCoordinator => "not_coordinator",
            Self::NotMember => "not_member",
            Self::NotRunning => "not_running",
            Self::BudgetExhausted => "budget_exhausted",
            Self::MemberLimit => "member_limit",
            Self::IdentityTaken => "identity_taken",
            Self::RunExists => "run_exists",
            Self::CompletionUnmet => "completion_unmet",
            Self::StaleRevision => "stale_revision",
            Self::Immutable => "immutable",
            Self::WrongState => "wrong_state",
            Self::NotOwner => "not_owner",
            Self::StaleToken => "stale_token",
            Self::ReservedByOther => "reserved_by_other",
            Self::DependencyCycle => "dependency_cycle",
            Self::Invalid => "invalid",
            Self::Calling => "calling",
            Self::Contended => "contended",
            Self::StoreMissing => "store_missing",
            Self::Store => "store",
            Self::Internal => "internal",
        }
    }
}

/// Who called the op: a member in its run role, or the harness itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BoardRole {
    Coordinator,
    Worker,
    Integrator,
    /// A harness-internal op (`_status`, `_snapshot`, membership, liveness).
    Host,
}

/// How the op ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum BoardOpOutcome {
    Ok,
    Refused { kind: RefusalKind },
}

/// One board op, as the event log's `swarm_op` record holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardOpObservation {
    /// The board method's name.
    pub op: String,
    /// The caller's board identity (`members.id`).
    pub actor_ref: Redacted,
    pub role: BoardRole,
    /// The run the op's last transaction found, when it found one.
    pub run_id: Option<Redacted>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(flatten)]
    pub outcome: BoardOpOutcome,
    /// The whole op, binding and rendering included.
    pub duration_us: u64,
    /// From `BEGIN IMMEDIATE` issued to acquired, summed over the op's
    /// transactions.
    pub lock_wait_us: u64,
    /// Whether the store's busy handler fired at least once.
    pub busy: bool,
    /// Whether the op moved the caller's message cursor.
    pub cursor_moved: bool,
    /// The bytes of the JSON the op answered (0 for a refusal).
    pub result_bytes: u64,
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
