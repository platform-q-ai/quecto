//! #2435: the decorator records a definitive refusal against the provider
//! and model the router sent the request to, once, and passes every reply
//! through unchanged.

use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::application::catalogue::CatalogueSnapshotStore;
use crate::domain::catalogue::ModelRef;
use crate::domain::message::Message;
use crate::infrastructure::providers::router::ProviderRouter;

const REFUSAL: &str = r#"HTTP 400 from Codex: {"detail":"The 'mini' model is not supported when using Codex with a ChatGPT account."}"#;
const DETAIL: &str = "The 'mini' model is not supported when using Codex with a ChatGPT account.";
const HELD: Duration = Duration::from_secs(3600);

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
    cleared: Mutex<Vec<String>>,
}

fn counting() -> Arc<Counting> {
    Arc::new(Counting {
        store: CatalogueSnapshotStore::empty(),
        calls: Mutex::new(Vec::new()),
        cleared: Mutex::new(Vec::new()),
    })
}

impl ModelRefusalSink for Counting {
    fn record_refusal(&self, reference: &ModelRef, reason: &str, held_for: Duration) -> bool {
        self.calls
            .lock()
            .unwrap()
            .push((reference.qualified_id(), reason.to_string()));
        self.store.record_refusal(reference, reason, held_for)
    }

    fn clear_refusal(&self, reference: &ModelRef) -> bool {
        self.cleared.lock().unwrap().push(reference.qualified_id());
        self.store.clear_refusal(reference)
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
    let sink = counting();
    (
        RefusalRecordingProvider::new(Arc::new(router), sink.clone(), HELD),
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
            "again",
            HELD
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

// ── Review round 1 (#2435) ──────────────────────────────────────────────────

/// A provider whose replies follow a script: each request takes the next
/// one, and an incremental stream hands its sender to the test.
#[derive(Debug)]
struct Scripted {
    replies: Mutex<std::collections::VecDeque<Result<&'static str, &'static str>>>,
    senders: Arc<Mutex<Vec<tokio::sync::mpsc::Sender<StreamEvent>>>>,
}

impl Scripted {
    fn new(replies: Vec<Result<&'static str, &'static str>>) -> Arc<Self> {
        Arc::new(Self {
            replies: Mutex::new(replies.into()),
            senders: Arc::new(Mutex::new(Vec::new())),
        })
    }

    fn next(&self) -> Result<LlmResponse, DomainError> {
        match self
            .replies
            .lock()
            .unwrap()
            .pop_front()
            .expect("a scripted reply")
        {
            Ok(text) => Ok(LlmResponse {
                content: Some(text.to_string()),
                tool_calls: vec![],
                usage: None,
                stop_reason: None,
                thinking_blocks: vec![],
            }),
            Err(error) => Err(DomainError::Provider(error.to_string())),
        }
    }
}

impl LlmProvider for Scripted {
    fn route_order(&self) -> Vec<String> {
        vec!["openai-oauth".to_string()]
    }
    fn name(&self) -> &str {
        "openai-oauth"
    }
    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>> {
        let reply = self.next();
        Box::pin(async move { reply })
    }
    fn chat_stream_incremental<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> Pin<Box<dyn Future<Output = tokio::sync::mpsc::Receiver<StreamEvent>> + Send + 'a>> {
        let (tx, rx) = tokio::sync::mpsc::channel(4);
        self.senders.lock().unwrap().push(tx);
        Box::pin(async move { rx })
    }
}

/// M1: a caller that drops its stream must reach the provider: the relay
/// lets go of the provider's stream at once, so the transport sees its
/// receiver closed and stops (no admission permit, no billed request).
#[tokio::test]
async fn dropping_the_receiver_closes_the_providers_stream() {
    let inner = Scripted::new(vec![]);
    let provider = RefusalRecordingProvider::new(inner.clone(), counting(), HELD);
    let messages = [Message::user("hi")];
    let events = provider
        .chat_stream_incremental(request(&messages, "openai-oauth/mini"))
        .await;
    let sender = inner.senders.lock().unwrap()[0].clone();
    drop(events);
    tokio::time::timeout(Duration::from_secs(2), sender.closed())
        .await
        .expect("the provider's stream is closed once its caller has gone");
}

/// M3: a reply the provider serves for the model releases its refusal.
#[tokio::test]
async fn a_served_reply_releases_the_refusal() {
    let inner = Scripted::new(vec![Err(REFUSAL), Ok("served")]);
    let sink = counting();
    let provider = RefusalRecordingProvider::new(inner, sink.clone(), HELD);
    let messages = [Message::user("hi")];
    let _ = provider.chat(request(&messages, "openai-oauth/mini")).await;
    assert_eq!(refused(&sink, "openai-oauth/mini").as_deref(), Some(DETAIL));
    provider
        .chat(request(&messages, "openai-oauth/mini"))
        .await
        .expect("served");
    assert_eq!(refused(&sink, "openai-oauth/mini"), None, "released");
    assert_eq!(
        sink.cleared.lock().unwrap().as_slice(),
        ["openai-oauth/mini"]
    );
}

/// M3: a stream that completes releases the refusal too.
#[tokio::test]
async fn a_completed_stream_releases_the_refusal() {
    let inner = Scripted::new(vec![]);
    let sink = counting();
    sink.store.record_refusal(
        &ModelRef::parse_qualified("openai-oauth/mini").unwrap(),
        DETAIL,
        HELD,
    );
    let provider = RefusalRecordingProvider::new(inner.clone(), sink.clone(), HELD);
    let messages = [Message::user("hi")];
    let mut events = provider
        .chat_stream_incremental(request(&messages, "openai-oauth/mini"))
        .await;
    let sender = inner.senders.lock().unwrap()[0].clone();
    sender
        .send(StreamEvent::Done(LlmResponse {
            content: Some("served".into()),
            tool_calls: vec![],
            usage: None,
            stop_reason: None,
            thinking_blocks: vec![],
        }))
        .await
        .unwrap();
    drop(sender);
    inner.senders.lock().unwrap().clear();
    assert!(matches!(events.recv().await, Some(StreamEvent::Done(_))));
    assert!(events.recv().await.is_none());
    assert_eq!(refused(&sink, "openai-oauth/mini"), None, "released");
}

/// M3 and review round 2: a zero hold (`model_refusal_ttl_secs = 0`)
/// records nothing at all; the reply passes through unchanged.
#[tokio::test]
async fn a_zero_hold_records_nothing() {
    let inner = Scripted::new(vec![Err(REFUSAL)]);
    let sink = counting();
    let provider = RefusalRecordingProvider::new(inner, sink.clone(), Duration::ZERO);
    let messages = [Message::user("hi")];
    let error = provider
        .chat(request(&messages, "openai-oauth/mini"))
        .await
        .unwrap_err();
    assert!(matches!(&error, DomainError::Provider(m) if m == REFUSAL));
    assert!(sink.calls.lock().unwrap().is_empty(), "not recorded");
    assert_eq!(refused(&sink, "openai-oauth/mini"), None);
}
