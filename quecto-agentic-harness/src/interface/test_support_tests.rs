use super::*;

#[test]
fn stub_provider_reports_name() {
    let provider = StubProvider;
    assert_eq!(provider.name(), "stub");
}

#[tokio::test]
async fn stub_provider_trait_methods_are_scripted() {
    let provider = make_stub_provider();
    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };

    assert!(provider.as_any().downcast_ref::<StubProvider>().is_some());
    let response = provider.chat(request).await.unwrap();
    assert_eq!(response.content.as_deref(), Some("stub response"));
    assert!(response.tool_calls.is_empty());
    assert!(response.usage.is_none());
    assert!(response.stop_reason.is_none());
    assert!(response.thinking_blocks.is_empty());

    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let streamed = provider.chat_stream(request).await.unwrap();
    assert_eq!(streamed.content.as_deref(), Some("stub response"));

    let messages = [];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "stub-model",
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let mut rx = provider.chat_stream_incremental(request).await;
    assert!(
        matches!(rx.recv().await, Some(crate::domain::provider::StreamEvent::Done(done)) if done.content.as_deref() == Some("stub response"))
    );
    assert!(rx.recv().await.is_none());
}
