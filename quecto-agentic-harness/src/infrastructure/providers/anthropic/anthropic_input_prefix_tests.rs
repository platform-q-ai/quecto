//! #2410 (extending #2398): each Anthropic request records, on its trace,
//! where the input it sends first differs from its session's last accepted
//! request, on every path a reply is read on.
use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;
use crate::domain::inference::events::request_observation::{
    InputBaseline, InputItemKind, InputPrefixParts, RequestTrace,
};
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const SSE: &str = "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{\"input_tokens\":3}}}\n\n\
event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"ok\"}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

/// A server that answers every request, except one whose body contains
/// `refused` (when given), which it refuses with a 400.
async fn server(refused: Option<&str>) -> MockServer {
    let server = MockServer::start().await;
    if let Some(needle) = refused {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(ResponseTemplate::new(400).set_body_string("{\"error\":{}}"))
            .with_priority(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .and(body_string_contains("\"stream\":true"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(SSE, "text/event-stream"))
        .with_priority(2)
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_json(serde_json::json!({
            "id": "m", "type": "message", "role": "assistant",
            "content": [{"type": "text", "text": "ok"}],
            "stop_reason": "end_turn",
            "usage": {"input_tokens": 3, "output_tokens": 1},
        })))
        .with_priority(3)
        .mount(&server)
        .await;
    server
}

/// How a request was sent.
#[derive(Clone, Copy, Debug)]
enum Path {
    Whole,
    Assembled,
    Streamed,
}
const PATHS: [Path; 3] = [Path::Whole, Path::Assembled, Path::Streamed];

/// Send `messages` in session `s` on a trace carrying `baseline`; what it
/// recorded.
async fn send(
    provider: &AnthropicProvider,
    path: Path,
    messages: &[Message],
    baseline: &InputBaseline,
) -> Option<InputPrefixParts> {
    let trace = Arc::new(RequestTrace::default());
    trace.attach_input_baseline(baseline.clone());
    let request = ChatRequest {
        trace: Some(trace.clone()),
        admission: None,
        messages,
        tools: &[],
        model: "claude-sonnet-4-5",
        max_tokens: 100,
        temperature: 0.0,
        session_id: Some("s"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    match path {
        Path::Whole => drop(provider.chat(request).await),
        Path::Assembled => drop(provider.chat_stream(request).await),
        Path::Streamed => {
            let mut events = provider.chat_stream_incremental(request).await;
            while events.recv().await.is_some() {}
        }
    }
    trace.input_prefix().map(|prefix| prefix.parts())
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }
}

/// A tool loop sends append-only requests: the cache markers that move
/// forward each request (#2410) are no edit.
#[tokio::test]
async fn a_tool_loop_records_append_only_requests_on_every_path() {
    for path in PATHS {
        let server = server(None).await;
        let provider = AnthropicProvider::new("sk-ant-test".into(), Some(server.uri()));
        let baseline = InputBaseline::default();
        let mut messages = vec![Message::system("sys"), Message::user("list the files")];
        let first = send(&provider, path, &messages, &baseline).await;
        let first = first.expect("observed");
        assert_eq!(
            (first.input_items, first.previous_items),
            (1, None),
            "{path:?}"
        );
        messages.push(Message::assistant("", vec![call("c1")]));
        messages.push(Message::tool("c1", "a.rs b.rs"));
        let second = send(&provider, path, &messages, &baseline).await;
        let second = second.expect("observed");
        assert_eq!(second.input_items, 3, "{path:?}");
        assert_eq!(second.previous_items, Some(1), "{path:?}");
        assert_eq!(second.first_changed_item, None, "{path:?}: {second:?}");
        assert!(second.prefix_tokens_estimate > 0, "{path:?}");
    }
}

/// A pruned tool result records where the inputs diverge, and its kind.
#[tokio::test]
async fn a_pruned_tool_result_records_its_index_and_kind() {
    let server = server(None).await;
    let provider = AnthropicProvider::new("sk-ant-test".into(), Some(server.uri()));
    let baseline = InputBaseline::default();
    let mut messages = vec![
        Message::system("sys"),
        Message::user("go"),
        Message::assistant("", vec![call("c1")]),
        Message::tool("c1", "a very long tool output"),
    ];
    send(&provider, Path::Whole, &messages, &baseline).await;
    messages[3] = Message::tool("c1", "[pruned]");
    messages.push(Message::user("next"));
    let observed = send(&provider, Path::Whole, &messages, &baseline)
        .await
        .expect("observed");
    assert_eq!(observed.first_changed_item, Some(2));
    assert_eq!(
        observed.first_changed_kind,
        Some(InputItemKind::FunctionCallOutput)
    );
}

/// Only an accepted send becomes the session's baseline: the request after
/// a refused one is compared with the last accepted request.
#[tokio::test]
async fn a_refused_send_never_becomes_the_baseline() {
    for path in PATHS {
        let server = server(Some("refuse me")).await;
        let provider = AnthropicProvider::new("sk-ant-test".into(), Some(server.uri()));
        let baseline = InputBaseline::default();
        let accepted = vec![Message::user("go")];
        send(&provider, path, &accepted, &baseline).await;
        let refused = vec![Message::user("refuse me")];
        send(&provider, path, &refused, &baseline).await;
        let mut next = accepted.clone();
        next.push(Message::assistant("ok", vec![]));
        next.push(Message::user("more"));
        let observed = send(&provider, path, &next, &baseline)
            .await
            .expect("observed");
        assert_eq!(observed.previous_items, Some(1), "{path:?}");
        assert_eq!(observed.first_changed_item, None, "{path:?}");
    }
}

/// A gate that never admits, and says when it was asked and when the pending
/// acquire was dropped.
#[derive(Debug, Default)]
struct NeverAdmits {
    asked: tokio::sync::Notify,
    withdrawn: Arc<std::sync::atomic::AtomicBool>,
}

/// Sets its flag when dropped: a pending acquire withdrawn.
struct OnDrop(Arc<std::sync::atomic::AtomicBool>);

impl Drop for OnDrop {
    fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl crate::application::admission::ports::inference_attempt::AttemptAdmission for NeverAdmits {
    fn acquire(
        &self,
    ) -> crate::application::admission::ports::inference_attempt::AttemptAcquisition<'_> {
        self.asked.notify_one();
        let guard = OnDrop(self.withdrawn.clone());
        Box::pin(async move {
            let _guard = guard;
            std::future::pending().await
        })
    }
}

/// #2436, #2628 review: a turn dropped while its streamed attempt waits for
/// admission withdraws that attempt, also when the request is measured for
/// the cache diagnostic, so no request goes out with no reader.
#[tokio::test]
async fn a_dropped_stream_withdraws_its_queued_attempt() {
    let gate = Arc::new(NeverAdmits::default());
    let client = crate::infrastructure::providers::transport::single_attempt_client::SingleAttemptClient::build(
        reqwest::Client::builder().no_proxy(),
    )
    .expect("a client");
    let provider = AnthropicProvider::new("sk-ant-test".into(), Some("http://127.0.0.1:1".into()))
        .with_attempt_admission(gate.clone(), client);
    let baseline = InputBaseline::default();
    let trace = Arc::new(RequestTrace::default());
    trace.attach_input_baseline(baseline);
    let messages = [Message::user("go")];
    let request = ChatRequest {
        trace: Some(trace),
        admission: None,
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-5",
        max_tokens: 100,
        temperature: 0.0,
        session_id: Some("s"),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let events = provider.chat_stream_incremental(request).await;
    tokio::time::timeout(std::time::Duration::from_secs(5), gate.asked.notified())
        .await
        .expect("the attempt asked for admission");
    drop(events);
    let withdrawn = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        while !gate.withdrawn.load(std::sync::atomic::Ordering::SeqCst) {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await;
    assert!(withdrawn.is_ok(), "the queued attempt was withdrawn");
}
