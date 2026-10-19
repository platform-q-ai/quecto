//! Contract (#2593): every provider adapter reports a reply its output
//! limit cut off partway through a tool call the same way — a finished
//! reply that stopped at `MaxTokens`, carrying the call with the argument
//! text it got — never as a provider error. The agent turn recognises the
//! cut-off call from that alone (`output_limit::cut_off_tool_call`).
//!
//! Each vendor's own truncation signal is the input: Anthropic's
//! `stop_reason: max_tokens`, OpenAI's `finish_reason: length`, and the
//! Responses API's `response.incomplete` with `incomplete_details.reason:
//! max_output_tokens`, through every call surface: the incremental stream
//! and the whole reply (`chat`). Anthropic documents a `tool_use` block
//! that ends a `max_tokens` reply as incomplete; a whole reply carries its
//! input already parsed, so the adapter reports that call with no
//! arguments, which the domain reads as cut off.

use quecto::application::providers::ports::{ChatRequest, LlmProvider};
use quecto::domain::conversation::value_objects::message::{
    LlmResponse, Message, StopReason, ToolArguments,
};
use quecto::domain::inference::services::output_limit::cut_off_tool_call;
use quecto::domain::inference::value_objects::provider::StreamEvent;
use quecto::infrastructure::providers::{
    anthropic::AnthropicProvider, codex::CodexProvider, openai::OpenAiProvider,
};
use wiremock::{Mock, MockServer, ResponseTemplate, matchers::method};

fn json_server_body(body: serde_json::Value) -> ResponseTemplate {
    ResponseTemplate::new(200)
        .insert_header("content-type", "application/json")
        .set_body_json(body)
}

async fn serving_json(body: serde_json::Value) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(json_server_body(body))
        .mount(&server)
        .await;
    server
}

/// The whole reply `chat` returns; no error.
async fn whole(provider: &dyn LlmProvider) -> LlmResponse {
    let messages = [
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    provider
        .chat(request(&messages))
        .await
        .expect("a cut-off reply is no error")
}

/// The reply stopped at the limit and the domain sees its `write` call
/// cut off, whatever argument text the surface kept.
fn assert_write_cut_off(response: &LlmResponse) {
    assert_eq!(response.stop_reason, Some(StopReason::MaxTokens));
    let cut = cut_off_tool_call(response).expect("the domain sees the cut-off call");
    assert_eq!(cut.tool, "write");
}

/// The argument text the output limit cut off, mid-string.
const CUT: &str = r#"{"path":"big.txt","content":"line 1\nli"#;

fn request<'a>(messages: &'a [Message]) -> ChatRequest<'a> {
    ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools: &[],
        model: "fixture-model",
        max_tokens: 8192,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

async fn serving(body: String) -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "text/event-stream")
                .set_body_string(body),
        )
        .mount(&server)
        .await;
    server
}

fn client() -> reqwest::Client {
    reqwest::Client::builder().no_proxy().build().unwrap()
}

/// The reply the incremental stream finished with; no error event.
async fn streamed(provider: &dyn LlmProvider) -> LlmResponse {
    let messages = [
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    let mut rx = provider.chat_stream_incremental(request(&messages)).await;
    let mut done = None;
    while let Some(event) = rx.recv().await {
        match event {
            StreamEvent::Done(response) => done = Some(response),
            StreamEvent::Error(error) => panic!("a cut-off reply is no error: {error}"),
            _ => {}
        }
    }
    done.expect("the stream finishes the cut-off reply")
}

fn assert_cut_off_write(response: &LlmResponse) {
    assert_eq!(response.stop_reason, Some(StopReason::MaxTokens));
    assert_eq!(response.tool_calls.len(), 1, "{response:?}");
    let call = &response.tool_calls[0];
    assert_eq!(call.name, "write");
    assert_eq!(call.argument_shape(), ToolArguments::Invalid(CUT));
    let cut = cut_off_tool_call(response).expect("the domain sees the cut-off call");
    assert_eq!(cut.tool, "write");
}

fn anthropic_body() -> String {
    let events = [
        (
            "message_start",
            serde_json::json!({"type":"message_start","message":{"id":"m1","model":"fixture-model","usage":{"input_tokens":5,"output_tokens":0}}}),
        ),
        (
            "content_block_start",
            serde_json::json!({"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"toolu_1","name":"write","input":{}}}),
        ),
        (
            "content_block_delta",
            serde_json::json!({"type":"content_block_delta","index":0,"delta":{"type":"input_json_delta","partial_json":CUT}}),
        ),
        (
            "content_block_stop",
            serde_json::json!({"type":"content_block_stop","index":0}),
        ),
        (
            "message_delta",
            serde_json::json!({"type":"message_delta","delta":{"stop_reason":"max_tokens"},"usage":{"output_tokens":8192}}),
        ),
        ("message_stop", serde_json::json!({"type":"message_stop"})),
    ];
    events
        .iter()
        .map(|(name, data)| format!("event: {name}\ndata: {data}\n\n"))
        .collect()
}

fn openai_body() -> String {
    let chunks = [
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_1","type":"function","function":{"name":"write","arguments":""}}]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"function":{"arguments":CUT}}]}}]}),
        serde_json::json!({"choices":[{"index":0,"delta":{},"finish_reason":"length"}]}),
    ];
    let mut body: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    body.push_str("data: [DONE]\n\n");
    body
}

fn responses_body(reason: &str) -> String {
    let events = [
        serde_json::json!({"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_1","name":"write","arguments":""}}),
        serde_json::json!({"type":"response.function_call_arguments.delta","output_index":0,"delta":CUT}),
        serde_json::json!({"type":"response.incomplete","response":{"status":"incomplete","incomplete_details":{"reason":reason},"usage":{"input_tokens":5,"output_tokens":8192}}}),
    ];
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}

#[tokio::test]
async fn anthropic_reports_a_tool_call_cut_off_at_max_tokens() {
    let server = serving(anthropic_body()).await;
    let provider = AnthropicProvider::with_client("fixture".into(), Some(server.uri()), client());
    assert_cut_off_write(&streamed(&provider).await);
}

#[tokio::test]
async fn openai_reports_a_tool_call_cut_off_at_length() {
    let server = serving(openai_body()).await;
    let provider = OpenAiProvider::with_client("fixture".into(), Some(server.uri()), client());
    assert_cut_off_write(&streamed(&provider).await);
}

#[tokio::test]
async fn codex_reports_a_tool_call_cut_off_at_max_output_tokens_when_streaming() {
    let server = serving(responses_body("max_output_tokens")).await;
    let provider = CodexProvider::new("fixture".into(), "acct".into(), Some(server.uri()));
    assert_cut_off_write(&streamed(&provider).await);
}

#[tokio::test]
async fn codex_reports_a_tool_call_cut_off_at_max_output_tokens_as_a_whole_reply() {
    let server = serving(responses_body("max_output_tokens")).await;
    let provider = CodexProvider::new("fixture".into(), "acct".into(), Some(server.uri()));
    let messages = [
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    let response = provider
        .chat(request(&messages))
        .await
        .expect("a cut-off reply is no error");
    assert_cut_off_write(&response);
}

/// The allowlist holds: an incomplete reply for any other reason (a
/// content filter) is still the provider failure it was.
#[tokio::test]
async fn codex_reports_another_incomplete_reason_as_a_failure() {
    let server = serving(responses_body("content_filter")).await;
    let provider = CodexProvider::new("fixture".into(), "acct".into(), Some(server.uri()));
    let messages = [
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    let error = provider
        .chat(request(&messages))
        .await
        .expect_err("a filtered reply stays a failure");
    assert!(error.to_string().contains("content_filter"), "{error}");
}

#[tokio::test]
async fn anthropic_reports_a_tool_use_ending_a_max_tokens_reply_as_cut_off_as_a_whole_reply() {
    let server = serving_json(serde_json::json!({
        "id": "m1", "type": "message", "role": "assistant", "model": "fixture-model",
        "content": [
            {"type": "text", "text": "Writing it now."},
            {"type": "tool_use", "id": "toolu_1", "name": "write", "input": {"path": "big.txt"}}
        ],
        "stop_reason": "max_tokens",
        "usage": {"input_tokens": 5, "output_tokens": 8192}
    }))
    .await;
    let provider = AnthropicProvider::with_client("fixture".into(), Some(server.uri()), client());
    let response = whole(&provider).await;
    assert_write_cut_off(&response);
    assert_eq!(
        response.tool_calls[0].argument_shape(),
        ToolArguments::Empty
    );
}

#[tokio::test]
async fn anthropic_keeps_a_whole_tool_use_that_ended_its_reply_normally() {
    let server = serving_json(serde_json::json!({
        "id": "m1", "type": "message", "role": "assistant", "model": "fixture-model",
        "content": [
            {"type": "tool_use", "id": "toolu_1", "name": "write", "input": {"path": "a.txt"}}
        ],
        "stop_reason": "tool_use",
        "usage": {"input_tokens": 5, "output_tokens": 20}
    }))
    .await;
    let provider = AnthropicProvider::with_client("fixture".into(), Some(server.uri()), client());
    let response = whole(&provider).await;
    assert_eq!(cut_off_tool_call(&response), None);
    assert_eq!(response.tool_calls[0].arguments, r#"{"path":"a.txt"}"#);
}

#[tokio::test]
async fn openai_reports_a_tool_call_cut_off_at_length_as_a_whole_reply() {
    let server = serving_json(serde_json::json!({
        "id": "c1", "object": "chat.completion", "model": "fixture-model",
        "choices": [{
            "index": 0,
            "message": {"role": "assistant", "content": null, "tool_calls": [
                {"id": "call_1", "type": "function", "function": {"name": "write", "arguments": CUT}}
            ]},
            "finish_reason": "length"
        }],
        "usage": {"prompt_tokens": 5, "completion_tokens": 8192, "total_tokens": 8197}
    }))
    .await;
    let provider = OpenAiProvider::with_client("fixture".into(), Some(server.uri()), client());
    assert_cut_off_write(&whole(&provider).await);
}

/// The allowlist holds for an incomplete reply that names no reason.
#[tokio::test]
async fn codex_reports_an_incomplete_reply_without_a_reason_as_a_failure() {
    let body = sse_lines(&[
        serde_json::json!({"type":"response.output_text.delta","delta":"Hi"}),
        serde_json::json!({"type":"response.incomplete","response":{"status":"incomplete"}}),
    ]);
    let server = serving(body).await;
    let provider = CodexProvider::new("fixture".into(), "acct".into(), Some(server.uri()));
    let messages = [
        Message::system("You write files."),
        Message::user("write the big file"),
    ];
    let error = provider
        .chat(request(&messages))
        .await
        .expect_err("an unexplained incomplete reply stays a failure");
    assert!(error.to_string().contains("response.incomplete"), "{error}");
}

fn sse_lines(events: &[serde_json::Value]) -> String {
    events.iter().map(|e| format!("data: {e}\n\n")).collect()
}
