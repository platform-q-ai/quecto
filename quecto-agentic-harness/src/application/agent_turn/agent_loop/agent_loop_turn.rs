use crate::application::agent_turn::agent_loop::agent_loop_stream::ends_turn_empty;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::conversation::value_objects::message::LlmResponse;

/// Internal vocabulary for the agent turn lifecycle.
///
/// These states intentionally describe the orchestration inside one public
/// `AgentLoop::process` call without becoming protocol-visible API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum TurnState {
    PrepareProviderRequest,
    AwaitProviderResponse,
    ExecuteToolCalls,
    FinalizeAssistantResponse,
    /// The model answered tool results with nothing (#2434): the turn
    /// ends with no reply recorded.
    EndTurnWithoutReply,
    StopAtToolIterationLimit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ProviderResponseTransition {
    FinalAssistantResponse,
    ToolCallContinuation,
    EmptyEndOfTurn,
}

/// What a reply does to the turn: tool calls continue it; an empty reply
/// the conversation lets be empty ends it with nothing to record (#2434);
/// anything else is the final answer.
pub(super) fn classify_provider_response(
    response: &LlmResponse,
    requirement: ReplyRequirement,
) -> ProviderResponseTransition {
    if ends_turn_empty(response, requirement) {
        ProviderResponseTransition::EmptyEndOfTurn
    } else if response.tool_calls.is_empty() {
        ProviderResponseTransition::FinalAssistantResponse
    } else {
        ProviderResponseTransition::ToolCallContinuation
    }
}

pub(super) fn next_state_after_provider_response(
    response: &LlmResponse,
    requirement: ReplyRequirement,
) -> TurnState {
    match classify_provider_response(response, requirement) {
        ProviderResponseTransition::FinalAssistantResponse => TurnState::FinalizeAssistantResponse,
        ProviderResponseTransition::ToolCallContinuation => TurnState::ExecuteToolCalls,
        ProviderResponseTransition::EmptyEndOfTurn => TurnState::EndTurnWithoutReply,
    }
}
