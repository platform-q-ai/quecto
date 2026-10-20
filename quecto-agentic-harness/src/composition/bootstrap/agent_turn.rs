//! Builders of the agent-turn use cases a dispatch loop holds (#1909).
use crate::application::agent_turn::use_cases::resume_after_provider_failure::ResumeAfterProviderFailure;

/// A session's automatic turns, taking turns and nothing suspended: the
/// loop's handle on the resume-after-a-provider-failure rule.
pub fn build_automatic_turns() -> ResumeAfterProviderFailure {
    ResumeAfterProviderFailure::new()
}
