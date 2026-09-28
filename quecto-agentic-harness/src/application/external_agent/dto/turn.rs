//! How a member's turn ended and what it used (#2285).

use crate::domain::external_agent::turn::TurnEnd;
use crate::domain::external_agent::usage::TurnUsage;
use crate::domain::message::StopReason;

/// How one turn ended and what it used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnOutcome {
    pub end: TurnEnd,
    pub stop_reason: Option<StopReason>,
    pub usage: TurnUsage,
    pub num_turns: Option<u32>,
    pub duration_ms: Option<u64>,
}
