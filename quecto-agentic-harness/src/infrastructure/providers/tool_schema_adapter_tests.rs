//! Every provider adapter sends the spawn tool's schema intact and inside
//! the portable subset: `container` typed as boolean or object, so no model
//! is left to guess its type (and send a quoted JSON string).

use crate::application::providers::ports::ChatRequest;
use crate::application::tools::ports::Tool;
use crate::domain::conversation::value_objects::message::Message;
use crate::domain::tool_policy::value_objects::tool::ToolDefinition;
use crate::infrastructure::providers::anthropic::AnthropicProvider;
use crate::infrastructure::providers::codex::CodexProvider;
use crate::infrastructure::providers::openai::OpenAiProvider;
use crate::infrastructure::test_support::tool_schema_portability::portability_violations;
use crate::infrastructure::tools::spawn::SpawnTool;
use serde_json::Value;

fn spawn_definition() -> ToolDefinition {
    SpawnTool::new(vec![]).definition()
}

fn request<'a>(messages: &'a [Message], tools: &'a [ToolDefinition]) -> ChatRequest<'a> {
    ChatRequest {
        trace: None,
        admission: None,
        messages,
        tools,
        model: "model-under-test",
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

/// The adapter sent the tool's own schema, unchanged, and it is portable
/// with `container` typed as a boolean or one object per accepted shape.
fn assert_spawn_schema_sent_intact(adapter: &str, sent: &Value, definition: &ToolDefinition) {
    let own: Value =
        serde_json::from_str(&definition.parameters_schema).expect("the spawn schema is JSON");
    assert_eq!(sent, &own, "{adapter} altered the spawn schema");
    assert_eq!(
        portability_violations(sent),
        Vec::<String>::new(),
        "{adapter} sends a spawn schema outside the portable subset"
    );
    let branch_types: Vec<&str> = sent["properties"]["container"]["anyOf"]
        .as_array()
        .unwrap_or_else(|| panic!("{adapter}: container has no anyOf"))
        .iter()
        .map(|branch| branch["type"].as_str().expect("a typed branch"))
        .collect();
    assert_eq!(
        branch_types,
        ["boolean", "object", "object", "object"],
        "{adapter}"
    );
}

#[test]
fn anthropic_api_key_sends_the_typed_spawn_schema() {
    let tools = [spawn_definition()];
    let messages = [Message::user("hi")];
    let (_system, body) =
        AnthropicProvider::build_request_body_with_oauth(&request(&messages, &tools), false);
    assert_eq!(body["tools"][0]["name"], "spawn");
    assert_spawn_schema_sent_intact(
        "anthropic (API key)",
        &body["tools"][0]["input_schema"],
        &tools[0],
    );
}

#[test]
fn anthropic_oauth_sends_the_typed_spawn_schema() {
    let tools = [spawn_definition()];
    let messages = [Message::user("hi")];
    let (_system, body) =
        AnthropicProvider::build_request_body_with_oauth(&request(&messages, &tools), true);
    let sent = body["tools"]
        .as_array()
        .expect("tools sent")
        .iter()
        .find(|tool| {
            tool["name"]
                .as_str()
                .is_some_and(|n| n.eq_ignore_ascii_case("spawn"))
        })
        .expect("the spawn tool is sent under its Claude Code name");
    assert_spawn_schema_sent_intact("anthropic (OAuth)", &sent["input_schema"], &tools[0]);
}

#[test]
fn openai_chat_completions_sends_the_typed_spawn_schema() {
    let tools = [spawn_definition()];
    let messages = [Message::user("hi")];
    let openai =
        OpenAiProvider::build_chat_completions_body_for_test("openai", &request(&messages, &tools));
    assert_eq!(openai["tools"][0]["function"]["name"], "spawn");
    assert_spawn_schema_sent_intact(
        "chat completions",
        &openai["tools"][0]["function"]["parameters"],
        &tools[0],
    );
    // xAI and OpenAI-compatible endpoints share the adapter, and the
    // provider name does not shape the tool JSON.
    for provider in ["xai", "fireworks"] {
        let body = OpenAiProvider::build_chat_completions_body_for_test(
            provider,
            &request(&messages, &tools),
        );
        assert_eq!(body["tools"], openai["tools"], "{provider}");
    }
}

#[test]
fn openai_responses_sends_the_typed_spawn_schema_non_strict() {
    let tools = [spawn_definition()];
    let messages = [Message::user("hi")];
    for (backend, body) in [
        (
            "responses (API key)",
            CodexProvider::build_request_body_public_api_key(&request(&messages, &tools)),
        ),
        (
            "responses (ChatGPT Codex OAuth)",
            CodexProvider::build_request_body_public_oauth(&request(&messages, &tools)),
        ),
    ] {
        assert_eq!(body["tools"][0]["name"], "spawn", "{backend}");
        // Strict mode would demand every property be required; the
        // optional spawn fields rely on it staying off.
        assert_eq!(body["tools"][0]["strict"], false, "{backend}");
        assert_spawn_schema_sent_intact(backend, &body["tools"][0]["parameters"], &tools[0]);
    }
}
