//! What folding one event changed (#2285).

use super::{ExecutionState, TurnOutcome};

/// What one [`crate::domain::external_agent::stream::ExternalAgentEvent`]
/// changed: the state it entered, if it changed the state, and the turn it
/// ended, if it ended one. The projection keeps no history of either.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ProjectionStep {
    pub transition: Option<ExecutionState>,
    pub turn_end: Option<TurnOutcome>,
}
