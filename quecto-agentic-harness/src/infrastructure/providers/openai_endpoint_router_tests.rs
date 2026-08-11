use super::openai_endpoint_router::*;
use crate::domain::error::DomainError;
use crate::domain::message::{LlmResponse, Message};
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

#[derive(Debug)]
struct CountingProvider {
    stream_calls: AtomicUsize,
    incremental_calls: AtomicUsize,
}

impl CountingProvider {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            stream_calls: AtomicUsize::new(0),
            incremental_calls: AtomicUsize::new(0),
        })
    }
}

impl LlmProvider for CountingProvider {
    fn name(&self) -> &str {
        "counting"
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("ok".to_string()),
                tool_calls: Vec::new(),
                usage: None,
                stop_reason: None,
                thinking_blocks: Vec::new(),
            })
        })
    }

    fn chat_stream<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        self.stream_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            Ok(LlmResponse {
                content: Some("stream".to_string()),
                tool_calls: Vec::new(),
                usage: None,
                stop_reason: None,
                thinking_blocks: Vec::new(),
            })
        })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        self.incremental_calls.fetch_add(1, Ordering::SeqCst);
        Box::pin(async {
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            rx
        })
    }
}

fn request<'a>(model: &'a str, messages: &'a [Message]) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools: &[],
        model,
        max_tokens: 128,
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
async fn streaming_routes_reasoning_models_to_responses_and_others_to_chat() {
    let chat = CountingProvider::new();
    let responses = CountingProvider::new();
    let router = OpenAiEndpointRouter::new(
        "openai".to_string(),
        chat.clone(),
        responses.clone(),
        ["o3".to_string()].into_iter().collect(),
    );

    let messages = vec![Message::user("hi")];
    router
        .chat_stream(request("gpt-4o", &messages))
        .await
        .unwrap();
    router.chat_stream(request("o3", &messages)).await.unwrap();
    router
        .chat_stream_incremental(request("gpt-4o", &messages))
        .await;
    router
        .chat_stream_incremental(request("o3", &messages))
        .await;

    assert_eq!(chat.stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(responses.stream_calls.load(Ordering::SeqCst), 1);
    assert_eq!(chat.incremental_calls.load(Ordering::SeqCst), 1);
    assert_eq!(responses.incremental_calls.load(Ordering::SeqCst), 1);
}
