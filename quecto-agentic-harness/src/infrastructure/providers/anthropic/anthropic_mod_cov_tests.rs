// Region-coverage tests for anthropic/mod.rs.
//
// Targets branches not exercised elsewhere: header construction (OAuth vs
// API key), cancellation short-circuits, attach_cost, parse_response error
// and OAuth-remap arms, tool_result image blocks, cache-control array/no-user
// branches, tool_choice OAuth remap, metadata edge cases, invalid tool schema.
// All inputs are in-memory; cancellation tests short-circuit before any I/O.

use super::*;
use crate::domain::message::{LlmResponse, Message, UsageInfo};
use crate::domain::provider::{CancelFlag, ChatRequest, RequestMetadata, StreamEvent, ToolChoice};
use crate::domain::tool::{ImageBlock, ToolDefinition};

fn base_req<'a>(
    messages: &'a [Message],
    tools: &'a [ToolDefinition],
    model: &'a str,
) -> ChatRequest<'a> {
    ChatRequest {
        messages,
        tools,
        model,
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    }
}

// --- is_oauth getter ---------------------------------------------------------

#[test]
fn is_oauth_true_for_oat_token() {
    let p = AnthropicProvider::new("sk-ant-oat01-abc".to_string(), None);
    assert!(p.is_oauth());
}

#[test]
fn is_oauth_false_for_api_key() {
    let p = AnthropicProvider::new("sk-ant-test".to_string(), None);
    assert!(!p.is_oauth());
}

// --- apply_headers -----------------------------------------------------------

#[test]
fn apply_headers_oauth_sets_bearer_and_identity() {
    let provider = AnthropicProvider::new("sk-ant-oat01-secret".to_string(), None);
    let client = reqwest::Client::new();
    let rb = client.post("http://localhost/v1/messages");
    let request = provider
        .apply_headers(rb, "claude-sonnet-4-6")
        .build()
        .expect("build request");
    let h = request.headers();
    assert_eq!(
        h.get("Authorization").unwrap(),
        "Bearer sk-ant-oat01-secret"
    );
    assert!(
        h.get("user-agent")
            .unwrap()
            .to_str()
            .unwrap()
            .starts_with("claude-cli/")
    );
    assert_eq!(h.get("x-app").unwrap(), "cli");
    assert!(h.get("x-api-key").is_none());
    assert_eq!(h.get("Accept").unwrap(), "application/json");
}

#[test]
fn apply_headers_api_key_sets_x_api_key() {
    let provider = AnthropicProvider::new("sk-ant-test".to_string(), None);
    let client = reqwest::Client::new();
    let rb = client.post("http://localhost/v1/messages");
    let request = provider
        .apply_headers(rb, "claude-sonnet-4-5")
        .build()
        .expect("build request");
    let h = request.headers();
    assert_eq!(h.get("x-api-key").unwrap(), "sk-ant-test");
    assert!(h.get("Authorization").is_none());
    assert_eq!(h.get("anthropic-version").unwrap(), "2023-06-01");
}

// --- attach_cost -------------------------------------------------------------

fn response_with_usage() -> LlmResponse {
    LlmResponse {
        content: Some("x".to_string()),
        tool_calls: vec![],
        usage: Some(UsageInfo {
            prompt_tokens: 1000,
            completion_tokens: 1000,
            cache_read_tokens: None,
            cache_write_tokens: None,
            context_tokens: None,
            cost: None,
        }),
        stop_reason: None,
        thinking_blocks: vec![],
    }
}

#[test]
fn attach_cost_sets_cost_for_known_model() {
    let mut resp = response_with_usage();
    AnthropicProvider::attach_cost(&mut resp, "claude-opus-4-6");
    assert!(resp.usage.unwrap().cost.is_some());
}

#[test]
fn attach_cost_leaves_none_for_unknown_model() {
    let mut resp = response_with_usage();
    AnthropicProvider::attach_cost(&mut resp, "totally-unknown-model-xyz");
    assert!(resp.usage.unwrap().cost.is_none());
}

#[test]
fn attach_cost_noop_when_no_usage() {
    let mut resp = LlmResponse {
        content: Some("x".to_string()),
        tool_calls: vec![],
        usage: None,
        stop_reason: None,
        thinking_blocks: vec![],
    };
    AnthropicProvider::attach_cost(&mut resp, "claude-opus-4-6");
    assert!(resp.usage.is_none());
}

// --- parse_response edge arms ------------------------------------------------

#[test]
fn parse_response_missing_content_is_err() {
    let body = serde_json::json!({"stop_reason": "end_turn"});
    let err = AnthropicProvider::parse_response(&body, false, &[]).unwrap_err();
    assert!(err.to_string().contains("missing content"));
}

#[test]
fn parse_response_oauth_remaps_tool_name() {
    use std::borrow::Cow;
    let tools = vec![ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("read"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let body = serde_json::json!({
        "content": [{"type": "tool_use", "id": "t1", "name": "Read", "input": {"path": "x"}}],
        "stop_reason": "tool_use"
    });
    let resp = AnthropicProvider::parse_response(&body, true, &tools).unwrap();
    assert_eq!(resp.tool_calls[0].name, "read");
}

#[test]
fn parse_response_without_usage_or_stop_reason() {
    let body = serde_json::json!({"content": [{"type": "text", "text": "hi"}]});
    let resp = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(resp.content.as_deref(), Some("hi"));
    assert!(resp.usage.is_none());
    assert!(resp.stop_reason.is_none());
}

#[test]
fn parse_response_ignores_unknown_block_type() {
    let body = serde_json::json!({
        "content": [{"type": "image", "source": {}}, {"type": "text", "text": "ok"}]
    });
    let resp = AnthropicProvider::parse_response(&body, false, &[]).unwrap();
    assert_eq!(resp.content.as_deref(), Some("ok"));
}

// --- build_tool_result_block: image blocks -----------------------------------

#[test]
fn tool_result_block_with_images_uses_array_content() {
    let mut m = Message::tool("tc1", "see attached");
    m.image_blocks.push(ImageBlock {
        mime_type: "image/png",
        data: "base64data".to_string(),
    });
    let json = AnthropicProvider::build_tool_result_message_public(&m);
    let block = &json["content"][0];
    let inner = block["content"].as_array().expect("array content");
    assert_eq!(inner[0]["type"], "text");
    assert_eq!(inner[1]["type"], "image");
    assert_eq!(inner[1]["source"]["media_type"], "image/png");
    assert_eq!(inner[1]["source"]["data"], "base64data");
}

// --- cache control: array branch & no-user branch ----------------------------

#[test]
fn cache_control_applied_to_tool_result_array_message() {
    let messages = vec![
        Message::user("go"),
        Message::assistant(
            "",
            vec![crate::domain::message::ToolCall {
                id: "tc1".into(),
                name: "bash".into(),
                arguments: "{}".into(),
            }],
        ),
        Message::tool("tc1", "done"),
    ];
    let req = base_req(&messages, &[], "claude-sonnet-4-6");
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    let last = body["messages"].as_array().unwrap().last().unwrap();
    let blocks = last["content"].as_array().expect("array content");
    let last_block = blocks.last().unwrap();
    assert_eq!(last_block["cache_control"]["type"], "ephemeral");
}

#[test]
fn cache_control_no_user_message_does_not_panic() {
    let messages = vec![Message::system("sys"), Message::assistant("hi", vec![])];
    let req = base_req(&messages, &[], "claude-sonnet-4-6");
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    // Only the assistant message remains; build must succeed without a user msg.
    let msgs = body["messages"].as_array().unwrap();
    assert_eq!(msgs.len(), 1);
    assert_eq!(msgs[0]["role"], "assistant");
}

// --- tool_choice Specific with OAuth remap -----------------------------------

#[test]
fn tool_choice_specific_oauth_remaps_name() {
    use std::borrow::Cow;
    let tools = vec![ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("read"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let messages = vec![Message::user("hi")];
    let mut req = base_req(&messages, &tools, "claude-sonnet-4-5");
    req.tool_choice = Some(ToolChoice::Specific("read".into()));
    let (_sys, body) = AnthropicProvider::build_request_body_with_oauth(&req, true);
    assert_eq!(body["tool_choice"]["type"], "tool");
    assert_eq!(body["tool_choice"]["name"], "Read");
}

// --- metadata present but user_id None ---------------------------------------

#[test]
fn metadata_present_without_user_id_omits_metadata() {
    let messages = vec![Message::user("hi")];
    let mut req = base_req(&messages, &[], "claude-sonnet-4-6");
    req.metadata = Some(RequestMetadata { user_id: None });
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert!(body.get("metadata").is_none() || body["metadata"].is_null());
}

// --- build_tool_defs invalid schema falls back to default --------------------

#[test]
fn tool_defs_invalid_schema_becomes_null() {
    use std::borrow::Cow;
    let tools = vec![ToolDefinition {
        name: Cow::Borrowed("custom"),
        description: Cow::Borrowed("desc"),
        parameters_schema: Cow::Borrowed("this is not json"),
    }];
    let messages = vec![Message::user("hi")];
    let req = base_req(&messages, &tools, "claude-sonnet-4-6");
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    assert!(body["tools"][0]["input_schema"].is_null());
}

// --- cancellation short-circuits (no network) --------------------------------

#[tokio::test]
async fn chat_cancelled_before_send_returns_error() {
    let provider = AnthropicProvider::new("sk-ant-test".to_string(), None);
    let flag = CancelFlag::new();
    flag.cancel();
    let messages = vec![Message::user("hi")];
    let mut req = base_req(&messages, &[], "claude-sonnet-4-6");
    req.cancel_flag = Some(flag);
    let err = provider.chat(req).await.unwrap_err();
    assert!(err.to_string().contains("cancelled"));
}

#[tokio::test]
async fn chat_stream_cancelled_before_send_returns_error() {
    let provider = AnthropicProvider::new("sk-ant-test".to_string(), None);
    let flag = CancelFlag::new();
    flag.cancel();
    let messages = vec![Message::user("hi")];
    let mut req = base_req(&messages, &[], "claude-sonnet-4-6");
    req.cancel_flag = Some(flag);
    let err = provider.chat_stream(req).await.unwrap_err();
    assert!(err.to_string().contains("cancelled"));
}

#[tokio::test]
async fn chat_stream_incremental_cancelled_emits_error_event() {
    let provider = AnthropicProvider::new("sk-ant-test".to_string(), None);
    let flag = CancelFlag::new();
    flag.cancel();
    let messages = vec![Message::user("hi")];
    let mut req = base_req(&messages, &[], "claude-sonnet-4-6");
    req.cancel_flag = Some(flag);
    let mut rx = provider.chat_stream_incremental(req).await;
    match rx.recv().await {
        Some(StreamEvent::Error(e)) => assert!(e.contains("cancelled")),
        other => panic!("expected Error event, got {:?}", other),
    }
}
