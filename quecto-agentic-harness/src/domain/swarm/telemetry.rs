//! Board op telemetry (#2303, epic #2265): what one board op did, as the
//! event log records it (`swarm_op`). Pure values: ids, kinds, durations
//! and sizes only. No board text (titles, bodies, evidence, reasons,
//! paths) is ever a field, and the caller-chosen id is [`Redacted`]; the
//! run id is the board's own (a uuid it generated), recorded as found.
use serde::{Deserialize, Serialize};

use super::{MemberExit, RunState};
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
    Refused {
        kind: RefusalKind,
        /// Whether the op's writes committed before it refused (#2277
        /// review M2: `create` commits the run, then its summary can
        /// refuse), so a record tells it from a refusal that wrote
        /// nothing; written only when true.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        committed: bool,
    },
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
    /// The harness's own op is [`BoardRole::Host`]; a member-facing op
    /// the board answered as a member's, or refused only after the
    /// operation gate authorised the caller as a member (#2313 review M2),
    /// records the caller's role in the run ([`run_role`]); `None`
    /// (written `null`) when the op read no run, or the caller was refused
    /// before or by the gate (no member, not the coordinator, no run).
    /// Never a role guessed.
    pub role: Option<BoardRole>,
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
    /// Whether the op moved the caller's cursor (#2276: the notification
    /// or wake cursor); `None` (written `null`) for an op that has no
    /// cursor to move, or a refusal.
    pub cursor_moved: Option<bool>,
    /// The bytes of the JSON the op answered (0 for a refusal): its
    /// compact serialization, as `serde_json` writes it, which is not
    /// always the size of Python's `json.dumps` text (whose separators
    /// carry spaces).
    pub result_bytes: u64,
    /// The decision the op took (#2277 review M1): a snake_case kind the
    /// dispatcher names from its own allowlist (`recorded`,
    /// `grace_pending`, `already_dead`, ...), never argument or board
    /// text; `None` (left out) for a refusal, but for one after the op's
    /// writes committed (#2277 review M2), whose decision is recorded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub decision: Option<String>,
    /// What the decision found and did, as counts and kinds only (#2277
    /// review M1), flat beside the other fields; each is left out when it
    /// does not apply.
    #[serde(flatten)]
    pub detail: BoardOpDetail,
}

/// Whether `decision` is shaped as a decision kind: nonempty lowercase
/// ASCII letters, digits and underscores, so no text can pass as one.
pub fn decision_kind(decision: &str) -> bool {
    !decision.is_empty()
        && decision
            .bytes()
            .all(|byte| matches!(byte, b'a'..=b'z' | b'0'..=b'9' | b'_'))
}

/// A run status as telemetry records it (#2277 review M1): one of the
/// statuses the board writes, or [`RunStatusKind::Unknown`] for a NULL or
/// any other text (only an edit from outside the board writes one), so
/// the text found is never recorded.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatusKind {
    Setup,
    Running,
    Paused,
    Succeeded,
    Blocked,
    Failed,
    Cancelled,
    BudgetExhausted,
    Unknown,
}

impl RunStatusKind {
    /// The kind of the status `status` a run row holds.
    pub fn of(status: Option<&RunState>) -> Self {
        match status.map(RunState::as_str) {
            Some("setup") => Self::Setup,
            Some("running") => Self::Running,
            Some("paused") => Self::Paused,
            Some("succeeded") => Self::Succeeded,
            Some("blocked") => Self::Blocked,
            Some("failed") => Self::Failed,
            Some("cancelled") => Self::Cancelled,
            Some("budget-exhausted") => Self::BudgetExhausted,
            _ => Self::Unknown,
        }
    }
}

/// The counts and kinds a served op's decision found and acted on
/// (#2277 review M1). Additive, as the refusal kinds are: a field is
/// added, never renamed or reused, and one that does not apply to the op
/// is `None` and left out. Never text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BoardOpDetail {
    /// The run's status as the op found it, before it wrote anything.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_status: Option<RunStatusKind>,
    /// The exit kind a death confirmation was given.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<MemberExit>,
    /// The file reservations a confirmed death left held (an abrupt
    /// exit's); 0 once an orderly exit released them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reservations_retained: Option<u64>,
    /// Whether the op ended the run by loss (a recorded loss, or the
    /// coordinator's confirmed death).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_by_loss: Option<bool>,
    /// For `summary` and `tasks` (#2277 review M2): the task owners whose
    /// liveness the op read, one per owned task (a summary's liveness
    /// watch and its page each count theirs).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub owners_scanned: Option<u64>,
    /// For a page the op answered (`summary`'s tasks, `tasks`, `events`):
    /// how many rows it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page_size: Option<u64>,
    /// For `events`: whether a later page holds more.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub has_more: Option<bool>,
    /// For a `summary` given the board's own cursor: whether an owner
    /// turned idle by the clock alone since it, which defeats the
    /// `unchanged` fast path.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fast_path_defeated: Option<bool>,
    /// For `_bootstrap`: whether it wrote the container's placeholder run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder_created: Option<bool>,
}

impl BoardOpDetail {
    /// No detail: nothing applies to the op.
    pub const NONE: Self = Self {
        run_status: None,
        exit: None,
        reservations_retained: None,
        ended_by_loss: None,
        owners_scanned: None,
        page_size: None,
        has_more: None,
        fast_path_defeated: None,
        placeholder_created: None,
    };
}

/// The role `member` holds in a run whose coordinator and integrator are
/// the ones given (#2303 reconcile): the coordinator first (Python's
/// `create` makes it the integrator too), then the integrator, and any
/// other member a worker. `None` unless the board accepted the caller as
/// a member of the run (`member_of_run`, #2313): a stranger, or a member
/// whose death was confirmed, holds no role, and is never guessed one.
pub fn run_role(
    member: &str,
    coordinator: Option<&str>,
    integrator: Option<&str>,
    member_of_run: bool,
) -> Option<BoardRole> {
    match (
        member_of_run,
        coordinator == Some(member),
        integrator == Some(member),
    ) {
        (false, _, _) => None,
        (true, true, _) => Some(BoardRole::Coordinator),
        (true, false, true) => Some(BoardRole::Integrator),
        (true, false, false) => Some(BoardRole::Worker),
    }
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
