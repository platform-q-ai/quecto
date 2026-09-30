use super::*;
use crate::domain::message::StopReason;

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
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: retrying,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "stream recovered");
    assert_eq!(provider.request_count(), 2);
    let observations = agent.take_request_observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].instrumented_attempts, 2);
    assert!(observations[0].attempt_diagnostics.is_empty());
}

#[tokio::test]
async fn does_not_retry_streaming_provider_failures_after_output() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::TextDelta("partial".to_string()),
        crate::domain::provider::StreamEvent::Error("HTTP 503 from Codex".to_string()),
    ]]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("HTTP 503 from Codex"), "{err}");
    assert_eq!(provider.request_count(), 1);
}

#[tokio::test]
async fn does_not_retry_non_streaming_openai_insufficient_quota_429() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        r#"HTTP 429 from OpenAI: {"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#.to_string(),
    ))]));
    let retrying = Arc::new(
        crate::infrastructure::providers::retry::RetryingProvider::new(
            provider.clone(),
            crate::infrastructure::providers::retry::RetryConfig::no_delay(4),
        ),
    );
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: retrying,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "quota-non-stream-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("HTTP 429 from OpenAI"), "{err}");
    assert!(err.contains("insufficient_quota"), "{err}");
    assert_eq!(provider.request_count(), 1, "quota failures are terminal");
}

#[tokio::test]
async fn does_not_retry_streaming_openai_insufficient_quota_429() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::Error(
            r#"HTTP 429 from OpenAI: {"error":{"message":"You exceeded your current quota, please check your plan and billing details.","type":"insufficient_quota","param":null,"code":"insufficient_quota"}}"#.to_string(),
        ),
    ]]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "quota-stream-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("HTTP 429 from OpenAI"), "{err}");
    assert!(err.contains("insufficient_quota"), "{err}");
    assert_eq!(provider.request_count(), 1, "quota failures are terminal");
}

#[tokio::test]
async fn provider_context_limit_errors_are_actionable() {
    let provider = Arc::new(MockProvider::new_results(vec![Err(DomainError::Provider(
        "HTTP 400 from OpenAI: maximum context length is 100000 tokens; requested 100001 tokens"
            .to_string(),
    ))]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 8192,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
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

#[tokio::test]
async fn retries_empty_streaming_done_before_success() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![crate::domain::provider::StreamEvent::Done(LlmResponse {
            content: None,
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        })],
        vec![crate::domain::provider::StreamEvent::Done(text_response(
            "stream recovered",
        ))],
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "empty-stream-done-retry-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await.unwrap();

    assert_eq!(result.response, "stream recovered");
    assert_eq!(provider.request_count(), 2);
    let observations = agent.take_request_observations();
    assert_eq!(observations.len(), 1);
    assert_eq!(observations[0].instrumented_attempts, 2);
    assert!(observations[0].attempt_diagnostics.is_empty());
}

#[tokio::test]
async fn empty_streaming_done_with_max_tokens_preserves_stop_reason() {
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        crate::domain::provider::StreamEvent::Done(LlmResponse {
            content: None,
            tool_calls: vec![],
            usage: None,
            stop_reason: Some(StopReason::MaxTokens),
            thinking_blocks: vec![],
        }),
    ]]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "empty-stream-max-tokens-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let err = agent.process(&mut messages).await.unwrap_err().to_string();

    assert!(err.contains("stop_reason=max_tokens"), "{err}");
    assert_eq!(provider.request_count(), 1);
}

#[derive(Debug)]
struct PausedAdmission;
impl crate::application::providers::ports::RequestAdmission for PausedAdmission {
    fn check(
        &self,
        _attempt: crate::domain::provider::RequestAttempt,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
    {
        Box::pin(async { Err(DomainError::Tool("swarm paused".into())) })
    }
}

#[tokio::test]
async fn paused_execution_never_calls_provider() {
    let provider = Arc::new(MockProvider::new(vec![text_response("must not run")]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1)
    .with_request_admission(Some(Arc::new(PausedAdmission)));

    let result = agent.process(&mut vec![Message::user("queued hint")]).await;
    assert!(result.is_err());
    assert_eq!(provider.request_count(), 0);
}

#[tokio::test]
async fn long_reset_horizon_prevents_stream_initiation_retry() {
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![crate::domain::provider::StreamEvent::Error(
            "HTTP 429: retry-after: 601828".to_string(),
        )],
        vec![crate::domain::provider::StreamEvent::Done(text_response(
            "stream recovered",
        ))],
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
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
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let mut messages = vec![Message::user("hello")];
    let result = agent.process(&mut messages).await;
    assert!(
        result.is_err(),
        "must not retry ahead of the provider reset"
    );
    assert_eq!(provider.request_count(), 1);
}

/// #2210 review: a stream the provider stopped sending is re-initiated once,
/// not to the full budget, and the failure says what happened rather than
/// blaming connectivity.
#[tokio::test]
async fn a_stalled_stream_is_retried_once_with_its_own_guidance() {
    use crate::domain::provider::StreamEvent;
    use crate::domain::provider_error::STREAM_IDLE_TIMEOUT;
    let stall = format!(
        "{STREAM_IDLE_TIMEOUT}the provider sent nothing for 300 s; the request was abandoned"
    );
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Error(stall.clone())],
        vec![StreamEvent::Error(stall.clone())],
        vec![StreamEvent::Done(text_response("too late"))],
    ]));
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "stall-retry-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1);

    let err = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(provider.request_count(), 2, "one retry only");
    assert!(err.contains(&stall), "{err}");
    assert!(
        err.contains("Stalled: the provider stopped sending"),
        "{err}"
    );
    assert!(!err.contains("check connectivity"), "{err}");
    assert!(!err.contains("retried"), "claims only what is known: {err}");
}

fn streaming_agent(provider: Arc<MockStreamingProvider>) -> AgentLoopImpl {
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "stall-test".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: true,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_max_tool_iterations(1)
}

/// #2210 review: a stall after output began is never replayed, and the
/// failure claims no retry.
#[tokio::test]
async fn a_stall_after_output_is_not_retried_and_claims_no_retry() {
    use crate::domain::provider::StreamEvent;
    use crate::domain::provider_error::STREAM_IDLE_TIMEOUT;
    let stall = format!("{STREAM_IDLE_TIMEOUT}the provider sent nothing for 300 s");
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![
        StreamEvent::TextDelta("partial".to_string()),
        StreamEvent::Error(stall.clone()),
    ]]));
    let mut agent = streaming_agent(provider.clone());
    let err = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(provider.request_count(), 1);
    assert!(
        err.contains("Stalled: the provider stopped sending"),
        "{err}"
    );
    assert!(!err.contains("retried"), "{err}");
}

/// #2210 review: the stall cap counts stalls, not attempts: a stall after
/// a network failure is still re-initiated once, within the budget.
#[tokio::test]
async fn a_stall_after_a_network_failure_is_still_retried_once() {
    use crate::domain::provider::StreamEvent;
    use crate::domain::provider_error::STREAM_IDLE_TIMEOUT;
    let stall = format!("{STREAM_IDLE_TIMEOUT}the provider sent nothing for 300 s");
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Error("connection reset by peer".to_string())],
        vec![StreamEvent::Error(stall)],
        vec![StreamEvent::Done(text_response("recovered"))],
    ]));
    let mut agent = streaming_agent(provider.clone());
    let result = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(result.response, "recovered");
    assert_eq!(provider.request_count(), 3);
}

/// #2155 review: a malformed-request error after output (here from a stream
/// no admission gate owns) is never repaired and sent again: one request,
/// and the turn fails with the error.
#[tokio::test]
async fn a_malformed_request_error_after_output_is_not_resent() {
    use crate::domain::provider::StreamEvent;
    let error = r#"HTTP 400 OpenAI stream error: {"error":{"type":"invalid_request_error"}}"#;
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![
            StreamEvent::TextDelta("partial".to_string()),
            StreamEvent::Error(error.to_string()),
        ],
        vec![StreamEvent::TextDelta("resent".to_string())],
    ]));
    let mut agent = streaming_agent(provider.clone());
    let err = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(provider.request_count(), 1);
    assert!(err.contains("invalid_request_error"), "{err}");
}

/// The same error before any output is still repaired and sent again.
#[tokio::test]
async fn a_malformed_request_error_before_output_is_recovered() {
    use crate::domain::provider::StreamEvent;
    let error = r#"HTTP 400 OpenAI stream error: {"error":{"type":"invalid_request_error"}}"#;
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Error(error.to_string())],
        vec![StreamEvent::Done(crate::domain::message::LlmResponse {
            content: Some("repaired".to_string()),
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        })],
    ]));
    let mut agent = streaming_agent(provider.clone());
    let result = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(provider.request_count(), 2);
    assert_eq!(result.response, "repaired");
}

/// Records the attempt of each admission check, admitting every one (#2339).
#[derive(Debug, Default)]
struct RecordingAdmission(std::sync::Mutex<Vec<crate::domain::provider::RequestAttempt>>);
impl crate::application::providers::ports::RequestAdmission for RecordingAdmission {
    fn check(
        &self,
        attempt: crate::domain::provider::RequestAttempt,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), DomainError>> + Send + '_>>
    {
        self.0.lock().unwrap().push(attempt);
        Box::pin(async { Ok(()) })
    }
}

/// #2339: the loop admits a request's first send once, and checks the
/// stream re-initiation after a transient failure as a reattempt, so the
/// admission records the two apart.
#[tokio::test]
async fn a_stream_reinitiation_is_admitted_as_a_reattempt() {
    use crate::domain::provider::{RequestAttempt, StreamEvent};
    let provider = Arc::new(MockStreamingProvider::new(vec![
        vec![StreamEvent::Error(
            "HTTP 503 from Codex: connection refused".to_string(),
        )],
        vec![StreamEvent::Done(text_response("stream recovered"))],
    ]));
    let admission = Arc::new(RecordingAdmission::default());
    let mut agent =
        streaming_agent(provider.clone()).with_request_admission(Some(admission.clone()));
    let result = agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(result.response, "stream recovered");
    assert_eq!(provider.request_count(), 2);
    assert_eq!(
        *admission.0.lock().unwrap(),
        [RequestAttempt::First, RequestAttempt::Reattempt]
    );
}

/// #2339: a request answered at its first send is admitted exactly once,
/// streaming or not.
#[tokio::test]
async fn a_request_answered_first_time_is_admitted_once() {
    use crate::domain::provider::{RequestAttempt, StreamEvent};
    let provider = Arc::new(MockStreamingProvider::new(vec![vec![StreamEvent::Done(
        text_response("first time"),
    )]]));
    let admission = Arc::new(RecordingAdmission::default());
    let mut agent =
        streaming_agent(provider.clone()).with_request_admission(Some(admission.clone()));
    agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(*admission.0.lock().unwrap(), [RequestAttempt::First]);
    let provider = Arc::new(MockProvider::new(vec![text_response("first time")]));
    let admission = Arc::new(RecordingAdmission::default());
    let mut agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: provider.clone(),
        tool_registry: Box::new(MockRegistry::default()),
        model: "test".into(),
        max_tokens: 1024,
        temperature: 0.0,
        retention: None,
        session_key: "admitted-once".into(),
        context_collapse_after_tool_calls: context_pruning::COLLAPSE_DISABLED,
        max_context_tokens: 100_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
        tool_profile_context: crate::domain::tool::ToolProfileContext::Parent,
    })
    .with_request_admission(Some(admission.clone()));
    agent
        .process(&mut vec![Message::user("hello")])
        .await
        .unwrap();
    assert_eq!(provider.request_count(), 1);
    assert_eq!(*admission.0.lock().unwrap(), [RequestAttempt::First]);
}
