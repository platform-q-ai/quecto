//! #2162: the `session_id` header, and encrypted reasoning kept and
//! replayed, each item before the call it led to, to its own origin.
use super::*;
use crate::domain::conversation::value_objects::message::ToolCall;

const ORIGIN: &str = "https://h/codex/responses|acct|gpt-6-sol";

fn sse(events: &[serde_json::Value]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn reasoning_done(output_index: u64, encrypted: Option<&str>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "type": "reasoning",
        "id": format!("rs_{output_index}"),
        "summary": [{"type": "summary_text", "text": "why"}],
    });
    if let Some(encrypted) = encrypted {
        item["encrypted_content"] = serde_json::json!(encrypted);
    }
    serde_json::json!({"type": "response.output_item.done", "output_index": output_index, "item": item})
}

fn call_added(output_index: u64, call_id: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "response.output_item.added",
        "output_index": output_index,
        "item": {"type": "function_call", "call_id": call_id, "name": "bash"},
    })
}

fn completed() -> serde_json::Value {
    serde_json::json!({"type": "response.completed", "response": {"status": "completed"}})
}

fn encrypted_blocks(response: &LlmResponse) -> Vec<(String, Option<String>, serde_json::Value)> {
    response
        .thinking_blocks
        .iter()
        .filter_map(|block| match block {
            ThinkingBlock::EncryptedReasoning {
                origin,
                leads_to,
                item,
            } => Some((
                origin.clone(),
                leads_to.clone(),
                serde_json::from_str(item).unwrap(),
            )),
            ThinkingBlock::Normal { .. } | ThinkingBlock::Redacted { .. } => None,
        })
        .collect()
}

#[test]
fn a_reasoning_item_with_encrypted_content_is_kept_without_its_id() {
    let raw = sse(&[reasoning_done(0, Some("gAAA")), completed()]);
    let response = CodexProvider::parse_sse_response(&raw).unwrap();
    assert_eq!(
        encrypted_blocks(&response),
        vec![(
            String::new(),
            None,
            serde_json::json!({
                "type": "reasoning",
                "summary": [{"type": "summary_text", "text": "why"}],
                "encrypted_content": "gAAA",
            })
        )],
        "unstamped (the provider stamps), led to no call, no id"
    );
}

#[test]
fn a_reasoning_item_without_encrypted_content_keeps_only_its_summary() {
    for encrypted in [None, Some("")] {
        let raw = sse(&[reasoning_done(0, encrypted), completed()]);
        let response = CodexProvider::parse_sse_response(&raw).unwrap();
        assert!(encrypted_blocks(&response).is_empty());
    }
}

/// Review: each item leads to the first call after it in the output.
#[test]
fn each_item_leads_to_the_call_that_followed_it() {
    let raw = sse(&[
        reasoning_done(0, Some("r0")),
        call_added(1, "call_a"),
        reasoning_done(2, Some("r2")),
        call_added(3, "call_b"),
        reasoning_done(4, Some("r4")),
        completed(),
    ]);
    let response = CodexProvider::parse_sse_response(&raw).unwrap();
    let led: Vec<(Option<String>, String)> = encrypted_blocks(&response)
        .into_iter()
        .map(|(_, leads_to, item)| {
            (
                leads_to,
                item["encrypted_content"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert_eq!(
        led,
        vec![
            (Some("call_a".into()), "r0".into()),
            (Some("call_b".into()), "r2".into()),
            (None, "r4".into()),
        ]
    );
}

/// Review: encrypted reasoning past the per-response cap is not kept.
#[test]
fn encrypted_reasoning_past_the_cap_is_dropped() {
    let big = "x".repeat(codex_sse_state::MAX_ENCRYPTED_REASONING_BYTES / 2 + 1);
    let raw = sse(&[
        reasoning_done(0, Some(&big)),
        reasoning_done(1, Some(&big)),
        reasoning_done(2, Some("small")),
        completed(),
    ]);
    let response = CodexProvider::parse_sse_response(&raw).unwrap();
    let kept: Vec<usize> = encrypted_blocks(&response)
        .iter()
        .map(|(_, _, item)| item["encrypted_content"].as_str().unwrap().len())
        .collect();
    assert_eq!(kept, vec![big.len(), "small".len()]);
}

fn block(origin: &str, leads_to: Option<&str>, content: &str) -> ThinkingBlock {
    ThinkingBlock::EncryptedReasoning {
        origin: origin.into(),
        leads_to: leads_to.map(str::to_string),
        item: serde_json::json!({"type": "reasoning", "summary": [], "encrypted_content": content})
            .to_string(),
    }
}

fn call(id: &str) -> ToolCall {
    ToolCall {
        id: id.into(),
        name: "bash".into(),
        arguments: "{}".into(),
    }
}

/// A turn of `calls` (each answered) whose assistant message carries `blocks`.
fn turn(calls: &[&str], text: &str, blocks: Vec<ThinkingBlock>) -> Vec<Message> {
    let mut assistant = Message::assistant(text, calls.iter().map(|id| call(id)).collect());
    assistant.thinking_blocks = blocks;
    let mut messages = vec![Message::system("sys"), Message::user("go"), assistant];
    messages.extend(calls.iter().map(|id| Message::tool(*id, "done")));
    messages
}

/// Each input item as `type:detail` (reasoning content, call id, role).
fn shape(input: &[serde_json::Value]) -> Vec<String> {
    input
        .iter()
        .map(|item| match item["type"].as_str() {
            Some("reasoning") => {
                format!("reasoning:{}", item["encrypted_content"].as_str().unwrap())
            }
            Some(kind) => format!("{kind}:{}", item["call_id"].as_str().unwrap_or_default()),
            None => item["role"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

#[test]
fn reasoning_is_replayed_to_its_origin_just_before_the_call_it_led_to() {
    let messages = turn(
        &["call_a", "call_b"],
        "",
        vec![
            block(ORIGIN, Some("call_a"), "r0"),
            block(ORIGIN, Some("call_b"), "r2"),
        ],
    );
    let (_, input) = CodexProvider::build_input_for(&messages, ORIGIN);
    assert_eq!(
        shape(&input),
        [
            "user",
            "reasoning:r0",
            "function_call:call_a",
            "reasoning:r2",
            "function_call:call_b",
            "function_call_output:call_a",
            "function_call_output:call_b",
        ]
    );
}

#[test]
fn reasoning_that_led_to_text_goes_before_the_text() {
    let messages = turn(&[], "answer", vec![block(ORIGIN, None, "r0")]);
    let (_, input) = CodexProvider::build_input_for(&messages, ORIGIN);
    assert_eq!(shape(&input), ["user", "reasoning:r0", "assistant"]);
}

/// Review: another endpoint, account or model — or none — replays nothing.
#[test]
fn reasoning_is_not_replayed_to_another_origin() {
    let other_account = "https://h/codex/responses|other|gpt-6-sol";
    let other_endpoint = "https://api/responses|api-key|gpt-6-sol";
    for (stamped, requested) in [
        (ORIGIN, other_account),
        (ORIGIN, other_endpoint),
        ("", ORIGIN),
        (ORIGIN, ""),
    ] {
        let messages = turn(&["call_a"], "", vec![block(stamped, Some("call_a"), "r0")]);
        let (_, input) = CodexProvider::build_input_for(&messages, requested);
        assert_eq!(
            shape(&input),
            [
                "user",
                "function_call:call_a",
                "function_call_output:call_a"
            ],
            "stamped {stamped:?}, requested {requested:?}"
        );
    }
}

#[test]
fn reasoning_is_not_replayed_without_the_item_it_led_to() {
    // The call it led to went unanswered (orphaned), and no text follows.
    let mut messages = turn(&["call_a"], "", vec![block(ORIGIN, Some("call_a"), "r0")]);
    messages.pop();
    let (_, input) = CodexProvider::build_input_for(&messages, ORIGIN);
    assert_eq!(shape(&input), ["user"]);
    // Reasoning that led to text is not sent with a tool turn's calls.
    let messages = turn(&["call_a"], "", vec![block(ORIGIN, None, "r0")]);
    let (_, input) = CodexProvider::build_input_for(&messages, ORIGIN);
    assert_eq!(
        shape(&input),
        [
            "user",
            "function_call:call_a",
            "function_call_output:call_a"
        ]
    );
}

#[test]
fn a_stored_item_that_is_not_an_object_is_skipped() {
    let mut messages = turn(&["call_a"], "", Vec::new());
    messages[2].thinking_blocks = vec![ThinkingBlock::EncryptedReasoning {
        origin: ORIGIN.into(),
        leads_to: Some("call_a".into()),
        item: "not json".into(),
    }];
    let (_, input) = CodexProvider::build_input_for(&messages, ORIGIN);
    assert_eq!(
        shape(&input),
        [
            "user",
            "function_call:call_a",
            "function_call_output:call_a"
        ]
    );
}

/// Review: a key's prefix reaches a header, so only a plain one is kept.
#[test]
fn a_cache_key_prefix_is_kept_only_when_plain() {
    assert!(CodexProvider::sanitize_cache_key("cli:default").starts_with("cli:"));
    assert!(CodexProvider::sanitize_cache_key("chat-1_x:a").starts_with("chat-1_x:"));
    for odd in ["caf\u{e9}:a", "a b:c", "tab\t:x", ":empty", &"p".repeat(65)] {
        let key = CodexProvider::sanitize_cache_key(odd);
        assert!(key.starts_with("session:"), "{odd:?} -> {key}");
        assert!(reqwest::header::HeaderValue::from_str(&key).is_ok());
    }
}

fn chat_request<'a>(messages: &'a [Message], session: Option<&'a str>) -> ChatRequest<'a> {
    ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model: "gpt-6-sol",
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

async fn server_answering(events: &[serde_json::Value]) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200).set_body_raw(sse(events), "text/event-stream"),
        )
        .mount(&server)
        .await;
    server
}

fn oauth_at(server: &wiremock::MockServer) -> CodexProvider {
    CodexProvider::with_client(
        "sk-test".into(),
        "acct".into(),
        Some(server.uri()),
        reqwest::Client::new(),
    )
}

async fn served(session: Option<&'static str>, oauth: bool) -> Vec<Option<String>> {
    let server = server_answering(&[completed()]).await;
    let provider = match oauth {
        true => oauth_at(&server),
        false => CodexProvider::with_api_key(
            "sk-test".into(),
            Some(server.uri()),
            reqwest::Client::new(),
        ),
    };
    let messages = vec![Message::system("sys"), Message::user("hi")];
    provider
        .chat(chat_request(&messages, session))
        .await
        .unwrap();
    server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            request
                .headers
                .get("session_id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
        .collect()
}

#[tokio::test]
async fn an_oauth_request_names_its_session_as_its_cache_key_does() {
    assert_eq!(
        served(Some("cli:default"), true).await,
        vec![Some(CodexProvider::sanitize_cache_key("cli:default"))]
    );
}

#[tokio::test]
async fn no_session_header_without_a_session_or_off_oauth() {
    assert_eq!(served(None, true).await, vec![None]);
    assert_eq!(served(Some("cli:default"), false).await, vec![None]);
}

fn stamped_origins(response: &LlmResponse) -> Vec<String> {
    encrypted_blocks(response)
        .into_iter()
        .map(|(origin, _, _)| origin)
        .collect()
}

/// Review: the assembled path stamps the origin the request went to.
#[tokio::test]
async fn the_assembled_path_stamps_its_origin() {
    let server = server_answering(&[reasoning_done(0, Some("gAAA")), completed()]).await;
    let provider = oauth_at(&server);
    let origin = provider.reasoning_origin("gpt-6-sol");
    assert!(
        origin.contains(&server.uri()) && origin.ends_with("|gpt-6-sol"),
        "{origin}"
    );
    assert!(
        !origin.contains("|acct|"),
        "the account is kept by digest: {origin}"
    );
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let response = provider.chat(chat_request(&messages, None)).await.unwrap();
    assert_eq!(stamped_origins(&response), vec![origin]);
}

/// Review: the incremental stream stamps it too.
#[tokio::test]
async fn the_incremental_stream_stamps_its_origin() {
    let server = server_answering(&[reasoning_done(0, Some("gAAA")), completed()]).await;
    let provider = oauth_at(&server);
    let origin = provider.reasoning_origin("gpt-6-sol");
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let mut events = provider
        .chat_stream_incremental(chat_request(&messages, None))
        .await;
    let mut done = None;
    while let Some(event) = events.recv().await {
        if let StreamEvent::Done(response) = event {
            done = Some(response);
        }
    }
    assert_eq!(stamped_origins(&done.expect("a response")), vec![origin]);
}

/// Review: a response's reasoning, kept on its assistant message, is sent
/// back in the next request to the same provider.
#[tokio::test]
async fn a_responses_reasoning_rides_the_next_request() {
    let server = server_answering(&[
        reasoning_done(0, Some("gAAA")),
        call_added(1, "call_a"),
        completed(),
    ])
    .await;
    let provider = oauth_at(&server);
    let mut messages = vec![Message::system("sys"), Message::user("hi")];
    let response = provider.chat(chat_request(&messages, None)).await.unwrap();
    let mut assistant = Message::assistant("", response.tool_calls.clone());
    assistant.thinking_blocks = response.thinking_blocks.clone();
    messages.push(assistant);
    messages.push(Message::tool("call_a", "done"));
    provider.chat(chat_request(&messages, None)).await.unwrap();
    let requests = server.received_requests().await.unwrap();
    let body: serde_json::Value = serde_json::from_slice(&requests[1].body).unwrap();
    let input = body["input"].as_array().unwrap();
    assert_eq!(
        shape(input),
        [
            "user",
            "reasoning:gAAA",
            "function_call:call_a",
            "function_call_output:call_a"
        ]
    );
}

/// A gate that grants every attempt at once.
#[derive(Debug)]
struct Grant;
#[derive(Debug)]
struct Granted;
impl crate::application::ports::AttemptPermit for Granted {
    fn feedback(
        &mut self,
        _: crate::domain::admission::value_objects::inference_admission::ThrottleFeedback,
    ) {
    }
    fn finish(
        self: Box<Self>,
        _: crate::domain::admission::value_objects::inference_admission::Feedback,
    ) {
    }
}
impl crate::application::ports::AttemptAdmission for Grant {
    fn acquire(&self) -> crate::application::ports::AttemptAcquisition<'_> {
        Box::pin(async {
            Ok(Box::new(Granted) as Box<dyn crate::application::ports::AttemptPermit>)
        })
    }
}

fn gated_at(server: &wiremock::MockServer) -> CodexProvider {
    oauth_at(server).with_attempt_admission(
        std::sync::Arc::new(Grant),
        crate::infrastructure::providers::SingleAttemptClient::build(
            reqwest::Client::builder().no_proxy(),
        )
        .unwrap(),
    )
}

/// Review: the admission-gated assembled and streamed paths stamp too.
#[tokio::test]
async fn the_admission_paths_stamp_their_origin() {
    let server = server_answering(&[reasoning_done(0, Some("gAAA")), completed()]).await;
    let provider = gated_at(&server);
    let origin = provider.reasoning_origin("gpt-6-sol");
    let messages = vec![Message::system("sys"), Message::user("hi")];
    let response = provider.chat(chat_request(&messages, None)).await.unwrap();
    assert_eq!(stamped_origins(&response), vec![origin.clone()]);
    let mut events = provider
        .chat_stream_incremental(chat_request(&messages, None))
        .await;
    let mut done = None;
    while let Some(event) = events.recv().await {
        if let StreamEvent::Done(response) = event {
            done = Some(response);
        }
    }
    assert_eq!(stamped_origins(&done.expect("a response")), vec![origin]);
}

/// A server that refuses any request replaying encrypted reasoning with
/// `refusal`, and answers every other with a completed response.
async fn server_refusing_replay(refusal: &str) -> wiremock::MockServer {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .and(wiremock::matchers::body_string_contains(
            r#""type":"reasoning""#,
        ))
        .respond_with(wiremock::ResponseTemplate::new(400).set_body_string(refusal.to_string()))
        .with_priority(1)
        .mount(&server)
        .await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_raw(sse(&[completed()]), "text/event-stream"),
        )
        .with_priority(2)
        .mount(&server)
        .await;
    server
}

/// A conversation whose last tool turn carries reasoning for `origin`.
fn replaying(origin: &str) -> Vec<Message> {
    turn(&["call_a"], "", vec![block(origin, Some("call_a"), "gAAA")])
}

/// Whether a request's input replays a reasoning item (every body names
/// `encrypted_content` in its `include`).
fn replayed(request: &wiremock::Request) -> bool {
    String::from_utf8_lossy(&request.body).contains(r#""type":"reasoning""#)
}

const REFUSAL: &str = r#"{"error":{"message":"The encrypted content could not be verified.","code":"invalid_encrypted_content"}}"#;

/// Review 2: a refused replay is resent once without it, and the provider
/// replays no more.
#[tokio::test]
async fn a_refused_replay_is_resent_without_it_and_not_replayed_again() {
    let server = server_refusing_replay(REFUSAL).await;
    let provider = oauth_at(&server);
    let messages = replaying(&provider.reasoning_origin("gpt-6-sol"));
    provider.chat(chat_request(&messages, None)).await.unwrap();
    provider.chat(chat_request(&messages, None)).await.unwrap();
    let sent: Vec<bool> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(replayed)
        .collect();
    assert_eq!(
        sent,
        [true, false, false],
        "refused, resent, then no replay"
    );
}

/// Review 2: the same on the streamed path; the refusal never reaches the
/// caller.
#[tokio::test]
async fn a_refused_streamed_replay_is_resent_without_it() {
    let server = server_refusing_replay(REFUSAL).await;
    let provider = oauth_at(&server);
    let messages = replaying(&provider.reasoning_origin("gpt-6-sol"));
    let mut events = provider
        .chat_stream_incremental(chat_request(&messages, None))
        .await;
    let mut seen = Vec::new();
    while let Some(event) = events.recv().await {
        seen.push(match event {
            StreamEvent::Done(_) => "done".to_string(),
            StreamEvent::Error(message) => format!("error: {message}"),
            _ => "other".to_string(),
        });
    }
    assert_eq!(seen.last().map(String::as_str), Some("done"), "{seen:?}");
    assert!(
        seen.iter().all(|event| !event.starts_with("error")),
        "{seen:?}"
    );
    let sent: Vec<bool> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(replayed)
        .collect();
    assert_eq!(sent, [true, false]);
}

/// Another 400 is not a refused replay: it is reported, not retried.
#[tokio::test]
async fn another_bad_request_is_not_retried() {
    let server = server_refusing_replay(r#"{"error":{"message":"bad tools"}}"#).await;
    let provider = oauth_at(&server);
    let messages = replaying(&provider.reasoning_origin("gpt-6-sol"));
    assert!(provider.chat(chat_request(&messages, None)).await.is_err());
    assert_eq!(server.received_requests().await.unwrap().len(), 1);
}

/// Review 2: no origin on either side replays nothing.
#[test]
fn nothing_is_replayed_without_an_origin() {
    let messages = turn(&["call_a"], "", vec![block("", Some("call_a"), "r0")]);
    let (_, input) = CodexProvider::build_input_for(&messages, "");
    assert_eq!(
        shape(&input),
        [
            "user",
            "function_call:call_a",
            "function_call_output:call_a"
        ]
    );
}

/// Swarm review: nothing derived from an API key reaches an origin (it is
/// saved with the session): each provider has its own random identity, so
/// two keys never share an origin and no key can be checked against one.
#[test]
fn an_api_key_origin_holds_nothing_derived_from_the_key() {
    let built = |key: &str| {
        CodexProvider::with_api_key(key.into(), Some("https://h".into()), reqwest::Client::new())
    };
    let one = built("sk-one");
    let origin = one.reasoning_origin("m");
    assert!(!origin.contains("sk-one"), "{origin}");
    assert!(
        !origin.contains(&format!("{:08x}", fnv1a("sk-one"))),
        "{origin}"
    );
    assert_eq!(
        origin,
        one.clone().reasoning_origin("m"),
        "stable for the provider"
    );
    assert_ne!(
        origin,
        built("sk-one").reasoning_origin("m"),
        "not the key's"
    );
    assert_ne!(origin, built("sk-two").reasoning_origin("m"));
}

/// Review 2: the streamed and gated paths send the header too.
#[tokio::test]
async fn every_path_names_its_session() {
    let server = server_answering(&[completed()]).await;
    let expected = Some(CodexProvider::sanitize_cache_key("cli:default"));
    let messages = vec![Message::system("sys"), Message::user("hi")];
    for provider in [oauth_at(&server), gated_at(&server)] {
        provider
            .chat(chat_request(&messages, Some("cli:default")))
            .await
            .unwrap();
        let mut events = provider
            .chat_stream_incremental(chat_request(&messages, Some("cli:default")))
            .await;
        while events.recv().await.is_some() {}
    }
    let headers: Vec<Option<String>> = server
        .received_requests()
        .await
        .unwrap()
        .iter()
        .map(|request| {
            request
                .headers
                .get("session_id")
                .and_then(|value| value.to_str().ok())
                .map(str::to_string)
        })
        .collect();
    assert_eq!(headers, vec![expected; 4]);
}

/// Review 2: a real chat key, at its longest, keeps its prefix.
#[test]
fn the_longest_chat_key_keeps_its_prefix() {
    let key = crate::domain::sessions::entities::session::user_chat_key(u64::MAX, u64::MAX);
    let prefix = key.split(':').next().unwrap();
    assert!(
        CodexProvider::sanitize_cache_key(&key).starts_with(&format!("{prefix}:")),
        "{key}"
    );
}
