// Tests for #437: Anthropic provider parity with Pi and OpenCode.
//
// Covers: Claude Code system prompt, beta headers, tool name remapping,
// thinking block replay, signature_delta, Accept header, stop reasons.

use super::*;
use crate::domain::message::{Message, StopReason, ThinkingBlock};
use crate::domain::provider::ChatRequest;

// #437: Anthropic provider parity with Pi and OpenCode
// ===========================================================================

// --- #437-1: Claude Code system prompt for OAuth ---

#[test]
fn test_oauth_prepends_claude_code_system_prompt() {
    let messages = vec![Message::system("Be helpful"), Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_with_oauth(&req, true);
    let system = body["system"].as_array().expect("system should be array");
    assert_eq!(system.len(), 2, "OAuth should have 2 system blocks");
    assert_eq!(
        system[0]["text"].as_str().unwrap(),
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
    assert_eq!(system[1]["text"].as_str().unwrap(), "Be helpful");
    // Both should have cache_control
    assert!(system[0]["cache_control"]["type"].as_str() == Some("ephemeral"));
    assert!(system[1]["cache_control"]["type"].as_str() == Some("ephemeral"));
}

#[test]
fn test_oauth_without_system_prompt_still_has_claude_code_identity() {
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_with_oauth(&req, true);
    let system = body["system"].as_array().expect("system should be array");
    assert_eq!(system.len(), 1);
    assert_eq!(
        system[0]["text"].as_str().unwrap(),
        "You are Claude Code, Anthropic's official CLI for Claude."
    );
}

#[test]
fn test_api_key_does_not_prepend_claude_code_system_prompt() {
    let messages = vec![Message::system("Be helpful"), Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_with_oauth(&req, false);
    let system = body["system"].as_array().expect("system should be array");
    assert_eq!(system.len(), 1);
    assert_eq!(system[0]["text"].as_str().unwrap(), "Be helpful");
}

// --- #437-2,3,7,9: Beta headers ---

#[test]
fn test_beta_header_api_key_non_46_model() {
    let beta = AnthropicProvider::build_beta_header_public("claude-sonnet-4-20250514", false);
    assert!(beta.contains("fine-grained-tool-streaming-2025-05-14"));
    assert!(beta.contains("interleaved-thinking-2025-05-14"));
    assert!(!beta.contains("claude-code-20250219"));
    assert!(!beta.contains("oauth-2025-04-20"));
}

#[test]
fn test_beta_header_api_key_46_model() {
    let beta = AnthropicProvider::build_beta_header_public("claude-opus-4-6", false);
    assert!(beta.contains("fine-grained-tool-streaming-2025-05-14"));
    assert!(!beta.contains("interleaved-thinking")); // omitted for 4.6
    assert!(!beta.contains("claude-code"));
}

#[test]
fn test_beta_header_oauth_non_46_model() {
    let beta = AnthropicProvider::build_beta_header_public("claude-sonnet-4-20250514", true);
    assert!(beta.contains("claude-code-20250219"));
    assert!(beta.contains("oauth-2025-04-20"));
    assert!(beta.contains("fine-grained-tool-streaming-2025-05-14"));
    assert!(beta.contains("interleaved-thinking-2025-05-14"));
}

#[test]
fn test_beta_header_oauth_46_model() {
    let beta = AnthropicProvider::build_beta_header_public("claude-sonnet-4-6", true);
    assert!(beta.contains("claude-code-20250219"));
    assert!(beta.contains("oauth-2025-04-20"));
    assert!(beta.contains("fine-grained-tool-streaming-2025-05-14"));
    assert!(!beta.contains("interleaved-thinking")); // omitted for 4.6
}

// --- #437-4: Tool name remapping ---

#[test]
fn test_to_claude_code_name_remaps_known_tools() {
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("read"),
        "Read"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("bash"),
        "Bash"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("edit"),
        "Edit"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("write"),
        "Write"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("grep"),
        "Grep"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("glob"),
        "Glob"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("webfetch"),
        "WebFetch"
    );
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("websearch"),
        "WebSearch"
    );
}

#[test]
fn test_to_claude_code_name_passes_unknown_through() {
    assert_eq!(
        AnthropicProvider::to_claude_code_name_public("my_custom_tool"),
        "my_custom_tool"
    );
}

#[test]
fn test_tool_defs_remapped_in_oauth_mode() {
    use std::borrow::Cow;
    let tools = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("Read a file"),
        parameters_schema: Cow::Borrowed(
            r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
        ),
    }];
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_with_oauth(&req, true);
    let tool_name = body["tools"][0]["name"].as_str().unwrap();
    assert_eq!(tool_name, "Read");
}

#[test]
fn test_tool_defs_not_remapped_in_api_key_mode() {
    use std::borrow::Cow;
    let tools = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("Read a file"),
        parameters_schema: Cow::Borrowed(
            r#"{"type":"object","properties":{"path":{"type":"string"}},"required":["path"]}"#,
        ),
    }];
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let (_sys, body) = AnthropicProvider::build_request_body_public(&req);
    let tool_name = body["tools"][0]["name"].as_str().unwrap();
    assert_eq!(tool_name, "read");
}

// --- #437-5: Thinking block replay ---

#[test]
fn test_assistant_message_with_normal_thinking_block() {
    use crate::domain::message::ThinkingBlock;
    let mut msg = Message::assistant("response text", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "Let me reason".to_string(),
        signature: "sig123".to_string(),
    });
    let json = AnthropicProvider::build_assistant_message(&msg, false);
    let content = json["content"].as_array().unwrap();
    let thinking_block = content
        .iter()
        .find(|b| b["type"] == "thinking")
        .expect("should have thinking block");
    assert_eq!(
        thinking_block["thinking"].as_str().unwrap(),
        "Let me reason"
    );
    assert_eq!(thinking_block["signature"].as_str().unwrap(), "sig123");
}

#[test]
fn test_assistant_message_with_redacted_thinking_block() {
    use crate::domain::message::ThinkingBlock;
    let mut msg = Message::assistant("response text", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Redacted {
        data: "opaque_data_abc".to_string(),
    });
    let json = AnthropicProvider::build_assistant_message(&msg, false);
    let content = json["content"].as_array().unwrap();
    let redacted = content
        .iter()
        .find(|b| b["type"] == "redacted_thinking")
        .expect("should have redacted_thinking");
    assert_eq!(redacted["data"].as_str().unwrap(), "opaque_data_abc");
}

#[test]
fn test_thinking_block_empty_signature_falls_back_to_text() {
    use crate::domain::message::ThinkingBlock;
    let mut msg = Message::assistant("response text", vec![]);
    msg.thinking_blocks.push(ThinkingBlock::Normal {
        thinking: "some reasoning".to_string(),
        signature: "".to_string(),
    });
    let json = AnthropicProvider::build_assistant_message(&msg, false);
    let content = json["content"].as_array().unwrap();
    // Should be a text block, not a thinking block
    assert!(
        content.iter().all(|b| b["type"] != "thinking"),
        "should NOT have thinking block"
    );
    let text_blocks: Vec<_> = content.iter().filter(|b| b["type"] == "text").collect();
    assert!(
        text_blocks
            .iter()
            .any(|b| b["text"].as_str().unwrap() == "some reasoning")
    );
}

// --- #437-6: signature_delta SSE handling ---

#[test]
fn test_sse_signature_delta_accumulates_signature() {
    use super::anthropic_sse::SseAccumulator;
    let mut acc = SseAccumulator::default();

    // Start a thinking block
    acc.handle_block_start(&serde_json::json!({"content_block": {"type": "thinking"}}));

    // Thinking delta
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "thinking_delta", "thinking": "reasoning"}}),
    );

    // Signature delta
    acc.handle_block_delta(
        &serde_json::json!({"delta": {"type": "signature_delta", "signature": "sig_abc"}}),
    );

    // Stop block
    acc.handle_block_stop();

    let blocks = acc.thinking_blocks();
    assert_eq!(blocks.len(), 1);
    match &blocks[0] {
        ThinkingBlock::Normal {
            thinking,
            signature,
        } => {
            assert_eq!(thinking, "reasoning");
            assert_eq!(signature, "sig_abc");
        }
        _ => panic!("expected Normal thinking block"),
    }
}

// --- #437-10: Accept header ---

#[tokio::test]
async fn test_accept_header_is_sent() {
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    let server = MockServer::start().await;
    let response_body = serde_json::json!({
        "id": "msg_accept",
        "type": "message",
        "role": "assistant",
        "content": [{"type": "text", "text": "ok"}],
        "stop_reason": "end_turn",
        "usage": {"input_tokens": 5, "output_tokens": 2}
    });
    Mock::given(method("POST"))
        .and(path("/v1/messages"))
        .and(header("Accept", "application/json"))
        .respond_with(ResponseTemplate::new(200).set_body_json(&response_body))
        .expect(1)
        .mount(&server)
        .await;

    let provider = AnthropicProvider::new("sk-ant-test".to_string(), Some(server.uri()));
    let messages = vec![Message::user("Hi")];
    let req = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "claude-sonnet-4-20250514",
        max_tokens: 1024,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let result = provider.chat(req).await;
    assert!(
        result.is_ok(),
        "chat should succeed with Accept header: {:?}",
        result
    );
}

// --- #437-15,16: Stop reason handling ---
// (These are already covered by StopReason::parse tests in domain/message.rs
//  but we verify here for completeness)

#[test]
fn test_pause_turn_maps_to_end_turn() {
    assert_eq!(StopReason::parse("pause_turn"), StopReason::EndTurn);
}

#[test]
fn test_sensitive_maps_to_error() {
    assert_eq!(StopReason::parse("sensitive"), StopReason::Error);
}

// --- #438: SSE streaming reverse-maps OAuth tool names ---

#[test]
fn test_sse_batch_reverse_maps_tool_name_with_tool_defs() {
    use std::borrow::Cow;
    let sse = "\
        event: content_block_start\n\
        data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"Read\"}}\n\n\
        event: content_block_delta\n\
        data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"path\\\":\\\"foo\\\"}\"}}\n\n\
        event: content_block_stop\n\
        data: {}\n\n\
        event: message_stop\n\
        data: {}\n\n";
    let tool_defs = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("read"),
        description: Cow::Borrowed("Read a file"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let resp = AnthropicProvider::parse_sse_response_with_tools_public(sse, &tool_defs).unwrap();
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].name, "read");
}

#[test]
fn test_sse_batch_no_remap_without_tool_defs() {
    let sse = "\
        event: content_block_start\n\
        data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"bash\"}}\n\n\
        event: content_block_delta\n\
        data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n\
        event: content_block_stop\n\
        data: {}\n\n\
        event: message_stop\n\
        data: {}\n\n";
    let resp = AnthropicProvider::parse_sse_response_public(sse).unwrap();
    assert_eq!(resp.tool_calls[0].name, "bash");
}

#[test]
fn test_sse_events_reverse_maps_tool_name_in_start_and_end() {
    use std::borrow::Cow;
    let sse = "\
        event: content_block_start\n\
        data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"Bash\"}}\n\n\
        event: content_block_delta\n\
        data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"command\\\":\\\"ls\\\"}\"}}\n\n\
        event: content_block_stop\n\
        data: {}\n\n\
        event: message_stop\n\
        data: {}\n\n";
    let tool_defs = vec![crate::domain::tool::ToolDefinition {
        name: Cow::Borrowed("bash"),
        description: Cow::Borrowed("Run command"),
        parameters_schema: Cow::Borrowed("{}"),
    }];
    let events = AnthropicProvider::parse_sse_events_with_tools_public(sse, &tool_defs);
    use crate::domain::provider::StreamEvent;

    // ToolCallStart should have remapped name
    let start = events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::ToolCallStart { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("no ToolCallStart");
    assert_eq!(start, "bash");

    // ToolCallEnd should have remapped name
    let end = events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::ToolCallEnd { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("no ToolCallEnd");
    assert_eq!(end, "bash");

    // Done response should have remapped name
    let done_name = events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::Done(resp) if !resp.tool_calls.is_empty() => {
                Some(resp.tool_calls[0].name.clone())
            }
            _ => None,
        })
        .expect("no Done with tool call");
    assert_eq!(done_name, "bash");
}

#[test]
fn test_sse_events_no_remap_without_tool_defs() {
    let sse = "\
        event: content_block_start\n\
        data: {\"content_block\":{\"type\":\"tool_use\",\"id\":\"toolu_01\",\"name\":\"bash\"}}\n\n\
        event: content_block_delta\n\
        data: {\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{}\"}}\n\n\
        event: content_block_stop\n\
        data: {}\n\n\
        event: message_stop\n\
        data: {}\n\n";
    let events = AnthropicProvider::parse_sse_events_public(sse);
    use crate::domain::provider::StreamEvent;
    let start = events
        .iter()
        .find_map(|ev| match ev {
            StreamEvent::ToolCallStart { name, .. } => Some(name.clone()),
            _ => None,
        })
        .expect("no ToolCallStart");
    assert_eq!(start, "bash");
}
