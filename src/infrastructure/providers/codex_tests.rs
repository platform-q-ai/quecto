use super::*;
use crate::domain::tool::ToolDefinition;

#[test]
fn test_build_input_basic_messages() {
    let messages = vec![
        Message::system("You are helpful."),
        Message::user("Hello"),
        Message::assistant("Hi there!", vec![]),
    ];
    let (instructions, input) = CodexProvider::build_input(&messages);
    assert_eq!(instructions.unwrap(), "You are helpful.");
    assert_eq!(input.len(), 2);
    assert_eq!(input[0]["role"], "user");
    assert_eq!(input[0]["content"], "Hello");
    assert_eq!(input[1]["role"], "assistant");
    assert_eq!(input[1]["content"], "Hi there!");
}

#[test]
fn test_build_input_tool_calls() {
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![ToolCall {
        id: "call_123".to_string(),
        name: "get_weather".into(),
        arguments: r#"{"location":"Paris"}"#.to_string(),
    }];

    let tool_msg = Message::tool("call_123", "sunny");

    let messages = vec![Message::user("Weather?"), assistant_msg, tool_msg];
    let (instructions, input) = CodexProvider::build_input(&messages);
    assert!(instructions.is_none());
    assert_eq!(input.len(), 3);
    // User message
    assert_eq!(input[0]["role"], "user");
    // Function call
    assert_eq!(input[1]["type"], "function_call");
    assert_eq!(input[1]["call_id"], "call_123");
    assert_eq!(input[1]["name"], "get_weather");
    // Function call output
    assert_eq!(input[2]["type"], "function_call_output");
    assert_eq!(input[2]["call_id"], "call_123");
    assert_eq!(input[2]["output"], "sunny");
}

#[test]
fn test_build_tools() {
    let tools = vec![ToolDefinition {
        name: "read".into(),
        description: "Read a file".into(),
        parameters_schema: r#"{"type":"object","properties":{"path":{"type":"string"}}}"#.into(),
    }];
    let result = CodexProvider::build_tools(&tools);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["type"], "function");
    assert_eq!(result[0]["name"], "read");
    assert!(result[0]["parameters"]["properties"]["path"].is_object());
}

#[test]
fn test_build_request_body() {
    let messages = vec![Message::system("Be concise."), Message::user("Hi")];
    let tools = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-5.1-codex",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let body = CodexProvider::build_request_body(&request);
    assert_eq!(body["model"], "gpt-5.1-codex");
    assert_eq!(body["instructions"], "Be concise.");
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["input"].as_array().unwrap().len(), 1);
    assert!(body.get("tools").is_none());
}

// --- prompt_cache_key ---

#[test]
fn test_build_request_body_includes_prompt_cache_key_when_session_id_set() {
    let messages = vec![Message::user("Hi")];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "gpt-5.3-codex",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: Some("cli:default".to_string()),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let body = CodexProvider::build_request_body(&request);
    let key = body["prompt_cache_key"]
        .as_str()
        .expect("prompt_cache_key should be present");
    // Sanitized key: prefix preserved, raw value hidden behind 8-hex-char digest.
    assert!(
        key.starts_with("cli:"),
        "expected key to start with 'cli:', got: {}",
        key
    );
    assert_eq!(
        key.len(),
        "cli:".len() + 8,
        "expected 8-hex-char digest after prefix, got: {}",
        key
    );
}

#[test]
fn test_build_request_body_omits_prompt_cache_key_when_no_session_id() {
    let messages = vec![Message::user("Hi")];
    let request = ChatRequest {
        messages: &messages,
        tools: &[],
        model: "gpt-5.3-codex",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let body = CodexProvider::build_request_body(&request);
    assert!(
        body.get("prompt_cache_key").is_none(),
        "expected prompt_cache_key to be absent, got: {:?}",
        body.get("prompt_cache_key")
    );
}

// --- sanitize_cache_key ---

#[test]
fn test_sanitize_cache_key_hides_telegram_id() {
    // Telegram session keys contain numeric chat IDs — must not appear verbatim.
    let raw = "telegram:12345";
    let sanitized = CodexProvider::sanitize_cache_key(raw);
    assert!(
        sanitized.starts_with("telegram:"),
        "prefix should be preserved: {}",
        sanitized
    );
    assert!(
        !sanitized.contains("12345"),
        "chat ID must not appear verbatim in: {}",
        sanitized
    );
    // Must be exactly "telegram:" + 8 hex chars
    assert_eq!(sanitized.len(), "telegram:".len() + 8);
}

#[test]
fn test_sanitize_cache_key_is_stable() {
    // Same input must produce same output across calls.
    let key = "cli:my-session";
    assert_eq!(
        CodexProvider::sanitize_cache_key(key),
        CodexProvider::sanitize_cache_key(key)
    );
}

#[test]
fn test_sanitize_cache_key_different_inputs_produce_different_hashes() {
    let a = CodexProvider::sanitize_cache_key("cli:session-a");
    let b = CodexProvider::sanitize_cache_key("cli:session-b");
    // Different sessions must map to different cache keys (no collisions on these inputs).
    assert_ne!(a, b, "different sessions must have different cache keys");
}

#[test]
fn test_sanitize_cache_key_no_colon_uses_full_as_prefix() {
    let key = "bare-session";
    let sanitized = CodexProvider::sanitize_cache_key(key);
    assert!(
        sanitized.starts_with("bare-session:"),
        "key without colon should use full value as prefix: {}",
        sanitized
    );
}

#[test]
fn test_build_request_body_responses_api_fields() {
    let messages = vec![Message::user("Hi")];
    let tools = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model: "gpt-5.3-codex",
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
    };
    let body = CodexProvider::build_request_body(&request);
    assert_eq!(body["tool_choice"], "auto");
    assert_eq!(body["parallel_tool_calls"], true);
    assert_eq!(body["reasoning"]["effort"], "medium");
    assert_eq!(body["reasoning"]["summary"], "auto");
    assert_eq!(body["text"]["verbosity"], "medium");
    let include = body["include"].as_array().unwrap();
    assert!(
        include
            .iter()
            .any(|v| v.as_str() == Some("reasoning.encrypted_content"))
    );
    assert!(body.get("max_completion_tokens").is_none());
}

#[test]
fn test_build_tools_includes_strict_false() {
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let result = CodexProvider::build_tools(&tools);
    assert_eq!(result.len(), 1);
    assert_eq!(result[0]["strict"], false);
}

#[test]
fn test_parse_sse_tool_call_after_reasoning_item() {
    // Reasoning item at output_index 0, function_call at output_index 1
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_x","name":"bash","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"cmd\""}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":":\"ls\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":3}}}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call_x");
    assert_eq!(resp.tool_calls[0].name, "bash");
    assert_eq!(resp.tool_calls[0].arguments, r#"{"cmd":"ls"}"#);
}

#[test]
fn test_parse_sse_multiple_tool_calls_after_reasoning() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"c1","name":"read","arguments":""}}
data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","call_id":"c2","name":"write","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":\"a.rs\"}"}
data: {"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"content\":\"hi\"}"}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.tool_calls.len(), 2);
    assert_eq!(resp.tool_calls[0].name, "read");
    assert_eq!(resp.tool_calls[0].arguments, r#"{"path":"a.rs"}"#);
    assert_eq!(resp.tool_calls[1].name, "write");
    assert_eq!(resp.tool_calls[1].arguments, r#"{"content":"hi"}"#);
}

#[test]
fn test_parse_response_text() {
    let body = serde_json::json!({
        "output": [
            {
                "type": "message",
                "content": [
                    { "type": "output_text", "text": "Hello!" }
                ]
            }
        ],
        "usage": {
            "input_tokens": 10,
            "output_tokens": 5,
        }
    });
    let resp = CodexProvider::parse_response(&body).unwrap();
    assert_eq!(resp.content.unwrap(), "Hello!");
    assert!(resp.tool_calls.is_empty());
    let usage = resp.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 10);
    assert_eq!(usage.completion_tokens, 5);
}

#[test]
fn test_parse_response_tool_call() {
    let body = serde_json::json!({
        "output": [
            {
                "type": "function_call",
                "call_id": "call_abc",
                "name": "shell",
                "arguments": "{\"command\":\"ls\"}"
            }
        ]
    });
    let resp = CodexProvider::parse_response(&body).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call_abc");
    assert_eq!(resp.tool_calls[0].name, "shell");
    assert_eq!(resp.tool_calls[0].arguments, r#"{"command":"ls"}"#);
}

#[test]
fn test_parse_sse_text_response() {
    let sse = r#"data: {"type":"response.output_text.delta","delta":"Hello"}
data: {"type":"response.output_text.delta","delta":" world"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":8,"output_tokens":2}}}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert_eq!(resp.content.unwrap(), "Hello world");
    let usage = resp.usage.unwrap();
    assert_eq!(usage.prompt_tokens, 8);
    assert_eq!(usage.completion_tokens, 2);
}

#[test]
fn test_parse_sse_tool_call() {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_x","name":"shell","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"cmd\""}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":":\"ls\"}"}
data: {"type":"response.function_call_arguments.done","output_index":0,"arguments":"{\"cmd\":\"ls\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":3}}}
data: [DONE]
"#;
    let resp = CodexProvider::parse_sse_response(sse).unwrap();
    assert!(resp.content.is_none());
    assert_eq!(resp.tool_calls.len(), 1);
    assert_eq!(resp.tool_calls[0].id, "call_x");
    assert_eq!(resp.tool_calls[0].name, "shell");
    assert_eq!(resp.tool_calls[0].arguments, r#"{"cmd":"ls"}"#);
}

#[tokio::test]
async fn test_codex_provider_http_error() {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(401).set_body_string("unauthorized"))
        .mount(&server)
        .await;

    let provider = CodexProvider::new(
        "test-token".to_string(),
        "acct-123".to_string(),
        Some(server.uri()),
    );
    let messages = vec![Message::system("You are helpful."), Message::user("hi")];
    let result = provider
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.1-codex",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
        })
        .await;
    assert!(result.is_err());
    let err = result.unwrap_err().to_string();
    assert!(err.contains("401"), "expected 401 in error: {}", err);
}

#[tokio::test]
async fn test_codex_provider_success() {
    let server = wiremock::MockServer::start().await;
    let sse_body = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hi!\"}\n\
                         data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":3,\"output_tokens\":1}}}\n\
                         data: [DONE]\n";
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(wiremock::ResponseTemplate::new(200).set_body_string(sse_body))
        .mount(&server)
        .await;

    let provider = CodexProvider::new(
        "test-token".to_string(),
        "acct-123".to_string(),
        Some(server.uri()),
    );
    let messages = vec![Message::system("You are helpful."), Message::user("hello")];
    let result = provider
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.1-codex",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
        })
        .await;
    let resp = result.unwrap();
    assert_eq!(resp.content.unwrap(), "Hi!");
}

#[tokio::test]
async fn test_codex_provider_rejects_provider_qualified_model_name() {
    let provider = CodexProvider::new("test-token".to_string(), "acct-123".to_string(), None);
    let messages = vec![Message::system("You are helpful."), Message::user("hello")];

    let result = provider
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model: "openai/gpt-5.3-codex",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
        })
        .await;

    let err = result.expect_err("provider-qualified model should be rejected");
    assert!(
        err.to_string().contains("bare model id"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
async fn test_codex_provider_rejects_missing_instructions() {
    let provider = CodexProvider::new("test-token".to_string(), "acct-123".to_string(), None);
    let messages = vec![Message::user("hello")];

    let result = provider
        .chat(ChatRequest {
            messages: &messages,
            tools: &[],
            model: "gpt-5.3-codex",
            max_tokens: 1024,
            temperature: 0.7,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
        })
        .await;

    let err = result.expect_err("missing instructions should be rejected");
    assert!(
        err.to_string().contains("requires instructions"),
        "unexpected error: {err}"
    );
}

// ===================================================================
// Issue #192: Orphaned function_call/function_call_output repair
// ===================================================================

#[test]
fn test_build_input_orphaned_function_call_removed() {
    // Assistant sends function_call but no matching tool result exists
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![ToolCall {
        id: "call_orphan".to_string(),
        name: "bash".into(),
        arguments: "{}".to_string(),
    }];
    let messages = vec![Message::user("go"), assistant_msg];
    let (_instructions, input) = CodexProvider::build_input(&messages);
    // Orphaned function_call must be removed
    let orphan_present = input
        .iter()
        .any(|item| item["type"] == "function_call" && item["call_id"] == "call_orphan");
    assert!(
        !orphan_present,
        "orphaned function_call should be removed from input, got: {:?}",
        input
    );
}

#[test]
fn test_build_input_orphaned_function_call_output_removed() {
    // Tool result exists but no matching function_call assistant message
    let tool_msg = Message::tool("call_orphan", "some result");
    let messages = vec![Message::user("go"), tool_msg];
    let (_instructions, input) = CodexProvider::build_input(&messages);
    // Orphaned function_call_output must be removed
    let orphan_present = input
        .iter()
        .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_orphan");
    assert!(
        !orphan_present,
        "orphaned function_call_output should be removed from input, got: {:?}",
        input
    );
}

#[test]
fn test_build_input_valid_matched_pair_preserved() {
    // Both function_call and function_call_output present — should be kept
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![ToolCall {
        id: "call_valid".to_string(),
        name: "read".into(),
        arguments: r#"{"path":"foo.rs"}"#.to_string(),
    }];
    let tool_msg = Message::tool("call_valid", "file content");
    let messages = vec![Message::user("read it"), assistant_msg, tool_msg];
    let (_instructions, input) = CodexProvider::build_input(&messages);

    let has_call = input
        .iter()
        .any(|item| item["type"] == "function_call" && item["call_id"] == "call_valid");
    let has_output = input
        .iter()
        .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_valid");
    assert!(has_call, "matched function_call should be preserved");
    assert!(
        has_output,
        "matched function_call_output should be preserved"
    );
}

#[test]
fn test_build_input_mixed_valid_and_orphaned() {
    // One valid pair + one orphaned function_call
    let mut good_assistant = Message::assistant("", vec![]);
    good_assistant.tool_calls = vec![ToolCall {
        id: "call_good".to_string(),
        name: "read".into(),
        arguments: "{}".to_string(),
    }];
    let good_tool = Message::tool("call_good", "result");

    let mut bad_assistant = Message::assistant("", vec![]);
    bad_assistant.tool_calls = vec![ToolCall {
        id: "call_bad".to_string(),
        name: "bash".into(),
        arguments: "{}".to_string(),
    }];
    // No matching tool result for call_bad

    let messages = vec![
        Message::user("start"),
        good_assistant,
        good_tool,
        bad_assistant,
    ];
    let (_instructions, input) = CodexProvider::build_input(&messages);

    let has_good_call = input
        .iter()
        .any(|item| item["type"] == "function_call" && item["call_id"] == "call_good");
    let has_good_output = input
        .iter()
        .any(|item| item["type"] == "function_call_output" && item["call_id"] == "call_good");
    let has_bad = input
        .iter()
        .any(|item| item.get("call_id").and_then(|v| v.as_str()) == Some("call_bad"));

    assert!(has_good_call, "matched call should be kept");
    assert!(has_good_output, "matched output should be kept");
    assert!(!has_bad, "orphaned call_bad should be removed");
}

#[test]
fn test_build_input_all_tool_calls_orphaned_fallback_to_text() {
    // When ALL tool calls on an assistant message are orphaned, the assistant's
    // narrative text content must not be silently dropped.
    let mut assistant_msg = Message::assistant("I was going to call a tool.", vec![]);
    assistant_msg.tool_calls = vec![ToolCall {
        id: "call_orphan".to_string(),
        name: "bash".into(),
        arguments: "{}".to_string(),
    }];
    // Deliberately no matching tool result message.

    let messages = vec![Message::user("Do something"), assistant_msg];
    let (_instructions, input) = CodexProvider::build_input(&messages);

    // The orphaned function_call must be absent.
    let has_orphan = input
        .iter()
        .any(|item| item["type"] == "function_call" && item["call_id"] == "call_orphan");
    assert!(!has_orphan, "orphaned function_call should be removed");

    // The assistant text content must be preserved.
    let has_text = input.iter().any(|item| {
        item["role"] == "assistant" && item["content"] == "I was going to call a tool."
    });
    assert!(
        has_text,
        "assistant text content must not be silently dropped"
    );
}

// --- #209: Shared reqwest::Client ---

#[test]
fn test_codex_provider_accepts_shared_client() {
    let client = reqwest::Client::new();
    let provider =
        CodexProvider::with_client("sk-test".to_string(), "acct-123".to_string(), None, client);
    assert_eq!(provider.name(), "codex");
}
