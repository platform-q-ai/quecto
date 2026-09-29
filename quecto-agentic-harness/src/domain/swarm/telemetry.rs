//! Board op telemetry (#2303, epic #2265): what one board op did, as the
//! event log records it (`swarm_op`). Pure values: ids, kinds, durations
//! and sizes only. No board text (titles, bodies, evidence, reasons,
//! paths) is ever a field, and the caller-chosen id is [`Redacted`]; the
//! run id is the board's own (a uuid it generated), recorded as found.
use serde::{Deserialize, Serialize};

use crate::domain::redaction::Redacted;

/// Why the board refused an op: a stable, allowlisted kind for every
/// refusal the board raises. The message text is for the member, never
/// telemetry. `BoardError` cannot be built without one, so a new refusal
/// without a kind does not compile.
///
/// The kinds cover every refusal the Python board raises (#2303 round-3
/// review L4; `tests/architecture/swarm_board_python_refusals.rs` maps each
/// Python text to its kind, for the slices that port them). Kinds are
/// additive: one is added, never renamed or reused, and a consumer must
/// accept a kind it does not know.
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
    /// The run's budget is spent: its deadline has passed, or it is paused
    /// (or ended) as `budget-exhausted`.
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
    /// A task, message, reservation or run in a state the op does not
    /// apply to.
    WrongState,
    /// A task or message owned by another member.
    NotOwner,
    /// A reservation or claim token that is no longer current.
    StaleToken,
    /// A file reserved by another member.
    ReservedByOther,
    /// A dependency that would close a cycle.
    DependencyCycle,
    /// A task, message or recipient the board does not hold.
    NotFound,
    /// A bounded board table is full: the task or file reservation board,
    /// a recipient's inbox, or a request ledger.
    CapacityFull,
    /// An op only the supervisor, outside the swarm, may take (resume,
    /// close, extend).
    SupervisorOnly,
    /// A launch whose process identity conflicts with the member's
    /// recorded one.
    LaunchConflict,
    /// A request id (or request observation id) reused with different
    /// data.
    RequestIdReused,
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
            Self::NotFound => "not_found",
            Self::CapacityFull => "capacity_full",
            Self::SupervisorOnly => "supervisor_only",
            Self::LaunchConflict => "launch_conflict",
            Self::RequestIdReused => "request_id_reused",
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

/// One board op, as the event log's `swarm_op` record holds it. The
/// envelope's `turn` is `null`: a board op is not filed under the agent's
/// turn (the dispatcher is called without one).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardOpObservation {
    /// The board method's name.
    pub op: String,
    /// The caller's board identity (`members.id`): the member id the
    /// caller chose, bounded by the board, and redacted when it is shaped
    /// like a credential.
    pub actor_ref: Redacted,
    pub role: BoardRole,
    /// The run the op found, when it found one and its id is one the
    /// board generates ([`board_run_id`]); any other id (a board edited
    /// from outside) is `None`, so it is never caller text and is not
    /// redacted.
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message_id: Option<i64>,
    #[serde(flatten)]
    pub outcome: BoardOpOutcome,
    /// The whole op, binding and rendering included.
    pub duration_us: u64,
    /// From `BEGIN IMMEDIATE` issued to acquired, summed over the op's
    /// transactions; `None` (written `null`) when nothing was measured: the
    /// op began no transaction. Never a zero standing in for "unknown".
    pub lock_wait_us: Option<u64>,
    /// The time the store's busy handler slept for this op, over every
    /// statement that found the database busy (`BEGIN`, a read, the
    /// commit); `None` when nothing was measured.
    pub busy_wait_us: Option<u64>,
    /// Whether the store's busy handler fired at least once; `None` when
    /// nothing was measured.
    pub busy: Option<bool>,
    /// Whether the op moved the caller's message cursor; `None` (written
    /// `null`) for an op that has no cursor to move, or a refusal.
    pub cursor_moved: Option<bool>,
    /// The bytes of the JSON the op answered (0 for a refusal): its
    /// compact serialization, as `serde_json` writes it, which is not
    /// always the size of Python's `json.dumps` text (whose separators
    /// carry spaces).
    pub result_bytes: u64,
}

/// Whether `id` is a run id the boards generate (#2303 round-4 review
/// L3): Python's `uuid.uuid4().hex`, and the Rust port's `IdSource`, is
/// 32 lowercase hex digits. Only such an id is recorded.
pub fn board_run_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[cfg(test)]
#[path = "telemetry_tests.rs"]
mod tests;
