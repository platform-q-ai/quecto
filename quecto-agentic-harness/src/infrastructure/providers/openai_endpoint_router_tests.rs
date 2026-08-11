use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider, StreamEvent};
use crate::infrastructure::providers::openai_endpoint_router::OpenAiEndpointRouter;

#[derive(Debug)]
struct RecordingProvider {
    name: &'static str,
    calls: Arc<Mutex<Vec<&'static str>>>,
}

fn response(content: impl Into<String>) -> LlmResponse {
    LlmResponse {
        content: Some(content.into()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

impl LlmProvider for RecordingProvider {
    fn name(&self) -> &str {
        self.name
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let calls = self.calls.clone();
        let name = self.name;
        Box::pin(async move {
            calls.lock().unwrap().push(name);
            Ok(response(name))
        })
    }

    fn chat_stream<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let calls = self.calls.clone();
        let name = self.name;
        Box::pin(async move {
            calls.lock().unwrap().push(name);
            Ok(response(format!("stream-{name}")))
        })
    }

    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let calls = self.calls.clone();
        let name = self.name;
        Box::pin(async move {
            calls.lock().unwrap().push(name);
            let (_tx, rx) = tokio::sync::mpsc::channel(1);
            rx
        })
    }
}

fn request(model: &str) -> ChatRequest<'_> {
    ChatRequest {
        messages: &[],
        tools: &[],
        model,
        max_tokens: 100,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

fn router(calls: Arc<Mutex<Vec<&'static str>>>) -> OpenAiEndpointRouter {
    OpenAiEndpointRouter::new(
        "router".to_string(),
        Arc::new(RecordingProvider {
            name: "chat",
            calls: calls.clone(),
        }),
        Arc::new(RecordingProvider {
            name: "responses",
            calls,
        }),
        HashSet::from(["o3".to_string()]),
    )
}

#[tokio::test]
async fn endpoint_router_routes_all_surfaces_by_reasoning_model() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let router = router(calls.clone());
    assert_eq!(router.name(), "router");

    assert_eq!(
        router
            .chat(request("gpt-4.1"))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("chat")
    );
    assert_eq!(
        router.chat(request("o3")).await.unwrap().content.as_deref(),
        Some("responses")
    );
    assert_eq!(
        router
            .chat_stream(request("gpt-4.1"))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("stream-chat")
    );
    assert_eq!(
        router
            .chat_stream(request("o3"))
            .await
            .unwrap()
            .content
            .as_deref(),
        Some("stream-responses")
    );
    let _ = router.chat_stream_incremental(request("gpt-4.1")).await;
    let _ = router.chat_stream_incremental(request("o3")).await;

    assert_eq!(
        calls.lock().unwrap().as_slice(),
        &[
            "chat",
            "responses",
            "chat",
            "responses",
            "chat",
            "responses"
        ]
    );
}
