// Issue #1066: OpenAI reasoning models + function tools must be routed via
// the Responses API under BOTH auth modes. Chat Completions rejects reasoning
// models combined with function tools (HTTP 400 "Function tools with
// reasoning_effort are not supported ... Please use /v1/responses instead" —
// reproduced live 2026-07-09 with gpt-5.6-sol). Non-reasoning
// openai-completions models stay on Chat Completions unchanged.

use super::*;
use crate::domain::message::Message;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::domain::tool::ToolDefinition;
use crate::infrastructure::config::Config;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

const RESPONSES_SSE_BODY: &str = concat!(
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n",
);

const CHAT_COMPLETIONS_BODY: &str = r#"{"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

const CHAT_COMPLETIONS_400_BODY: &str = r#"{"error":{"message":"Function tools with reasoning_effort are not supported. Please use /v1/responses instead.","type":"invalid_request_error"}}"#;

/// Mock OpenAI API mirroring production behaviour for reasoning models:
/// chat completions rejects tools with 400; the Responses endpoint succeeds.
async fn mock_openai_rejecting_reasoning_on_chat() -> MockServer {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/chat/completions$"))
        .respond_with(ResponseTemplate::new(400).set_body_string(CHAT_COMPLETIONS_400_BODY))
        .mount(&server)
        .await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/responses$"))
        .respond_with(ResponseTemplate::new(200).set_body_string(RESPONSES_SSE_BODY))
        .mount(&server)
        .await;
    server
}

fn config_with_openai_key(api_base: &str) -> Config {
    serde_json::from_value(serde_json::json!({
        "providers": { "openai": { "api_key": "sk-test-1066", "api_base": api_base } }
    }))
    .expect("config should deserialize")
}

async fn send_turn_with_tools(
    provider: &std::sync::Arc<dyn LlmProvider>,
    model: &str,
) -> Result<crate::domain::message::LlmResponse, crate::domain::error::DomainError> {
    let messages = vec![
        Message::system("You are a coding agent."),
        Message::user("List the files"),
    ];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model,
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    provider.chat(request).await
}

/// AC (#1066): an agent turn with tools against every GPT-5.6 tier over
/// API-key auth completes without HTTP 400, served by the Responses endpoint,
/// with no OAuth-only chatgpt-account-id header.
#[tokio::test]
async fn openai_api_key_reasoning_models_with_tools_use_responses_api_1066() {
    for model in ["gpt-5.6-sol", "gpt-5.6-terra", "gpt-5.6-luna"] {
        let server = mock_openai_rejecting_reasoning_on_chat().await;
        let tmp = tempfile::TempDir::new().unwrap();
        let config = config_with_openai_key(&server.uri());
        let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

        let result = send_turn_with_tools(&provider, &format!("openai-api/{model}")).await;
        assert!(
            result.is_ok(),
            "agent turn for {model} over API-key auth must complete without \
             HTTP 400 (#1066): {result:?}"
        );

        let requests = server.received_requests().await.unwrap();
        let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_string()).collect();
        let responses_req = requests
            .iter()
            .find(|r| r.url.path().ends_with("/responses"))
            .unwrap_or_else(|| {
                panic!(
                    "reasoning model {model} must be served by the Responses \
                     endpoint (#1066); mock saw: {paths:?}"
                )
            });
        // Routing must be up-front per OpenAI's documentation — not an
        // error-driven "try Chat Completions, retry on 400" fallback.
        assert!(
            !paths.iter().any(|p| p.ends_with("/chat/completions")),
            "reasoning model {model} must never touch Chat Completions \
             (#1066); mock saw: {paths:?}"
        );
        assert!(
            !responses_req.headers.contains_key("chatgpt-account-id"),
            "API-key Responses requests must not carry the OAuth-only \
             chatgpt-account-id header (#1066)"
        );
    }
}

/// AC (#1066, guard): non-reasoning openai-completions models keep Chat
/// Completions behaviour exactly as today over API-key auth.
#[tokio::test]
async fn openai_api_key_non_reasoning_models_stay_on_chat_completions_1066() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path_regex(r".*/chat/completions$"))
        .respond_with(
            ResponseTemplate::new(200)
                .insert_header("content-type", "application/json")
                .set_body_string(CHAT_COMPLETIONS_BODY),
        )
        .mount(&server)
        .await;
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_with_openai_key(&server.uri());
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

    let result = send_turn_with_tools(&provider, "openai-api/gpt-5.5").await;
    assert!(result.is_ok(), "gpt-5.5 turn must succeed: {result:?}");

    let requests = server.received_requests().await.unwrap();
    let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_string()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("/chat/completions")),
        "non-reasoning model must stay on Chat Completions; mock saw: {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("/responses")),
        "non-reasoning model must not be rerouted to the Responses API; \
         mock saw: {paths:?}"
    );
}

async fn send_toolless_turn_with_effort(
    provider: &std::sync::Arc<dyn LlmProvider>,
    model: &str,
) -> Result<crate::domain::message::LlmResponse, crate::domain::error::DomainError> {
    let messages = vec![
        Message::system("You are a coding agent."),
        Message::user("Summarize this repo"),
    ];
    let tools: Vec<ToolDefinition> = vec![];
    let request = ChatRequest {
        messages: &messages,
        tools: &tools,
        model,
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: Some(crate::domain::provider::EffortLevel::XHigh),
    };
    provider.chat(request).await
}

/// Review follow-up (#1066): a reasoning model WITHOUT tools must also route
/// to the Responses API — Chat Completions never transmits a configured
/// effort, so leaving tool-less turns there silently drops the setting.
#[tokio::test]
async fn openai_api_key_toolless_reasoning_turn_uses_responses_and_transmits_effort_1066() {
    let server = mock_openai_rejecting_reasoning_on_chat().await;
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_with_openai_key(&server.uri());
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

    let result = send_toolless_turn_with_effort(&provider, "openai-api/gpt-5.6-sol").await;
    assert!(
        result.is_ok(),
        "tool-less reasoning turn must succeed: {result:?}"
    );

    let requests = server.received_requests().await.unwrap();
    let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_string()).collect();
    let responses_req = requests
        .iter()
        .find(|r| r.url.path().ends_with("/responses"))
        .unwrap_or_else(|| {
            panic!(
                "tool-less reasoning turn must use the Responses API (#1066); mock saw: {paths:?}"
            )
        });
    let body: serde_json::Value = serde_json::from_slice(&responses_req.body).unwrap();
    assert_eq!(
        body["reasoning"]["effort"], "xhigh",
        "configured effort must be transmitted on tool-less reasoning turns (#1066)"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("/chat/completions")),
        "tool-less reasoning turn must not fall back to Chat Completions (#1066); saw {paths:?}"
    );
}

/// Review follow-up (#1066): endpoint routing must honour the effective
/// (user-override-aware) model registry, not the builtin one — a
/// `models.json` entry marking a model `reasoning: true` under `openai-api`
/// must route it to the Responses API.
#[tokio::test]
async fn openai_api_key_models_json_reasoning_override_routes_to_responses_1066() {
    let server = mock_openai_rejecting_reasoning_on_chat().await;
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("models.json"),
        r#"{"providers":{"openai-api":{"models":[{"id":"o5-preview","reasoning":true}]}}}"#,
    )
    .unwrap();
    let config = config_with_openai_key(&server.uri());
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();

    let result = send_turn_with_tools(&provider, "openai-api/o5-preview").await;
    assert!(
        result.is_ok(),
        "models.json reasoning override must route o5-preview via the \
         Responses API instead of 400-ing on Chat Completions (#1066): {result:?}"
    );
    let requests = server.received_requests().await.unwrap();
    let paths: Vec<_> = requests.iter().map(|r| r.url.path().to_string()).collect();
    assert!(
        paths.iter().any(|p| p.ends_with("/responses")),
        "override-flagged reasoning model must hit the Responses endpoint (#1066); saw {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p.ends_with("/chat/completions")),
        "override-flagged reasoning model must never touch Chat Completions (#1066); saw {paths:?}"
    );
}
