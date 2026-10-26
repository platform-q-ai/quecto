//! Builders of the agent-turn use cases a dispatch loop holds (#1909,
//! #1855).
use crate::application::agent_turn::use_cases::inspect_session_usage::InspectSessionUsage;
use crate::application::agent_turn::use_cases::parent_outcome::ParentOutcomeRuntime;
use crate::application::agent_turn::use_cases::resume_after_provider_failure::ResumeAfterProviderFailure;
use crate::domain::agents::services::turn_outcome_policy::turn_outcomes_switched_on;
use crate::interface::cli::uds_session::AgentSession;

/// A UDS loop's session tracker for `model`, holding the agent-turn use
/// cases: its automatic turns (taking turns, nothing suspended) and its
/// usage (nothing used yet).
pub fn build_turn_session(model: String) -> AgentSession {
    AgentSession::new(
        model,
        ResumeAfterProviderFailure::new(),
        InspectSessionUsage::new(),
    )
}

/// The variable a launcher sets to `1` to switch a root's turn-outcome
/// guard on (#2708): the Pocket relay, for the harness of a task.
pub const TURN_OUTCOMES_VARIABLE: &str = "QUECTO_TURN_OUTCOMES";

/// The runtime when the launcher switched the guard on, else `None`: the
/// root then runs exactly as before, advertising and offering nothing.
pub fn build_requested_parent_outcome() -> Option<std::sync::Arc<ParentOutcomeRuntime>> {
    let value = std::env::var(TURN_OUTCOMES_VARIABLE).ok();
    turn_outcomes_switched_on(value.as_deref()).then(build_parent_outcome)
}

/// A trusted root parent's turn-outcome runtime (#2708): its epochs are
/// fresh UUIDs, so a new incarnation never repeats an old one.
pub fn build_parent_outcome() -> std::sync::Arc<ParentOutcomeRuntime> {
    std::sync::Arc::new(ParentOutcomeRuntime::new(|| {
        uuid::Uuid::new_v4().to_string()
    }))
}
