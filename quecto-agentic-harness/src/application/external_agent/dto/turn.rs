//! How a member's turn ended and what it used (#2285).

use crate::domain::conversation::value_objects::message::StopReason;
use crate::domain::external_agent::turn::TurnEnd;
use crate::domain::external_agent::usage::TurnUsage;

/// How one turn ended and what it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub end: TurnEnd,
    pub stop_reason: Option<StopReason>,
    pub usage: TurnUsage,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
    /// What the turn's stream said that did not change how it ended; the
    /// session logs these.
    pub warnings: Vec<TurnWarning>,
}

/// A signal of a turn the classification does not act on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TurnWarning {
    /// An assistant event carried an `error`, yet the result says the
    /// turn completed: the CLI's result is followed.
    AssistantError(String),
    /// The process's cumulative cost went down without a new process
    /// (the CLI reports 0 after some errors): nothing was charged.
    CumulativeCostDropped {
        previous_micro_usd: u64,
        reported_micro_usd: u64,
    },
}
