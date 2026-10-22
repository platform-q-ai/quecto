//! Builders of the agent-turn use cases a dispatch loop holds (#1909,
//! #1855).
use crate::application::agent_turn::use_cases::inspect_session_usage::InspectSessionUsage;
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
