use super::*;
use crate::domain::message::{LlmResponse, Message, UsageInfo};
use crate::domain::provider::StreamEvent;
use std::sync::Arc;

fn response_with_prompt_tokens(content: &str, prompt_tokens: u32) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens,
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
async fn non_streaming_result_context_tokens_uses_provider_prompt_tokens() {
    let (agent, _) = make_agent(vec![response_with_prompt_tokens("hello", 2_246)], vec![]);
    let mut messages = vec![Message::user("Hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(result.context_tokens, 2_246);
    assert_eq!(result.input_tokens, 2_246);
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
async fn streaming_result_context_tokens_uses_provider_prompt_tokens() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![StreamEvent::Done(
        response_with_prompt_tokens("hello", 2_246),
    )]]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        system_prompt_provider: None,
        audit_log: None,
    });
    let mut messages = vec![Message::user("Hi")];

    let result = agent.run_loop(&mut messages).await.unwrap();

    assert_eq!(result.context_tokens, 2_246);
    assert_eq!(result.input_tokens, 2_246);
}
