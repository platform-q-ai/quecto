//! Builders of the agent-turn use cases a dispatch loop holds (#1909,
//! #1855).
use crate::application::agent_turn::use_cases::inspect_session_usage::InspectSessionUsage;
use crate::application::agent_turn::use_cases::parent_outcome::ParentOutcomeRuntime;
use crate::application::agent_turn::use_cases::resume_after_provider_failure::ResumeAfterProviderFailure;
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

/// A trusted root parent's turn-outcome runtime (#2708): its epochs are
/// fresh UUIDs, so a new incarnation never repeats an old one.
pub fn build_parent_outcome() -> std::sync::Arc<ParentOutcomeRuntime> {
    std::sync::Arc::new(ParentOutcomeRuntime::new(|| {
        uuid::Uuid::new_v4().to_string()
    }))
}
