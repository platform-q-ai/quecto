//! #2124: a reply that hits the output limit with nothing visible (only
//! reasoning) is not a final answer: the model is asked again, concisely,
//! and a turn that keeps doing it fails instead of ending silently.
use super::*;
use crate::domain::message::{StopReason, ThinkingBlock};

fn reasoning_only_at_the_limit() -> LlmResponse {
    let mut response = text_response("");
    response.content = None;
    response.stop_reason = Some(StopReason::MaxTokens);
    if let Some(usage) = response.usage.as_mut() {
        usage.completion_tokens = 8192;
    }
    response.thinking_blocks = vec![ThinkingBlock::Normal {
        thinking: "let me think about this at great length".into(),
        signature: String::new(),
    }];
    response
}

fn agent(responses: Vec<Result<LlmResponse, DomainError>>) -> (AgentLoopImpl, Arc<MockProvider>) {
    agent_with(responses, false)
}

fn agent_with(
    responses: Vec<Result<LlmResponse, DomainError>>,
    streaming: bool,
) -> (AgentLoopImpl, Arc<MockProvider>) {
    agent_sized(responses, streaming, 190_000)
}

fn agent_sized(
    responses: Vec<Result<LlmResponse, DomainError>>,
    streaming: bool,
    max_context_tokens: usize,
) -> (AgentLoopImpl, Arc<MockProvider>) {
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
        max_context_tokens,
        progress_callback: None,
        streaming,
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
        Ok(text_response("never reached")),
    ]);
    let mut messages = vec![Message::user("question")];
    let error = agent
        .run_loop(&mut messages)
        .await
        .expect_err("never an empty final answer");
    assert!(error.to_string().contains("max_tokens"), "{error}");
    assert_eq!(provider.request_count(), 2, "one retry, then fail");
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

#[tokio::test]
async fn the_retry_may_use_the_model_cap_and_later_requests_do_not() {
    let (agent, provider) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    // Configured 8192, the model declares 32768: the retry may go up to
    // twice the configured limit (a cost ceiling), not the full cap.
    let mut agent = agent.with_model_max_tokens(Some(32_768));
    let mut messages = vec![Message::user("question")];
    agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(provider.seen_max_tokens(), [8192, 16_384]);
}

#[tokio::test]
async fn the_ledger_never_records_the_owners_prompt_as_run_added() {
    // #1072: feedback merged into the owner's prompt is not a message the
    // run added, so it must not appear in (or be duplicated into) the ledger.
    let (mut agent, _) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    let users = result
        .appended_messages
        .iter()
        .filter(|m| m.role == Role::User)
        .count();
    assert_eq!(users, 0, "{:?}", result.appended_messages);
    assert_eq!(
        result.appended_messages.last().unwrap().content,
        "the answer"
    );
}

#[tokio::test]
async fn whitespace_only_text_at_the_limit_counts_as_nothing_visible() {
    let mut blank = reasoning_only_at_the_limit();
    blank.content = Some("  \n ".into());
    let (mut agent, provider) = agent(vec![Ok(blank), Ok(text_response("the answer"))]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn the_streaming_path_retries_a_reasoning_only_cut_off_too() {
    let (mut agent, provider) = agent_with(
        vec![
            Ok(reasoning_only_at_the_limit()),
            Ok(text_response("the answer")),
        ],
        true,
    );
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn a_full_context_window_reported_as_max_tokens_is_not_an_output_cut_off() {
    // Barely any output was produced, so the output limit was not the cause.
    let mut context_full = reasoning_only_at_the_limit();
    if let Some(usage) = context_full.usage.as_mut() {
        usage.completion_tokens = 12;
    }
    let (mut agent, provider) = agent(vec![Ok(context_full), Ok(text_response("unused"))]);
    let mut messages = vec![Message::user("question")];
    agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn the_retry_budget_counts_consecutive_cut_offs_not_the_whole_run() {
    let mut tool_call = text_response("");
    tool_call.content = None;
    tool_call.tool_calls = vec![crate::domain::message::ToolCall {
        id: "c1".into(),
        name: "missing".into(),
        arguments: "{}".into(),
    }];
    let (mut agent, provider) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(tool_call),
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent
        .run_loop(&mut messages)
        .await
        .expect("each cut-off gets its retry");
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 4);
}

fn missing_tool_call() -> LlmResponse {
    let mut call = text_response("");
    call.content = None;
    call.tool_calls = vec![crate::domain::message::ToolCall {
        id: "c1".into(),
        name: "missing".into(),
        arguments: "{}".into(),
    }];
    call
}

fn feedback_in_ledger(result: &crate::domain::agent::AgentResult) -> Vec<String> {
    result
        .appended_messages
        .iter()
        .filter(|m| m.role == Role::User)
        .map(|m| m.content.clone())
        .collect()
}

#[tokio::test]
async fn feedback_the_run_adds_after_tool_results_is_in_the_ledger_once() {
    let (mut agent, _) = agent(vec![
        Ok(missing_tool_call()),
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    let feedback = feedback_in_ledger(&result);
    assert_eq!(feedback.len(), 1, "{feedback:?}");
    assert!(feedback[0].contains("output limit"));
}

#[tokio::test]
async fn feedback_merged_into_run_added_feedback_updates_that_ledger_entry() {
    let (mut agent, _) = agent(vec![
        Ok(missing_tool_call()),
        Ok(reasoning_only_at_the_limit()),
        Err(DomainError::Provider(
            "provider error (400): invalid_request_error: tool_use input is malformed".into(),
        )),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    let feedback = feedback_in_ledger(&result);
    assert_eq!(feedback.len(), 1, "one run-added message: {feedback:?}");
    assert!(
        feedback[0].contains("output limit") && feedback[0].contains("malformed"),
        "the entry carries the merged text: {feedback:?}"
    );
}

#[tokio::test]
async fn the_raised_limit_fits_what_the_context_window_leaves() {
    let (agent, provider) = agent_sized(
        vec![
            Ok(reasoning_only_at_the_limit()),
            Ok(text_response("the answer")),
        ],
        false,
        12_000,
    );
    let mut agent = agent.with_model_max_tokens(Some(32_768));
    let mut messages = vec![Message::user("question")];
    agent.run_loop(&mut messages).await.unwrap();
    let sent = provider.seen_max_tokens();
    assert_eq!(sent[0], 8192);
    assert!(sent[1] > 8192 && sent[1] < 12_000 - 1024, "{sent:?}");
}

#[tokio::test]
async fn the_dropped_replys_tokens_are_still_counted() {
    let (mut agent, _) = agent(vec![
        Ok(reasoning_only_at_the_limit()),
        Ok(text_response("the answer")),
    ]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert!(
        result.output_tokens >= 8192 + 20,
        "{}",
        result.output_tokens
    );
}

#[tokio::test]
async fn an_unreported_output_count_is_treated_as_a_cut_off() {
    let mut unreported = reasoning_only_at_the_limit();
    if let Some(usage) = unreported.usage.as_mut() {
        usage.completion_tokens = 0;
    }
    let (mut agent, provider) = agent(vec![Ok(unreported), Ok(text_response("the answer"))]);
    let mut messages = vec![Message::user("question")];
    let result = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(result.response, "the answer");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn a_retry_reply_is_judged_against_the_limit_that_retry_was_sent_with() {
    // The retry asked for 16384; 5000 output tokens is under half of that,
    // so it is not an output cut-off (a full context reported as max_tokens).
    let mut context_full = reasoning_only_at_the_limit();
    if let Some(usage) = context_full.usage.as_mut() {
        usage.completion_tokens = 5000;
    }
    let (agent, provider) = agent(vec![Ok(reasoning_only_at_the_limit()), Ok(context_full)]);
    let mut agent = agent.with_model_max_tokens(Some(32_768));
    let mut messages = vec![Message::user("question")];
    agent
        .run_loop(&mut messages)
        .await
        .expect("not failed as a cut-off");
    assert_eq!(provider.request_count(), 2);
}
