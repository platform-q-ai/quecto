use super::*;

#[derive(Debug)]
struct DummyProvider;

impl LlmProvider for DummyProvider {
    fn name(&self) -> &str {
        "dummy"
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        Box::pin(async { Err(DomainError::Provider("dummy".into())) })
    }
}

#[derive(Debug)]
struct OkProvider;

impl LlmProvider for OkProvider {
    fn name(&self) -> &str {
        "ok"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("hello".to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            })
        })
    }
}

fn request<'a>(messages: &'a [Message], tools: &'a [ToolDefinition]) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools,
        model: "dummy-model",
        max_tokens: 1,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[test]
fn effort_levels_list_renders_empty_single_and_multiple_slices() {
    assert_eq!(EffortLevel::levels_list(&[]), "");
    assert_eq!(EffortLevel::levels_list(&[EffortLevel::Max]), "max");
    assert_eq!(
        EffortLevel::levels_list(&[
            EffortLevel::None,
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
        ]),
        "none, low, medium, high, xhigh"
    );
}

#[test]
fn effort_levels_for_model_selects_anthropic_or_openai_scale() {
    assert_eq!(
        EffortLevel::levels_for_model("anthropic-api/claude-sonnet-4.6"),
        EffortLevel::ANTHROPIC_LEVELS
    );
    assert_eq!(
        EffortLevel::levels_for_model("claude-opus-4.6"),
        EffortLevel::ANTHROPIC_LEVELS
    );
    assert_eq!(
        EffortLevel::levels_for_model("openai-api/gpt-5.6"),
        EffortLevel::OPENAI_LEVELS
    );
}

#[tokio::test]
async fn dummy_provider_trait_surface_uses_name_chat_and_default_streams() {
    let provider = DummyProvider;
    let messages = [];
    let tools = [];

    assert_eq!(provider.name(), "dummy");
    assert!(provider.as_any().is::<()>());
    let err = provider
        .chat(request(&messages, &tools))
        .await
        .expect_err("dummy fails");
    assert!(err.to_string().contains("dummy"));

    let stream_err = provider
        .chat_stream(request(&messages, &tools))
        .await
        .expect_err("default stream delegates to chat");
    assert!(stream_err.to_string().contains("dummy"));

    let mut rx = provider
        .chat_stream_incremental(request(&messages, &tools))
        .await;
    match rx.recv().await.expect("terminal event") {
        StreamEvent::Error(e) => assert!(e.contains("dummy")),
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(rx.recv().await.is_none());
}

#[tokio::test]
async fn default_incremental_stream_emits_done_for_successful_chat_stream() {
    let provider = OkProvider;
    let messages = [];
    let tools = [];

    assert!(provider.as_any().is::<OkProvider>());
    let direct = provider
        .chat_stream(request(&messages, &tools))
        .await
        .unwrap();
    assert_eq!(direct.content.as_deref(), Some("hello"));

    let mut rx = provider
        .chat_stream_incremental(request(&messages, &tools))
        .await;
    match rx.recv().await.expect("done event") {
        StreamEvent::Done(resp) => assert_eq!(resp.content.as_deref(), Some("hello")),
        other => panic!("unexpected event: {other:?}"),
    }
    assert!(rx.recv().await.is_none());
}
