//! The read-only projection of a swarm's board that a supervising client
//! reads on its parent's socket (#2475, #2478): the run, its items, its
//! members and totals, all at one board revision.
//!
//! Only allowlisted columns cross: a claim's or reservation's token never
//! does, and neither does any column a later schema adds. The board's
//! "tasks" are projected as items, with the project board's item field
//! names (`criteria`, `depends_on`, `state`, `owner`, `evidence`), while
//! `state` keeps the swarm's own vocabulary. It maps onto a project
//! board item's state as: `ready` → `todo`, `claimed` → `doing`,
//! `submitted` → `review`, `completed` → `done`, `blocked` → `blocked`.
use serde_json::{Map, Value};

use super::reads::SummaryCounts;

/// One read of a board.
#[derive(Clone, Debug, PartialEq)]
pub struct BoardProjection {
    /// The board's event cursor: every board write moves it.
    pub revision: i64,
    /// `None` until a run is created.
    pub run: Option<RunProjection>,
    pub items: Vec<ItemProjection>,
    pub members: Vec<MemberProjection>,
    /// The run's criterion evidence, one row per criterion and actor.
    pub evidence: Vec<EvidenceProjection>,
    /// Items by state as `summary` counts them, and claim counts.
    pub totals: SummaryCounts,
}

/// The run as stored; JSON columns loaded.
#[derive(Clone, Debug, PartialEq)]
pub struct RunProjection {
    pub id: Value,
    pub goal: Value,
    pub status: Value,
    pub outcome: Value,
    pub outcome_reason: Value,
    /// The latest pause or resume event, `0` for none.
    pub generation: i64,
    pub criteria: Value,
    pub member_limit: Value,
    pub deadline: Value,
    pub coordinator: Value,
    pub integrator: Value,
    /// The usage budget as stored, with `observed_tokens` and
    /// `unknown_usage_requests` from its ledger.
    pub budget: Map<String, Value>,
}

/// One item of the run's board.
#[derive(Clone, Debug, PartialEq)]
pub struct ItemProjection {
    pub id: Value,
    pub title: Value,
    /// The board's `acceptance`.
    pub criteria: Value,
    /// The board's `dependencies`: other items' ids.
    pub depends_on: Value,
    /// As `summary` reads it: a `ready` item with an unmet dependency is
    /// `blocked` with the blocker `unmet dependencies`.
    pub state: Value,
    /// The member holding it.
    pub owner: Value,
    pub blocker: Value,
    pub evidence: Value,
    /// The paths reserved for it, in path order.
    pub reserved_paths: Vec<Value>,
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
    pub id: Value,
    pub role: MemberRole,
    pub status: Value,
    /// The items it holds a claim on (claimed, blocked or submitted).
    pub claims: Vec<Value>,
    /// The time of its latest board event, `null` for none.
    pub last_activity: Value,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EvidenceProjection {
    pub criterion: Value,
    pub artifact: Value,
    pub revision: Value,
    pub kind: Value,
    pub actor: Value,
    pub accepted: Value,
}
