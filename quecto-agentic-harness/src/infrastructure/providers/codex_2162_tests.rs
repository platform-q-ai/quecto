//! #2162: the `session_id` header, and encrypted reasoning kept and
//! replayed to the model that produced it.
use super::*;
use crate::domain::message::ToolCall;

fn sse(events: &[serde_json::Value]) -> String {
    events
        .iter()
        .map(|event| format!("data: {event}\n\n"))
        .collect()
}

fn reasoning_done(encrypted: Option<&str>) -> serde_json::Value {
    let mut item = serde_json::json!({
        "type": "reasoning",
        "id": "rs_123",
        "summary": [{"type": "summary_text", "text": "why"}],
    });
    if let Some(encrypted) = encrypted {
        item["encrypted_content"] = serde_json::json!(encrypted);
    }
    serde_json::json!({"type": "response.output_item.done", "output_index": 0, "item": item})
}

fn completed() -> serde_json::Value {
    serde_json::json!({"type": "response.completed", "response": {"status": "completed"}})
}

fn encrypted(model: &str, item: serde_json::Value) -> ThinkingBlock {
    ThinkingBlock::EncryptedReasoning {
        model: model.into(),
        item: item.to_string(),
    }
}

fn replay_item() -> serde_json::Value {
    serde_json::json!({"type": "reasoning", "summary": [], "encrypted_content": "gAAA"})
}

#[test]
fn a_reasoning_item_with_encrypted_content_is_kept_without_its_id() {
    let raw = sse(&[reasoning_done(Some("gAAA")), completed()]);
    let response = CodexProvider::parse_sse_response(&raw).unwrap();
    assert_eq!(
        response.thinking_blocks.len(),
        2,
        "{:?}",
        response.thinking_blocks
    );
    let ThinkingBlock::EncryptedReasoning { model, item } = &response.thinking_blocks[1] else {
        panic!("{:?}", response.thinking_blocks)
    };
    assert_eq!(model, "", "stamped by the provider, not the parser");
    let item: serde_json::Value = serde_json::from_str(item).unwrap();
    assert_eq!(
        item,
        serde_json::json!({
            "type": "reasoning",
            "summary": [{"type": "summary_text", "text": "why"}],
            "encrypted_content": "gAAA",
        })
    );
}

#[test]
fn a_reasoning_item_without_encrypted_content_keeps_only_its_summary() {
    for encrypted in [None, Some("")] {
        let raw = sse(&[reasoning_done(encrypted), completed()]);
        let response = CodexProvider::parse_sse_response(&raw).unwrap();
        assert!(
            response
                .thinking_blocks
                .iter()
                .all(ThinkingBlock::is_visible),
            "{:?}",
            response.thinking_blocks
        );
    }
}

#[test]
fn finishing_a_response_stamps_its_reasoning_with_the_requested_model() {
    let raw = sse(&[reasoning_done(Some("gAAA")), completed()]);
    let mut response = CodexProvider::parse_sse_response(&raw).unwrap();
    CodexProvider::finish_response(&mut response, "gpt-6-sol");
    assert!(matches!(
        &response.thinking_blocks[1],
        ThinkingBlock::EncryptedReasoning { model, .. } if model == "gpt-6-sol"
    ));
}

fn tool_turn(blocks: Vec<ThinkingBlock>) -> Vec<Message> {
    let mut assistant = Message::assistant(
        "",
        vec![ToolCall {
            id: "call_1".into(),
            name: "bash".into(),
            arguments: "{}".into(),
        }],
    );
    assistant.thinking_blocks = blocks;
    vec![
        Message::system("sys"),
        Message::user("go"),
        assistant,
        Message::tool("call_1", "done"),
    ]
}

fn types(input: &[serde_json::Value]) -> Vec<String> {
    input
        .iter()
        .map(|item| {
            item["type"]
                .as_str()
                .or(item["role"].as_str())
                .unwrap_or_default()
                .to_string()
        })
        .collect()
}

#[test]
fn reasoning_is_replayed_to_its_own_model_just_before_the_turn_it_led_to() {
    let messages = tool_turn(vec![
        ThinkingBlock::Normal {
            thinking: "why".into(),
            signature: String::new(),
        },
        encrypted("gpt-6-sol", replay_item()),
    ]);
    let (_, input) = CodexProvider::build_input_for(&messages, "gpt-6-sol");
    assert_eq!(
        types(&input),
        ["user", "reasoning", "function_call", "function_call_output"]
    );
    assert_eq!(input[1], replay_item());
}

#[test]
fn reasoning_is_not_replayed_to_another_model_or_an_unstamped_one() {
    for (stamped, requested) in [("gpt-6-sol", "gpt-6"), ("", "gpt-6-sol"), ("gpt-6-sol", "")] {
        let messages = tool_turn(vec![encrypted(stamped, replay_item())]);
        let (_, input) = CodexProvider::build_input_for(&messages, requested);
        assert_eq!(
            types(&input),
            ["user", "function_call", "function_call_output"],
            "stamped {stamped:?}, requested {requested:?}"
        );
    }
}

#[test]
fn reasoning_is_not_replayed_without_the_turn_it_led_to() {
    // Every call orphaned (no result) and no text: nothing follows it.
    let mut messages = tool_turn(vec![encrypted("gpt-6-sol", replay_item())]);
    messages.pop();
    let (_, input) = CodexProvider::build_input_for(&messages, "gpt-6-sol");
    assert_eq!(types(&input), ["user"]);
}

#[test]
fn a_stored_item_that_is_not_an_object_is_skipped() {
    let messages = tool_turn(vec![ThinkingBlock::EncryptedReasoning {
        model: "gpt-6-sol".into(),
        item: "not json".into(),
    }]);
    let (_, input) = CodexProvider::build_input_for(&messages, "gpt-6-sol");
    assert_eq!(
        types(&input),
        ["user", "function_call", "function_call_output"]
    );
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

async fn send_one(provider: CodexProvider, session: Option<&'static str>) {
    let messages = vec![Message::system("sys"), Message::user("hi")];
    provider
        .chat(chat_request(&messages, session))
        .await
        .unwrap();
}

async fn served(session: Option<&'static str>, oauth: bool) -> Vec<Option<String>> {
    let server = wiremock::MockServer::start().await;
    wiremock::Mock::given(wiremock::matchers::method("POST"))
        .respond_with(
            wiremock::ResponseTemplate::new(200)
                .set_body_raw(sse(&[completed()]), "text/event-stream"),
        )
        .mount(&server)
        .await;
    let provider = match oauth {
        true => CodexProvider::with_client(
            "sk-test".into(),
            "acct".into(),
            Some(server.uri()),
            reqwest::Client::new(),
        ),
        false => CodexProvider::with_api_key(
            "sk-test".into(),
            Some(server.uri()),
            reqwest::Client::new(),
        ),
    };
    send_one(provider, session).await;
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
