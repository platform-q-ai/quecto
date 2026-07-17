use super::*;

#[tokio::test]
async fn retries_retryable_provider_failures_before_returning_success() {
    // Non-streaming transient retry is owned by the `RetryingProvider`
    // decorator (composed over the router in `build_agent_provider`); the agent
    // loop no longer double-retries the non-streaming path. Compose the decorator
    // here so this end-to-end test exercises the real production stack.
    let provider = Arc::new(MockProvider::new_results(vec![
        Err(DomainError::Provider(
            "HTTP 503 Service Unavailable".to_string(),
        )),
        Ok(text_response("recovered")),
    ]));
    let retrying = Arc::new(
        crate::infrastructure::providers::retry::RetryingProvider::new(
            provider.clone(),
            crate::infrastructure::providers::retry::RetryConfig::no_delay(4),
        ),
    );
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: retrying,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "retry-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "recovered");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn retries_streaming_provider_failures_before_any_output() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![crate::domain::provider::StreamEvent::Error(
            "HTTP 503 from Codex: connection refused".to_string(),
        )],
        vec![crate::domain::provider::StreamEvent::Done(text_response(
            "stream recovered",
        ))],
    ]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "stream-retry-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "stream recovered");
    assert_eq!(provider.request_count(), 2);
}

#[tokio::test]
async fn does_not_retry_streaming_provider_failures_after_output() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::TextDelta("partial".to_string()),
        crate::domain::provider::StreamEvent::Error("HTTP 503 from Codex".to_string()),
    ]]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "stream-no-retry-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("HTTP 503 from Codex"), "{err}");
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn provider_context_limit_errors_are_actionable() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        "HTTP 400 from OpenAI: maximum context length is 100000 tokens; requested 100001 tokens"
            .to_string(),
    ))]));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 8192,
        temperature: 0.0,
        spill_store: None,
        session_key: "limit-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(
        err.to_ascii_lowercase().contains("context/output limit"),
        "{err}"
    );
    assert!(err.contains("reducing prompt history"), "{err}");
    assert!(err.contains("max output tokens"), "{err}");
}
