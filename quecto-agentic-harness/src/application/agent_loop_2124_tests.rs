//! #2124: a reply that hits the output limit with nothing visible (only
//! reasoning) is not a final answer: the model is asked again, concisely,
//! and a turn that keeps doing it fails instead of ending silently.
use super::*;
use crate::domain::message::{StopReason, ThinkingBlock};

fn reasoning_only_at_the_limit() -> LlmResponse {
    let mut response = text_response("");
    response.content = None;
    response.stop_reason = Some(StopReason::MaxTokens);
    response.thinking_blocks = vec![ThinkingBlock::Normal {
        thinking: "let me think about this at great length".into(),
        signature: String::new(),
    }];
    response
}

fn agent(responses: Vec<Result<LlmResponse, DomainError>>) -> (AgentLoopImpl, Arc<MockProvider>) {
    let provider = Arc::new(MockProvider::new_results(responses));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 8192,
        temperature: 0.7,
        retention: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    });
    (agent, provider)
}

#[tokio::test]
async fn a_reasoning_only_reply_at_the_limit_is_asked_again_not_accepted_as_empty() {
    let (mut agent, provider) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent
        .run_loop(&mut messages)
        .await
        .expect("the turn completes");
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
    let asked = messages
        .iter()
        .filter(|m| m.role == Role::User)
        .any(|m| m.content.contains("output limit") && m.content.contains("8192"));
    assert!(
        asked,
        "the model is told why and how to answer: {messages:?}"
    );
    let consecutive_user = messages
        .windows(2)
        .any(|w| w[0].role == Role::User && w[1].role == Role::User);
    assert!(
        !consecutive_user,
        "feedback merges into the trailing user message"
    );
}

#[tokio::test]
async fn a_turn_that_keeps_hitting_the_limit_with_nothing_visible_fails_clearly() {
    let (mut agent, provider) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(reasoning_only_at_the_limit()),
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("never reached")),
    ]);
    let mut messages = vec![Message::user("question")];
    let error = agent
        .run_loop(&mut messages)
        .await
        .expect_err("never an empty final answer");
    assert!(error.to_string().contains("max_tokens"), "{error}");
    assert_eq!(provider.request_count(), 3, "two retries, then fail");
}

#[tokio::test]
async fn a_visible_answer_cut_at_the_limit_is_still_delivered() {
    let mut cut = text_response("a partial but visible answer");
    cut.stop_reason = Some(StopReason::MaxTokens);
    let (mut agent, provider) = agent(vec![Ok(cut)]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "a partial but visible answer");
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn only_an_output_limit_stop_is_retried() {
    // A reasoning-only reply that ended normally is not a cut-off (#2124's
    // scope): it is delivered as it was, with one request.
    let mut ended = reasoning_only_at_the_limit();
    ended.stop_reason = Some(StopReason::EndTurn);
    let (mut agent, provider) = agent(vec![Ok(ended), Ok(text_response("unused"))]);
    let mut messages = vec![Message::user("question")];
    agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn a_cut_off_reply_with_a_tool_call_is_not_treated_as_empty() {
    let mut cut = text_response("");
    cut.content = None;
    cut.stop_reason = Some(StopReason::MaxTokens);
    cut.tool_calls = vec![crate::domain::message::ToolCall {
        id: "c1".into(),
        name: "missing".into(),
        arguments: "{}".into(),
    }];
    let (mut agent, provider) = agent(vec![Ok(cut), Ok(text_response("done"))]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "done");
    let asked = messages.iter().any(|m| m.content.contains("output limit"));
    assert!(
        !asked,
        "a tool call is handled as a tool call, not as no answer"
    );
    assert_eq!(provider.request_count(), 2);
}
