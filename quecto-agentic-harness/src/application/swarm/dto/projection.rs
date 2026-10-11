//! The read-only projection of a swarm's board that a supervising client
//! reads on its parent's socket (#2475, #2478): the run, its items, its
//! members and totals, read together.
//!
//! The store answers only allowlisted columns ([`ProjectionRows`]): no
//! claim, reservation or member token, no member pid, socket or launcher,
//! and no column a later schema adds. The use case types them, refusing
//! a value the board never writes, into [`BoardProjection`].
//!
//! The board's "tasks" are projected as items, with the project board's
//! item field names (`criteria`, `depends_on`, `state`, `owner`,
//! `evidence`). `state` keeps the swarm's own vocabulary
//! ([`ItemState`]); [`ItemState::project_board_state`] maps it onto a
//! project board item's state.
use serde_json::Value;

/// How many rows of each kind a projection reads at most.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProjectionLimits {
    pub items: i64,
    pub files: i64,
    pub members: i64,
    pub evidence: i64,
}

/// The allowlisted columns of a board, as stored, at one revision.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ProjectionRows {
    pub revision: i64,
    /// The latest pause or resume event, `0` for none.
    pub generation: i64,
    pub run: Option<StoredRunColumns>,
    /// By id, at most the limit.
    pub items: Vec<StoredItemColumns>,
    pub item_count: i64,
    /// `(path, task)`, by path, at most the limit.
    pub files: Vec<(Value, Value)>,
    pub file_count: i64,
    /// By id, at most the limit.
    pub members: Vec<StoredMemberColumns>,
    pub member_count: i64,
    /// By criterion then actor, at most the limit.
    pub evidence: Vec<StoredEvidenceColumns>,
    pub evidence_count: i64,
    /// The usage budget's stored payload (JSON loaded), `None` when no
    /// budget was ever set.
    pub budget: Option<Value>,
    /// The ledger's two totals; zero when there is no ledger yet.
    pub observed_tokens: Value,
    pub unknown_usage_requests: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredRunColumns {
    pub id: Value,
    pub goal: Value,
    /// JSON loaded.
    pub criteria: Value,
    pub coordinator: Value,
    pub integrator: Value,
    pub member_limit: Value,
    pub deadline: Value,
    pub status: Value,
    /// NULL on a board an older writer laid out without the column.
    pub outcome: Value,
    pub outcome_reason: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredItemColumns {
    pub id: Value,
    pub title: Value,
    /// `acceptance`, JSON loaded.
    pub criteria: Value,
    /// `dependencies`, JSON loaded.
    pub depends_on: Value,
    pub status: Value,
    pub owner: Value,
    pub blocker: Value,
    /// JSON loaded.
    pub evidence: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredMemberColumns {
    pub id: Value,
    pub status: Value,
    /// `max(time)` of its events.
    pub last_activity: Value,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct StoredEvidenceColumns {
    pub criterion: Value,
    pub artifact: Value,
    pub revision: Value,
    pub kind: Value,
    pub actor: Value,
    pub accepted: Value,
}

/// One read of a board.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardProjection {
    /// Grows with every board write the projection shows: the latest
    /// event's id plus the usage ledger's latest row.
    pub revision: i64,
    /// `None` until a run is created.
    pub run: Option<RunProjection>,
    pub items: Vec<ItemProjection>,
    pub members: Vec<MemberProjection>,
    /// The run's criterion evidence, one row per criterion and actor.
    pub evidence: Vec<EvidenceProjection>,
    /// Over every item and member, whatever was left out.
    pub totals: ProjectionTotals,
    /// Rows were left out: past a row limit or the byte budget.
    pub truncated: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RunProjection {
    pub id: String,
    pub goal: String,
    pub status: String,
    pub outcome: Option<String>,
    pub outcome_reason: Option<String>,
    pub generation: i64,
    /// The run's criteria as created: a list of `{id, kind, description}`.
    pub criteria: Value,
    pub member_limit: i64,
    pub deadline: f64,
    pub coordinator: Option<String>,
    pub integrator: Option<String>,
    pub budget: BudgetProjection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BudgetProjection {
    pub token_limit: Option<i64>,
    pub strict_unknown: bool,
    pub warned: bool,
    pub observed_tokens: i64,
    pub unknown_usage_requests: i64,
}

/// An item's state in the swarm's vocabulary; nothing else is accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ItemState {
    Ready,
    Claimed,
    Blocked,
    Submitted,
    Completed,
}

impl ItemState {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "ready" => Some(Self::Ready),
            "claimed" => Some(Self::Claimed),
            "blocked" => Some(Self::Blocked),
            "submitted" => Some(Self::Submitted),
            "completed" => Some(Self::Completed),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ready => "ready",
            Self::Claimed => "claimed",
            Self::Blocked => "blocked",
            Self::Submitted => "submitted",
            Self::Completed => "completed",
        }
    }

    /// The project board item state it maps to: a ready item is to do
    /// (waiting on its dependencies or not), a claimed one in hand, a
    /// submitted one in review, a completed one done, and a blocked one
    /// (its owner blocked it) blocked.
    pub fn project_board_state(self) -> &'static str {
        match self {
            Self::Ready => "todo",
            Self::Claimed => "doing",
            Self::Blocked => "blocked",
            Self::Submitted => "review",
            Self::Completed => "done",
        }
    }

    /// The states in which an item's owner holds a claim.
    pub fn holds_claim(self) -> bool {
        matches!(self, Self::Claimed | Self::Blocked | Self::Submitted)
    }
}

/// One item of the run's board.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemProjection {
    pub id: i64,
    pub title: String,
    /// The board's `acceptance`.
    pub criteria: Vec<String>,
    /// The board's `dependencies`: other items' ids.
    pub depends_on: Vec<i64>,
    /// As stored.
    pub state: ItemState,
    /// A ready item one of whose dependencies is not completed: to do, but
    /// not yet claimable. (`summary` reads it as `blocked`.)
    pub waiting: bool,
    /// The member holding it.
    pub owner: Option<String>,
    pub blocker: Option<String>,
    /// The evidence submitted for it, as the board stores it.
    pub evidence: Value,
    /// The paths reserved for it, by path.
    pub reserved_paths: Vec<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberRole {
    Coordinator,
    Integrator,
    Worker,
}

impl MemberRole {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Coordinator => "coordinator",
            Self::Integrator => "integrator",
            Self::Worker => "worker",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MemberProjection {
    pub id: String,
    pub role: MemberRole,
    pub status: String,
    /// The items it holds a claim on (claimed, blocked or submitted).
    pub claims: Vec<i64>,
    /// The time of its latest board event.
    pub last_activity: Option<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceProjection {
    pub criterion: String,
    pub artifact: Option<String>,
    pub revision: Option<String>,
    pub kind: Option<String>,
    pub actor: String,
    pub accepted: bool,
}

/// Items by stored state (`waiting` counts the ready ones whose
/// dependencies are not all completed), and members without a claim or
/// dead.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProjectionTotals {
    pub items: i64,
    pub ready: i64,
    pub waiting: i64,
    pub claimed: i64,
    pub blocked: i64,
    pub submitted: i64,
    pub completed: i64,
    pub members: i64,
    pub members_without_claim: i64,
    pub members_dead: i64,
}
