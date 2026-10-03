//! #2435: the decorator records a definitive refusal against the provider
//! and model the router sent the request to, once, and passes every reply
//! through unchanged.

use std::sync::Mutex;

use super::*;
use crate::application::catalogue::CatalogueSnapshotStore;
use crate::domain::catalogue::ModelRef;
use crate::domain::message::Message;
use crate::infrastructure::providers::router::ProviderRouter;

const REFUSAL: &str = r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#;
const DETAIL: &str = "The 'mini' model is not supported when using Codex with a ChatGPT account.";

/// A provider that fails every request with `error`.
#[derive(Debug)]
struct Failing {
    name: &'static str,
    error: &'static str,
}

impl LlmProvider for Failing {
    fn route_order(&self) -> Vec<String> {
        vec![self.name.to_string()]
    }
    fn name(&self) -> &str {
        self.name
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let error = self.error.to_string();
        Box::pin(async move { Err(DomainError::Provider(error)) })
    }
}

/// A sink that counts what it is told, beside the store it forwards to.
#[derive(Debug)]
struct Counting {
    store: CatalogueSnapshotStore,
    calls: Mutex<Vec<(String, String)>>,
}

impl ModelRefusalSink for Counting {
    fn record_refusal(&self, reference: &ModelRef, reason: &str) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push((reference.qualified_id(), reason.to_string()));
        self.store.record_refusal(reference, reason)
    }
}

fn request<'a>(messages: &'a [Message], model: &'a str) -> ChatRequest<'a> {
    ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model,
        max_tokens: 64,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

fn decorated(error: &'static str) -> (RefusalRecordingProvider, Arc<Counting>) {
    let router = ProviderRouter::new(vec![
        Arc::new(Failing {
            name: "openai-oauth",
            error,
        }),
        Arc::new(Failing {
            name: "openai-api",
            error: "HTTP 500 from OpenAI: overloaded",
        }),
    ]);
    let sink = Arc::new(Counting {
        store: CatalogueSnapshotStore::empty(),
        calls: Mutex::new(Vec::new()),
    });
    (
        RefusalRecordingProvider::new(Arc::new(router), sink.clone()),
        sink,
    )
}

fn refused(sink: &Counting, qualified: &str) -> Option<String> {
    sink.store
        .refusal(&ModelRef::parse_qualified(qualified).unwrap())
}

#[tokio::test]
async fn a_refusal_is_recorded_against_the_provider_and_model_it_was_sent_to() {
    let (provider, sink) = decorated(REFUSAL);
    let messages = [Message::user("hi")];
    let error = provider
        .chat(request(&messages, "openai-oauth/mini"))
        .await
        .unwrap_err();
    assert!(
        matches!(&error, DomainError::Provider(message) if message == REFUSAL),
        "unchanged: {error:?}"
    );
    assert_eq!(refused(&sink, "openai-oauth/mini").as_deref(), Some(DETAIL));
    assert_eq!(refused(&sink, "openai-api/mini"), None);
}

#[tokio::test]
async fn a_bare_id_is_recorded_against_the_provider_the_router_chose() {
    let (provider, sink) = decorated(REFUSAL);
    let messages = [Message::user("hi")];
    let _ = provider.chat_stream(request(&messages, "mini")).await;
    assert_eq!(refused(&sink, "openai-oauth/mini").as_deref(), Some(DETAIL));
}

#[tokio::test]
async fn a_streamed_refusal_is_recorded_and_the_events_pass_through() {
    let (provider, sink) = decorated(REFUSAL);
    let messages = [Message::user("hi")];
    let mut events = provider
        .chat_stream_incremental(request(&messages, "openai-oauth/mini"))
        .await;
    let Some(StreamEvent::Error(message)) = events.recv().await else {
        panic!("the error event passes through");
    };
    assert!(message.contains(DETAIL), "{message}");
    assert!(events.recv().await.is_none(), "the stream ends after it");
    assert_eq!(refused(&sink, "openai-oauth/mini").as_deref(), Some(DETAIL));
}

#[tokio::test]
async fn the_same_refusal_twice_is_recorded_once() {
    let (provider, sink) = decorated(REFUSAL);
    let messages = [Message::user("hi")];
    for _ in 0..3 {
        let _ = provider.chat(request(&messages, "openai-oauth/mini")).await;
    }
    assert_eq!(
        sink.store
            .refusal(&ModelRef::parse_qualified("openai-oauth/mini").unwrap())
            .as_deref(),
        Some(DETAIL)
    );
    assert!(
        !sink.store.record_refusal(
            &ModelRef::parse_qualified("openai-oauth/mini").unwrap(),
            "again"
        ),
        "already held"
    );
}

#[tokio::test]
async fn other_failures_and_unroutable_models_record_nothing() {
    let (provider, sink) = decorated(r#"HTTP 400 from Codex: {"detail":"bad tool schema"}"#);
    let messages = [Message::user("hi")];
    let _ = provider.chat(request(&messages, "openai-oauth/mini")).await;
    let _ = provider.chat(request(&messages, "openai-api/mini")).await;
    let _ = provider.chat(request(&messages, "elsewhere/mini")).await;
    assert!(sink.calls.lock().unwrap().is_empty());
}

#[test]
fn the_decorator_routes_as_its_router_does() {
    let (provider, _) = decorated(REFUSAL);
    assert_eq!(provider.route_order(), vec!["openai-oauth", "openai-api"]);
    assert_eq!(provider.name(), "router");
    assert!(matches!(
        provider.route_check("elsewhere/x"),
        RouteCheck::UnknownProvider { .. }
    ));
}
