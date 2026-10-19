use super::agent_loop_turn::*;
use crate::domain::conversation::services::reply_requirement::ReplyRequirement;
use crate::domain::conversation::value_objects::message::{LlmResponse, ToolCall};

fn text_response() -> LlmResponse {
    LlmResponse {
        content: Some("done".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

fn tool_response() -> LlmResponse {
    LlmResponse {
        content: None,
        tool_calls: vec![ToolCall {
            id: "call_1".to_string(),
            name: "read".to_string(),
            arguments: "{}".to_string(),
        }],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[test]
fn final_assistant_response_transitions_to_finalization() {
    assert_eq!(
        next_state_after_provider_response(&text_response(), ReplyRequirement::Output),
        TurnState::FinalizeAssistantResponse
    );
}

#[test]
fn tool_call_response_transitions_to_tool_continuation() {
    assert_eq!(
        next_state_after_provider_response(&tool_response(), ReplyRequirement::MayBeEmpty),
        TurnState::ExecuteToolCalls
    );
}

#[test]
fn mixed_content_and_tool_calls_prefers_tool_continuation() {
    let mut response = tool_response();
    response.content = Some("I need a tool".to_string());

    assert_eq!(
        next_state_after_provider_response(&response, ReplyRequirement::MayBeEmpty),
        TurnState::ExecuteToolCalls
    );
}
