//! Additional region-coverage unit tests for the OpenAI provider.
//!
//! Focus: pure logic only — request-body construction across all roles,
//! response parsing (success + error paths), SSE delta application
//! (including the tool-call cap), and auth-header building. Header tests
//! use `RequestBuilder::build()` to inspect headers without any network I/O.

use super::*;
use crate::domain::message::Message;
use crate::domain::tool::ToolDefinition;

fn jwt_with_account(account_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{account_id}"}}}}"#
    ));
    format!("{header}.{payload}.sig")
}

fn req<'a>(
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    model: &'a str,
) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools,
        model,
        max_tokens: 256,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

// --- build_request_body: all roles, tool_calls, tool_call_id, tools ---

#[test]
fn build_request_body_maps_all_roles_and_tool_fields() {
    let mut assistant = Message::assistant("calling", vec![]);
    assistant.tool_calls = vec![ToolCall {
        id: "call_1".into(),
        name: "bash".into(),
        arguments: r#"{"command":"ls"}"#.into(),
    }];
    let messages = vec![
        Message::system("sys"),
        Message::user("hi"),
        assistant,
        Message::tool("call_1", "output"),
    ];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "run".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }];
    let body = OpenAiProvider::build_request_body(&req(&messages, &tools, "gpt-5.2"));

    assert_eq!(body["model"], "gpt-5.2");
    assert_eq!(body["max_completion_tokens"], 256);
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs[0]["role"], "system");
    assert_eq!(msgs[1]["role"], "user");
    assert_eq!(msgs[2]["role"], "assistant");
    assert_eq!(msgs[2]["tool_calls"][0]["id"], "call_1");
    assert_eq!(msgs[2]["tool_calls"][0]["function"]["name"], "bash");
    assert_eq!(msgs[3]["role"], "tool");
    assert_eq!(msgs[3]["tool_call_id"], "call_1");
    assert_eq!(body["tools"][0]["function"]["name"], "bash");
}

#[test]
fn build_request_body_omits_tools_when_empty() {
    let messages = vec![Message::user("hi")];
    let body = OpenAiProvider::build_request_body(&req(&messages, &[], "gpt-5.2"));
    assert!(body.get("tools").is_none());
}

// --- parse_response ---

#[test]
fn parse_response_missing_choices_is_error() {
    let body = serde_json::json!({ "usage": {} });
    let err = OpenAiProvider::parse_response(&body).unwrap_err();
    assert!(err.to_string().contains("missing choices"));
}

#[test]
fn parse_response_empty_choices_is_error() {
    let body = serde_json::json!({ "choices": [] });
    let err = OpenAiProvider::parse_response(&body).unwrap_err();
    assert!(err.to_string().contains("empty choices"));
}

#[test]
fn parse_response_extracts_tool_calls_and_usage() {
    let body = serde_json::json!({
        "choices": [{
            "message": {
                "role": "assistant",
                "content": null,
                "tool_calls": [{
                    "id": "c1",
                    "function": { "name": "read", "arguments": "{\"path\":\"a\"}" }
                }]
            }
        }],
        "usage": { "prompt_tokens": 12, "completion_tokens": 4 }
    });
    let resp = OpenAiProvider::parse_response(&body).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "c1");
    assert_eq!(resp.tool_calls[0].name, "read");
    let usage = resp.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 12);
    assert_eq!(usage.completion_tokens, 4);
}

#[test]
fn parse_response_text_without_usage() {
    let body = serde_json::json!({
        "choices": [{ "message": { "role": "assistant", "content": "hello" } }]
    });
    let resp = OpenAiProvider::parse_response(&body).unwrap();
    assert_eq!(resp.content.as_deref(), Some("hello"));
    assert!(resp.tool_calls.is_empty());
    assert!(resp.usage.is_none());
}

// --- apply_delta ---

#[test]
fn apply_delta_appends_content_and_builds_tool_calls() {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();

    OpenAiProvider::apply_delta(
        &serde_json::json!({ "content": "ab" }),
        &mut content,
        &mut tool_calls,
    );
    OpenAiProvider::apply_delta(
        &serde_json::json!({
            "tool_calls": [{ "index": 0, "id": "c1", "function": { "name": "bash", "arguments": "{" } }]
        }),
        &mut content,
        &mut tool_calls,
    );
    OpenAiProvider::apply_delta(
        &serde_json::json!({
            "tool_calls": [{ "index": 0, "function": { "arguments": "}" } }]
        }),
        &mut content,
        &mut tool_calls,
    );

    assert_eq!(content, "ab");
    assert_eq!(tool_calls.len(), 1);
    assert_eq!(tool_calls[0].id, "c1");
    assert_eq!(tool_calls[0].name, "bash");
    assert_eq!(tool_calls[0].arguments, "{}");
}

#[test]
fn apply_delta_skips_index_at_or_above_cap() {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    OpenAiProvider::apply_delta(
        &serde_json::json!({
            "tool_calls": [{ "index": 128, "id": "x", "function": { "name": "y", "arguments": "" } }]
        }),
        &mut content,
        &mut tool_calls,
    );
    assert!(tool_calls.is_empty());
}

#[test]
fn apply_delta_fills_gap_indices() {
    let mut content = String::new();
    let mut tool_calls: Vec<ToolCall> = Vec::new();
    OpenAiProvider::apply_delta(
        &serde_json::json!({
            "tool_calls": [{ "index": 2, "id": "c3", "function": { "name": "n", "arguments": "a" } }]
        }),
        &mut content,
        &mut tool_calls,
    );
    // Indices 0 and 1 are backfilled with empty placeholders.
    assert_eq!(tool_calls.len(), 3);
    assert_eq!(tool_calls[2].id, "c3");
    assert!(tool_calls[0].id.is_empty());
}

// --- parse_sse_response edge cases ---

#[test]
fn parse_sse_response_ignores_malformed_and_choiceless_chunks() {
    let sse = "\
data: not-json\n\
data: {\"id\":\"x\"}\n\
data: {\"choices\":[{\"delta\":{\"content\":\"ok\"}}]}\n\
data: [DONE]\n";
    let resp = OpenAiProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.content.as_deref(), Some("ok"));
}

#[test]
fn parse_sse_response_extracts_usage_chunk() {
    let sse = concat!(
        "data: {\"choices\":[{\"delta\":{\"content\":\"Hello\"}}]}\n\n",
        "data: {\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5,\"total_tokens\":15}}\n\n",
        "data: [DONE]\n\n",
    );
    let response = OpenAiProvider::parse_sse_response(sse).unwrap();
    assert_eq!(response.content.as_deref(), Some("Hello"));
    let usage = response.usage.expect("usage chunk should be captured");
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
}

// --- apply_auth_headers (no network; inspect via build()) ---

#[test]
fn apply_auth_headers_includes_account_id_for_oauth_token() {
    let provider = OpenAiProvider::with_client_and_name_and_oauth_headers(
        "openai",
        jwt_with_account("acct_42"),
        None,
        reqwest::Client::new(),
        true,
    );
    let builder = provider.client.post("http://localhost/v1/chat/completions");
    let request = provider.apply_auth_headers(builder).build().unwrap();
    let headers = request.headers();
    assert!(
        headers
            .get("authorization")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("Bearer ")
    );
    assert_eq!(headers.get("chatgpt-account-id").unwrap(), "acct_42");
}

#[test]
fn apply_auth_headers_omits_account_id_when_absent() {
    let provider = OpenAiProvider::new("sk-test".into(), None);
    let builder = provider.client.post("http://localhost/v1/chat/completions");
    let request = provider.apply_auth_headers(builder).build().unwrap();
    let headers = request.headers();
    assert!(headers.get("authorization").is_some());
    assert!(headers.get("chatgpt-account-id").is_none());
}
