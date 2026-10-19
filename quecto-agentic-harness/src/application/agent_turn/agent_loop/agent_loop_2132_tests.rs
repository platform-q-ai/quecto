//! #2132: feedback the turn added to correct a reply lasts only as long as
//! the correction. When the turn then fails, the feedback whose retry did
//! not succeed is withdrawn from the conversation, so the next turn never
//! starts with a note about a reply that no longer exists. Feedback that
//! led to an accepted reply stays: it is part of the exchange it explains.
use crate::application::agent_turn::agent_loop::tests::{
    MockProvider, MockRegistry, test_config, text_response, tool_call_response,
};
use crate::application::agent_turn::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::conversation::value_objects::message::{
    LlmResponse, Message, Role, StopReason, ThinkingBlock,
};
use crate::domain::shared::error::DomainError;
use std::sync::Arc;

fn malformed() -> Result<LlmResponse, DomainError> {
    Err(DomainError::Provider(
        "provider error (400): invalid_request_error: tool_use input is malformed".into(),
    ))
}

fn reasoning_only_at_the_limit() -> Result<LlmResponse, DomainError> {
    let mut response = text_response("");
    response.content = None;
    response.stop_reason = Some(StopReason::MaxTokens);
    // The whole budget spent (test_config asks for 1024): an output-limit stop.
    if let Some(usage) = response.usage.as_mut() {
        usage.completion_tokens = 1024;
    }
    response.thinking_blocks = vec![ThinkingBlock::Normal {
        thinking: "thinking at length".into(),
        signature: String::new(),
    }];
    Ok(response)
}

fn agent(responses: Vec<Result<LlmResponse, DomainError>>) -> AgentLoopImpl {
    let provider = Arc::new(MockProvider::new_results(responses));
    let mut registry = MockRegistry::new();
    registry.register(Arc::new(
        crate::application::agent_turn::agent_loop::tests::MockTool::new("echo", "echoed"),
    ));
    AgentLoopImpl::new(AgentLoopConfig {
        ..test_config(provider, Box::new(registry))
    })
}

fn feedback(messages: &[Message]) -> Vec<&str> {
    messages
        .iter()
        .filter(|m| m.role == Role::User && m.content != "the task")
        .map(|m| m.content.as_str())
        .collect()
}

#[tokio::test]
async fn a_turn_that_fails_after_malformed_feedback_withdraws_it() {
    let mut agent = agent(vec![malformed(), malformed(), malformed(), malformed()]);
    let mut messages = vec![Message::user("the task")];
    agent
        .run_loop(&mut messages)
        .await
        .expect_err("the malformed budget runs out");
    assert!(
        feedback(&messages).is_empty(),
        "no retry feedback outlives the failed turn: {messages:?}"
    );
    assert_eq!(
        messages.len(),
        1,
        "the conversation is as the turn found it"
    );
}

#[tokio::test]
async fn a_turn_that_fails_after_output_limit_feedback_withdraws_it() {
    let mut agent = agent(vec![
        reasoning_only_at_the_limit(),
        reasoning_only_at_the_limit(),
    ]);
    let mut messages = vec![Message::user("the task")];
    agent
        .run_loop(&mut messages)
        .await
        .expect_err("the cut-off budget runs out");
    assert!(feedback(&messages).is_empty(), "{messages:?}");
}

#[tokio::test]
async fn feedback_that_led_to_an_accepted_reply_stays_when_a_later_failure_ends_the_turn() {
    let mut agent = agent(vec![
        malformed(),
        Ok(tool_call_response("echo", "{}")),
        malformed(),
        malformed(),
        malformed(),
    ]);
    let mut messages = vec![Message::user("the task")];
    agent
        .run_loop(&mut messages)
        .await
        .expect_err("the malformed budget runs out");
    let kept = feedback(&messages);
    assert_eq!(kept.len(), 1, "only the settled correction stays: {kept:?}");
    assert!(kept[0].contains("malformed"));
    assert!(
        messages.iter().any(|m| m.role == Role::Tool),
        "the accepted tool turn stays"
    );
}

#[tokio::test]
async fn a_turn_that_recovers_keeps_its_feedback() {
    let mut agent = agent(vec![malformed(), Ok(text_response("fixed"))]);
    let mut messages = vec![Message::user("the task")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "fixed");
    assert_eq!(feedback(&messages).len(), 1);
}

/// #2606 review: the withdrawal is announced: the last conversation the
/// loop reports is the one the failed turn left, without the feedback.
#[tokio::test]
async fn a_failed_turn_reports_the_conversation_without_its_feedback() {
    use crate::domain::agents::value_objects::agent::AgentProgressEvent;
    let sizes = Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen = sizes.clone();
    let callback: crate::domain::agents::value_objects::agent::ProgressCallback =
        Arc::new(move |event| {
            if let AgentProgressEvent::ConversationChanged { messages } = event {
                seen.lock().unwrap().push(messages.len());
            }
        });
    let provider = Arc::new(MockProvider::new_results(vec![
        malformed(),
        malformed(),
        malformed(),
        malformed(),
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        progress_callback: Some(callback),
        ..test_config(provider, Box::new(MockRegistry::new()))
    });
    let mut messages = vec![Message::user("the task")];
    agent.run_loop(&mut messages).await.expect_err("fails");
    assert_eq!(messages.len(), 1);
    assert_eq!(
        sizes.lock().unwrap().last().copied(),
        Some(1),
        "the last reported conversation has the feedback withdrawn"
    );
}
