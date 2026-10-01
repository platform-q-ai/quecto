//! #2398: each request records, on its trace, where the input it sends
//! first differs from its session's last accepted request.
use super::*;
use crate::domain::message::{Message, ToolCall};
use crate::domain::request_observation::{
    InputBaseline, InputItemKind, InputPrefixParts, RequestTrace,
};
use crate::domain::token_estimate::estimate_tokens;
use std::sync::Arc;
use wiremock::matchers::{body_string_contains, method};
use wiremock::{Mock, MockServer, ResponseTemplate};

const MODEL: &str = "gpt-6-sol";
/// Replayed reasoning's encrypted content, which the refusing server
/// recognises.
const BLOB: &str = "gAAA-replayed-blob";

fn completed_sse() -> String {
    let event =
        serde_json::json!({"type": "response.completed", "response": {"status": "completed"}});
    format!("data: {event}\n\n")
}

/// A server that answers every request, except one whose body contains
/// `refused` (when given), which it answers with `status` and `message`.
async fn server(refused: Option<(&str, u16, &str)>) -> MockServer {
    let server = MockServer::start().await;
    if let Some((needle, status, message)) = refused {
        Mock::given(method("POST"))
            .and(body_string_contains(needle))
            .respond_with(ResponseTemplate::new(status).set_body_string(message))
            .with_priority(1)
            .mount(&server)
            .await;
    }
    Mock::given(method("POST"))
        .respond_with(ResponseTemplate::new(200).set_body_raw(completed_sse(), "text/event-stream"))
        .mount(&server)
        .await;
    server
}

/// A provider at `server`.
fn provider_at(server: &MockServer) -> CodexProvider {
    CodexProvider::with_client(
        "sk-test".into(),
        "acct".into(),
        Some(server.uri()),
        reqwest::Client::new(),
    )
}

/// A trace carrying the session's `baseline`.
fn traced(baseline: &InputBaseline) -> Arc<RequestTrace> {
    let trace = Arc::new(RequestTrace::default());
    trace.attach_input_baseline(baseline.clone());
    trace
}

fn request<'a>(
    messages: &'a [Message],
    session: Option<&'a str>,
    trace: Option<Arc<RequestTrace>>,
) -> ChatRequest<'a> {
    ChatRequest {
        trace,
        admission: None,
        messages,
        tools: &[],
        model: MODEL,
        max_tokens: 100,
        temperature: 0.0,
        session_id: session,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

/// How a request was sent.
#[derive(Clone, Copy, Debug)]
enum Path {
    Assembled,
    Streamed,
}

/// Send `messages` in `session` on `trace`; whether it succeeded.
async fn send_on(
    provider: &CodexProvider,
    path: Path,
    messages: &[Message],
    session: Option<&str>,
    trace: Option<Arc<RequestTrace>>,
) -> bool {
    let request = request(messages, session, trace);
    match path {
        Path::Assembled => provider.chat(request).await.is_ok(),
        Path::Streamed => {
            let mut events = provider.chat_stream_incremental(request).await;
            let mut done = false;
            while let Some(event) = events.recv().await {
                done |= matches!(event, StreamEvent::Done(_));
            }
            done
        }
    }
}

/// What a sent request of `messages` in session `s`, whose baseline is
/// `baseline`, records.
async fn send(
    provider: &CodexProvider,
    path: Path,
    messages: &[Message],
    baseline: &InputBaseline,
) -> Option<InputPrefixParts> {
    let trace = traced(baseline);
    send_on(provider, path, messages, Some("s"), Some(trace.clone())).await;
    trace.input_prefix().map(|prefix| prefix.parts())
}

/// The body `messages` are sent as, replaying no reasoning.
fn plain_body(messages: &[Message]) -> serde_json::Value {
    let auth = ResponsesAuth::ChatGptOAuth {
        account_id: "acct".into(),
    };
    CodexProvider::build_request_body(&request(messages, None, None), &auth, "")
}

/// The estimated tokens of the first `count` input items `messages` send
/// (none of them reasoning).
fn item_tokens(messages: &[Message], count: usize) -> usize {
    plain_body(messages)["input"].as_array().unwrap()[..count]
        .iter()
        .map(|item| estimate_tokens(&item.to_string()))
        .sum()
}

/// The estimated tokens of the instructions `messages` send (they send no
/// tools).
fn head_tokens(messages: &[Message]) -> usize {
    let body = plain_body(messages);
    assert!(body.get("tools").is_none());
    estimate_tokens(&body["instructions"].to_string())
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }
}

/// A tool turn whose call carries encrypted reasoning for `provider`.
fn reasoning_turn(provider: &CodexProvider) -> Vec<Message> {
    let mut assistant = Message::assistant("", vec![call("c1")]);
    assistant.thinking_blocks = vec![ThinkingBlock::EncryptedReasoning {
        origin: provider.reasoning_origin(MODEL),
        leads_to: Some("c1".into()),
        item: serde_json::json!({"type": "reasoning", "summary": [], "encrypted_content": BLOB})
            .to_string(),
    }];
    vec![
        Message::system("sys"),
        Message::user("go"),
        assistant,
        Message::tool("c1", "done"),
    ]
}

#[tokio::test]
async fn a_tool_loop_records_append_only_requests() {
    for path in [Path::Assembled, Path::Streamed] {
        let server = server(None).await;
        let provider = provider_at(&server);
        let baseline = InputBaseline::default();
        let mut messages = vec![Message::system("sys"), Message::user("list the files")];
        let first = send(&provider, path, &messages, &baseline)
            .await
            .expect("observed");
        assert_eq!(
            first,
            InputPrefixParts {
                input_items: 1,
                previous_items: None,
                first_changed_item: None,
                first_changed_kind: None,
                prefix_tokens_estimate: 0,
                unchanged_prefix_tokens_estimate: 0,
                request_tokens_estimate: head_tokens(&messages) + item_tokens(&messages, 1),
            },
            "{path:?}"
        );
        let previous = messages.clone();
        messages.push(Message::assistant("", vec![call("c1")]));
        messages.push(Message::tool("c1", "a.rs b.rs"));
        let second = send(&provider, path, &messages, &baseline)
            .await
            .expect("observed");
        let prefix = item_tokens(&previous, 1);
        assert_eq!(
            second,
            InputPrefixParts {
                input_items: 3,
                previous_items: Some(1),
                first_changed_item: None,
                first_changed_kind: None,
                prefix_tokens_estimate: prefix,
                unchanged_prefix_tokens_estimate: head_tokens(&messages) + prefix,
                request_tokens_estimate: head_tokens(&messages) + item_tokens(&messages, 3),
            },
            "{path:?}"
        );
    }
}

#[tokio::test]
async fn an_edited_tool_output_records_its_index_and_kind() {
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let mut messages = vec![
        Message::system("sys"),
        Message::user("go"),
        Message::assistant("", vec![call("c1")]),
        Message::tool("c1", "a very long tool output"),
    ];
    send(&provider, Path::Assembled, &messages, &baseline).await;
    messages[3] = Message::tool("c1", "[pruned]");
    messages.push(Message::user("next"));
    let observed = send(&provider, Path::Assembled, &messages, &baseline)
        .await
        .expect("observed");
    assert_eq!(observed.input_items, 4);
    assert_eq!(observed.previous_items, Some(3));
    assert_eq!(observed.first_changed_item, Some(2));
    assert_eq!(
        observed.first_changed_kind,
        Some(InputItemKind::FunctionCallOutput)
    );
    assert_eq!(observed.prefix_tokens_estimate, item_tokens(&messages, 2));
}

#[tokio::test]
async fn a_removed_message_records_where_the_inputs_diverge() {
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let before = [
        Message::system("sys"),
        Message::user("1"),
        Message::user("2"),
        Message::user("3"),
    ];
    send(&provider, Path::Assembled, &before, &baseline).await;
    let after = [
        Message::system("sys"),
        Message::user("1"),
        Message::user("3"),
    ];
    let observed = send(&provider, Path::Assembled, &after, &baseline)
        .await
        .expect("observed");
    assert_eq!(observed.first_changed_item, Some(1));
    assert_eq!(observed.first_changed_kind, Some(InputItemKind::User));
    assert_eq!(observed.prefix_tokens_estimate, item_tokens(&after, 1));
}

/// A retry sends the same input again under the same trace: the record of
/// its first send stays.
#[tokio::test]
async fn a_resent_request_keeps_the_record_of_its_first_send() {
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let base = [Message::system("sys"), Message::user("a")];
    send(&provider, Path::Assembled, &base, &baseline).await;
    let messages = [Message::system("sys"), Message::user("b")];
    let trace = traced(&baseline);
    for _ in 0..2 {
        send_on(
            &provider,
            Path::Assembled,
            &messages,
            Some("s"),
            Some(trace.clone()),
        )
        .await;
    }
    let observed = trace.input_prefix().expect("observed").parts();
    assert_eq!(observed.first_changed_item, Some(0));
    assert_eq!(observed.first_changed_kind, Some(InputItemKind::User));
}

/// The input compared is the one the first body sends, replayed reasoning
/// and all.
#[tokio::test]
async fn the_replaying_body_is_the_one_compared() {
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let mut messages = reasoning_turn(&provider);
    send(&provider, Path::Assembled, &messages, &baseline).await;
    messages.push(Message::user("next"));
    let observed = send(&provider, Path::Assembled, &messages, &baseline)
        .await
        .expect("observed");
    assert_eq!(
        observed.input_items, 5,
        "user, reasoning, call, output, user"
    );
    assert_eq!(observed.previous_items, Some(4));
    assert_eq!(observed.first_changed_item, None);
}

/// A refused replay is resent without it, and replay stays off: the plain
/// body accepted is the baseline, so the next request is append-only.
#[tokio::test]
async fn a_refused_replay_leaves_the_accepted_plain_body_as_the_baseline() {
    for path in [Path::Assembled, Path::Streamed] {
        let refusal = "{\"error\":{\"message\":\"The encrypted content could not be verified\"}}";
        let server = server(Some((BLOB, 400, refusal))).await;
        let provider = provider_at(&server);
        let baseline = InputBaseline::default();
        let mut messages = reasoning_turn(&provider);
        let first = send(&provider, path, &messages, &baseline)
            .await
            .expect("observed");
        assert_eq!(
            first.input_items, 4,
            "{path:?}: compared as the replaying body"
        );
        messages.push(Message::user("next"));
        let observed = send(&provider, path, &messages, &baseline)
            .await
            .expect("observed");
        assert_eq!(
            server.received_requests().await.unwrap().len(),
            3,
            "{path:?}"
        );
        assert_eq!(observed.input_items, 4, "{path:?}: replay is off");
        assert_eq!(observed.previous_items, Some(3), "{path:?}");
        assert_eq!(observed.first_changed_item, None, "{path:?}: {observed:?}");
    }
}

/// A send that fails (or is never accepted) does not become the baseline.
#[tokio::test]
async fn a_failed_send_does_not_become_the_baseline() {
    for path in [Path::Assembled, Path::Streamed] {
        let server = server(Some(("EDITED", 500, "overloaded"))).await;
        let provider = provider_at(&server);
        let baseline = InputBaseline::default();
        let base = [Message::system("sys"), Message::user("a")];
        assert!(send_on(&provider, path, &base, Some("s"), Some(traced(&baseline))).await);
        let edited = [Message::system("sys"), Message::user("EDITED")];
        assert!(!send_on(&provider, path, &edited, Some("s"), Some(traced(&baseline))).await);
        let appended = [
            Message::system("sys"),
            Message::user("a"),
            Message::user("b"),
        ];
        let observed = send(&provider, path, &appended, &baseline)
            .await
            .expect("observed");
        assert_eq!(observed.previous_items, Some(1), "{path:?}");
        assert_eq!(observed.first_changed_item, None, "{path:?}");
    }
}

/// Only a request of a named session whose trace carries its baseline is
/// compared.
#[tokio::test]
async fn only_a_request_carrying_its_sessions_baseline_is_compared() {
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let base = [Message::system("sys"), Message::user("a")];
    send(&provider, Path::Assembled, &base, &baseline).await;
    let other = [Message::system("sys"), Message::user("unobserved")];
    for trace in [None, Some(Arc::new(RequestTrace::default()))] {
        assert!(send_on(&provider, Path::Assembled, &other, Some("s"), trace.clone()).await);
        assert_eq!(trace.and_then(|trace| trace.input_prefix()), None);
    }
    let appended = [
        Message::system("sys"),
        Message::user("a"),
        Message::user("b"),
    ];
    let observed = send(&provider, Path::Assembled, &appended, &baseline)
        .await
        .expect("observed");
    assert_eq!(observed.previous_items, Some(1));
    assert_eq!(observed.first_changed_item, None);
    let trace = traced(&baseline);
    send_on(
        &provider,
        Path::Assembled,
        &appended,
        None,
        Some(trace.clone()),
    )
    .await;
    assert_eq!(trace.input_prefix(), None, "no session, nothing compared");
}

/// A provider rebuilt mid-session (an OAuth refresh) compares with what its
/// predecessor sent: the baseline is the session's, not the provider's.
#[tokio::test]
async fn a_provider_rebuilt_mid_session_compares_with_the_previous_request() {
    let server = server(None).await;
    let baseline = InputBaseline::default();
    let before = [
        Message::system("sys"),
        Message::user("a"),
        Message::user("b"),
    ];
    let first = send(&provider_at(&server), Path::Assembled, &before, &baseline)
        .await
        .expect("observed");
    assert_eq!(first.previous_items, None);
    let after = [
        Message::system("sys"),
        Message::user("EDITED"),
        Message::user("b"),
    ];
    let observed = send(&provider_at(&server), Path::Assembled, &after, &baseline)
        .await
        .expect("observed");
    assert_eq!(observed.previous_items, Some(2));
    assert_eq!(observed.first_changed_item, Some(0));
    let fresh = send(
        &provider_at(&server),
        Path::Assembled,
        &after,
        &InputBaseline::default(),
    )
    .await
    .expect("observed");
    assert_eq!(
        fresh.previous_items, None,
        "another session's baseline is its own"
    );
}

/// The record carries no content, and nothing kept carries the session key.
#[tokio::test]
async fn the_record_carries_no_content() {
    const SECRET: &str = "sk-proj-QX7hunter2SECRETtoken9d1f";
    let session = format!("cli:{SECRET}");
    let server = server(None).await;
    let provider = provider_at(&server);
    let baseline = InputBaseline::default();
    let base = [Message::system(SECRET), Message::user(SECRET)];
    send_on(
        &provider,
        Path::Assembled,
        &base,
        Some(&session),
        Some(traced(&baseline)),
    )
    .await;
    let messages = [
        Message::system(SECRET),
        Message::user(format!("{SECRET}?")),
        Message::assistant(SECRET, vec![call("c1")]),
        Message::tool("c1", SECRET),
    ];
    let trace = traced(&baseline);
    send_on(
        &provider,
        Path::Assembled,
        &messages,
        Some(&session),
        Some(trace.clone()),
    )
    .await;
    let observed = trace.input_prefix().expect("observed");
    assert_eq!(observed.parts().first_changed_item, Some(0));
    let text = format!(
        "{observed:?} {} {baseline:?}",
        serde_json::to_string(&observed).unwrap(),
    );
    for fragment in ["sk-proj", "hunter2", "SECRET", "9d1f", "cli:"] {
        assert!(!text.contains(fragment), "{fragment} in {text}");
    }
}

/// A server that streams one text delta, then holds the reply open.
async fn stalling_server() -> (String, tokio::task::JoinHandle<()>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    let task = tokio::spawn(async move {
        let (mut socket, _) = listener.accept().await.unwrap();
        let mut request = [0u8; 65536];
        let _ = socket.read(&mut request).await;
        let event = serde_json::json!({"type": "response.output_text.delta", "delta": "hi"});
        let data = format!("data: {event}\n\n");
        let reply = format!(
            "HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ntransfer-encoding: chunked\r\n\r\n{:x}\r\n{data}\r\n",
            data.len()
        );
        socket.write_all(reply.as_bytes()).await.unwrap();
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
        drop(socket);
    });
    (url, task)
}

/// Review: with no admission gate, the relay returns once its caller drops
/// the receiver, though the reply stalls, so the send it relays stops.
#[tokio::test]
async fn the_relay_returns_once_its_caller_drops_the_receiver() {
    let (url, server) = stalling_server().await;
    let provider = CodexProvider::with_client(
        "sk-test".into(),
        "acct".into(),
        Some(url),
        reqwest::Client::new(),
    );
    let messages = [Message::system("sys"), Message::user("hi")];
    let (call, bodies) = provider.prepare(&request(&messages, Some("s"), None));
    let (tx, mut rx) = tokio::sync::mpsc::channel(64);
    let relay = tokio::spawn(provider.clone().stream(call, bodies, tx));
    let first = tokio::time::timeout(std::time::Duration::from_secs(10), rx.recv())
        .await
        .expect("a first event");
    assert!(
        matches!(first, Some(StreamEvent::TextDelta(_))),
        "{first:?}"
    );
    drop(rx);
    tokio::time::timeout(std::time::Duration::from_secs(5), relay)
        .await
        .expect("the relay returns")
        .unwrap();
    server.abort();
}
