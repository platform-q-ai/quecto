use super::*;
use crate::domain::agent::AgentProgressEvent;
use crate::domain::message::{LlmResponse, Message, UsageInfo};
use crate::domain::provider::StreamEvent;
use std::sync::Arc;

fn response_with_provider_input_tokens(content: &str, provider_input_tokens: u32) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: provider_input_tokens,
            completion_tokens: 7,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_tokens: None,
            cost: None,
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[tokio::test]
async fn non_streaming_result_context_tokens_uses_provider_reported_occupancy() {
    let (agent, _) = make_agent(
        vec![response_with_provider_input_tokens("hello", 280_000)],
        vec![],
    );
    let mut messages = vec![Message::user("Hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();
    let active_conversation_tokens = context_pruning::estimate_total_tokens(&messages);

    assert_eq!(
        result.context_tokens, 280_000,
        "context gauge must use provider-reported prompt occupancy, not the smaller char heuristic ({active_conversation_tokens})"
    );
    assert_eq!(result.input_tokens, 280_000);
    assert_eq!(result.billed_input_tokens, 280_000);
}

#[tokio::test]
async fn non_streaming_result_context_tokens_falls_back_to_estimate_without_usage() {
    let (agent, _) = make_agent(
        vec![LlmResponse {
            content: Some("hello".to_string()),
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        }],
        vec![],
    );
    let mut messages = vec![Message::user("Hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(
        result.context_tokens,
        context_pruning::estimate_total_tokens(&messages)
    );
}

#[tokio::test]
async fn streaming_result_context_tokens_uses_provider_reported_occupancy() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![StreamEvent::Done(
        response_with_provider_input_tokens("hello", 280_000),
    )]]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });
    let mut messages = vec![Message::user("Hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();
    let active_conversation_tokens = context_pruning::estimate_total_tokens(&messages);

    assert_eq!(
        result.context_tokens, 280_000,
        "context gauge must use provider-reported prompt occupancy, not the smaller char heuristic ({active_conversation_tokens})"
    );
    assert_eq!(result.input_tokens, 280_000);
    assert_eq!(result.billed_input_tokens, 280_000);
}

#[tokio::test]
async fn thinking_context_tokens_carry_provider_truth_forward_across_collapse() {
    let (agent, _) = make_agent(
        vec![
            response_with_provider_input_tokens("first", 10_000),
            response_with_provider_input_tokens("second", 8_000),
        ],
        vec![],
    );
    let events = Arc::new(std::sync::Mutex::new(Vec::new()));
    let captured = Arc::clone(&events);
    let agent = agent.with_progress_callback(Some(Arc::new(move |event| {
        if let AgentProgressEvent::Thinking { context_tokens, .. } = event {
            captured.lock().unwrap().push(context_tokens);
        }
    })));
    let mut messages = vec![Message::user("x".repeat(400))];

    let first = agent.run_loop(&mut messages).await.unwrap();
    assert_eq!(first.context_tokens, 10_000);

    let before_collapse = context_pruning::estimate_total_tokens(&messages);
    messages[0].content = "x".to_string();
    messages[0].invalidate_token_cache();
    let after_collapse = context_pruning::estimate_total_tokens(&messages);
    assert!(after_collapse < before_collapse);

    let second = agent.run_loop(&mut messages).await.unwrap();

    let thinking_values = events.lock().unwrap().clone();
    assert_eq!(
        thinking_values[1],
        10_000usize.saturating_sub(thinking_values[0].saturating_sub(after_collapse)),
        "pre-call Thinking gauge should reconcile provider truth by the estimate delta since the provider-reported call"
    );
    assert_eq!(
        second.context_tokens, 8_000,
        "next provider usage report should replace calibrated value with exact provider truth"
    );
}
