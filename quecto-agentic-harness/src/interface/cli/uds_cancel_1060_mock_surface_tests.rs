use super::issue_1060_tests::{ScriptedProvider, StreamingProvider};
use crate::domain::message::{LlmResponse, Message};

fn text_response(content: &str) -> LlmResponse {
    LlmResponse {
        content: Some(content.to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use std::sync::Mutex;

fn request(messages: &[Message]) -> ChatRequest<'_> {
    ChatRequest {
        messages,
        tools: &[],
        model: "stub",
        max_tokens: 16,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

#[tokio::test]
async fn scripted_provider_covers_name_chat_stream_incremental_and_as_any() {
    let provider = ScriptedProvider {
        responses: Mutex::new(vec![text_response("ok")]),
    };
    assert_eq!(provider.name(), "scripted-1060");
    assert!(
        provider
            .as_any()
            .downcast_ref::<ScriptedProvider>()
            .is_none()
    );
    assert_eq!(
        provider
            .chat(request(&[]))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("ok")
    );

    let provider = ScriptedProvider {
        responses: Mutex::new(vec![text_response("stream")]),
    };
    let mut rx = provider.chat_stream_incremental(request(&[])).await;
    match rx.recv().await.expect("terminal event") {
        StreamEvent::Done(response) => assert_eq!(response.content.as_deref(), Some("stream")),
        other => panic!("expected done from default stream wrapper, got {other:?}"),
    }
}

#[tokio::test]
async fn streaming_provider_covers_name_chat_and_incremental_events() {
    let provider = StreamingProvider {
        deltas: vec!["a".into(), "b".into()],
        response: text_response("ab"),
    };
    assert_eq!(provider.name(), "streaming-1060");
    assert_eq!(
        provider
            .chat(request(&[]))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("ab")
    );

    let mut rx = provider.chat_stream_incremental(request(&[])).await;
    assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(s)) if s == "a"));
    assert!(matches!(rx.recv().await, Some(StreamEvent::TextDelta(s)) if s == "b"));
    assert!(
        matches!(rx.recv().await, Some(StreamEvent::Done(resp)) if resp.content.as_deref() == Some("ab"))
    );
}
