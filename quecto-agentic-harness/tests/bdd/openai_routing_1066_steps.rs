//! Issue #1066: OpenAI models must follow OpenAI's documented endpoint and
//! reasoning-effort rules (both auth modes).
//!
//! Endpoint routing: OpenAI reasoning models driven with function tools must
//! be routed to the Responses API under both auth modes (Chat Completions
//! rejects them with HTTP 400 "Function tools with reasoning_effort are not
//! supported ... Please use /v1/responses instead"). Non-reasoning models —
//! including third-party openai-completions providers — stay on Chat
//! Completions unchanged.
//!
//! Effort vocabulary: OpenAI's documented scale (none, low, medium, high,
//! xhigh) must be configurable; unknown strings are rejected naming the valid
//! values.

use super::*;

use quecto::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use quecto::interface::cli::build_agent_provider;
use wiremock::matchers::{method, path_regex};
use wiremock::{Mock, MockServer, ResponseTemplate};

/// ChatGPT account id embedded in the test OAuth JWT.
const OAUTH_ACCOUNT_ID: &str = "acct-1066";

/// API key used by the API-key-auth scenarios.
const TEST_API_KEY: &str = "sk-test-1066";

/// Owns a dedicated tokio runtime together with the wiremock server so that
/// starting, querying, and (crucially) dropping the server all happen inside
/// a tokio reactor — cucumber's own executor is futures_executor, which has
/// none.
pub struct MockOpenAiApi {
    rt: tokio::runtime::Runtime,
    server: Option<MockServer>,
}

impl std::fmt::Debug for MockOpenAiApi {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MockOpenAiApi").finish_non_exhaustive()
    }
}

impl MockOpenAiApi {
    fn new<F>(configure: F) -> Self
    where
        F: for<'a> FnOnce(
            &'a MockServer,
        ) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + 'a>>,
    {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .expect("build tokio runtime for mock OpenAI API");
        let server = rt.block_on(async {
            let server = MockServer::start().await;
            configure(&server).await;
            server
        });
        Self {
            rt,
            server: Some(server),
        }
    }

    fn uri(&self) -> String {
        self.server.as_ref().expect("server alive").uri()
    }

    fn requests(&self) -> Vec<wiremock::Request> {
        let server = self.server.as_ref().expect("server alive");
        self.rt
            .block_on(server.received_requests())
            .expect("request recording is enabled on MockServer::start()")
    }

    fn block_on<F: std::future::Future>(&self, fut: F) -> F::Output {
        self.rt.block_on(fut)
    }
}

impl Drop for MockOpenAiApi {
    fn drop(&mut self) {
        // Drop the server on a plain thread that enters the runtime handle:
        // wiremock's Drop needs a tokio reactor, and doing this off-thread
        // keeps the current thread free of an EnterGuard when `rt` itself
        // drops (and avoids a double panic if a scenario is already
        // unwinding).
        if let Some(server) = self.server.take() {
            let handle = self.rt.handle().clone();
            let _ = std::thread::spawn(move || {
                let _guard = handle.enter();
                drop(server);
            })
            .join();
        }
    }
}

/// SSE body a Responses-API endpoint returns for a successful turn.
const RESPONSES_SSE_BODY: &str = concat!(
    "data: {\"type\":\"response.output_text.delta\",\"delta\":\"done\"}\n",
    "data: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":1,\"output_tokens\":1}}}\n",
);

/// JSON body a Chat Completions endpoint returns for a successful turn.
const CHAT_COMPLETIONS_BODY: &str = r#"{"choices":[{"message":{"role":"assistant","content":"done"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}"#;

/// OpenAI's live rejection of reasoning models + function tools on Chat
/// Completions (reproduced 2026-07-09 with gpt-5.6-sol).
const CHAT_COMPLETIONS_400_BODY: &str = r#"{"error":{"message":"Function tools with reasoning_effort are not supported. Please use /v1/responses instead.","type":"invalid_request_error"}}"#;

/// Map a feature-level endpoint name to its URL path suffix.
fn endpoint_path_suffix(endpoint: &str) -> &'static str {
    match endpoint {
        "Responses" => "/responses",
        "Chat Completions" => "/chat/completions",
        other => panic!("unknown endpoint name in feature file: {other}"),
    }
}

/// Fake ChatGPT OAuth JWT carrying the `chatgpt_account_id` claim.
fn test_openai_oauth_jwt(account_id: &str) -> String {
    use base64::Engine;
    let header = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(r#"{"alg":"none"}"#);
    let payload = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(format!(
        r#"{{"https://api.openai.com/auth":{{"chatgpt_account_id":"{}"}}}}"#,
        account_id
    ));
    format!("{}.{}.sig", header, payload)
}

#[given("OpenAI's Chat Completions endpoint rejects reasoning models combined with function tools")]
fn given_chat_completions_rejects_reasoning_with_tools(world: &mut QuectoWorld) {
    world.openai_mock = Some(MockOpenAiApi::new(|server| {
        Box::pin(async move {
            Mock::given(method("POST"))
                .and(path_regex(r".*/chat/completions$"))
                .respond_with(ResponseTemplate::new(400).set_body_string(CHAT_COMPLETIONS_400_BODY))
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/responses$"))
                .respond_with(ResponseTemplate::new(200).set_body_string(RESPONSES_SSE_BODY))
                .mount(server)
                .await;
        })
    }));
}

#[given("OpenAI's Chat Completions endpoint accepts agent turns with tools")]
fn given_chat_completions_accepts_turns(world: &mut QuectoWorld) {
    world.openai_mock = Some(MockOpenAiApi::new(|server| {
        Box::pin(async move {
            Mock::given(method("POST"))
                .and(path_regex(r".*/chat/completions$"))
                .respond_with(
                    ResponseTemplate::new(200)
                        .insert_header("content-type", "application/json")
                        .set_body_string(CHAT_COMPLETIONS_BODY),
                )
                .mount(server)
                .await;
            Mock::given(method("POST"))
                .and(path_regex(r".*/responses$"))
                .respond_with(ResponseTemplate::new(200).set_body_string(RESPONSES_SSE_BODY))
                .mount(server)
                .await;
        })
    }));
}

fn base_dir_and_mock_uri(world: &QuectoWorld) -> (std::path::PathBuf, String) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let uri = world
        .openai_mock
        .as_ref()
        .expect("OpenAI endpoint behaviour not configured — add the endpoint Given step")
        .uri();
    (base, uri)
}

fn build_provider_from_config(world: &mut QuectoWorld, config_json: serde_json::Value) {
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("no base dir — add 'Given a temp base directory'");
    let config: Config = serde_json::from_value(config_json).expect("provider config should parse");
    let provider = build_agent_provider(&config, &base, &reqwest::Client::new())
        .expect("agent provider should build");
    world.provider = Some(provider);
}

#[given("an agent provider configured with an OpenAI API key")]
fn given_agent_provider_with_openai_api_key(world: &mut QuectoWorld) {
    let (_base, uri) = base_dir_and_mock_uri(world);
    build_provider_from_config(
        world,
        serde_json::json!({
            "providers": { "openai": { "api_key": TEST_API_KEY, "api_base": uri } }
        }),
    );
}

#[given("an agent provider configured with ChatGPT OAuth credentials")]
fn given_agent_provider_with_chatgpt_oauth(world: &mut QuectoWorld) {
    let (base, uri) = base_dir_and_mock_uri(world);
    let store = CredentialStore::new(&base);
    store
        .store(Credential {
            provider: "openai".to_string(),
            token: test_openai_oauth_jwt(OAUTH_ACCOUNT_ID),
            method: AuthMethod::OAuth,
            expires_at: Some(i64::MAX),
            refresh_token: Some("rt-1066".to_string()),
            account_id: Some(OAUTH_ACCOUNT_ID.to_string()),
        })
        .expect("store openai OAuth credential");
    build_provider_from_config(
        world,
        serde_json::json!({
            "providers": { "openai": { "api_base": uri } }
        }),
    );
}

#[given(
    expr = "an agent provider configured with a third-party openai-completions endpoint {string}"
)]
fn given_agent_provider_with_third_party_endpoint(world: &mut QuectoWorld, prefix: String) {
    let (_base, uri) = base_dir_and_mock_uri(world);
    build_provider_from_config(
        world,
        serde_json::json!({
            "providers": {
                "openai_compatible": {
                    "endpoints": [{
                        "prefix": prefix,
                        "api_key": "sk-third-party-1066",
                        "api_base": uri
                    }]
                }
            }
        }),
    );
}

#[when(expr = "I send an agent turn with tools for model {string}")]
fn when_send_agent_turn_with_tools(world: &mut QuectoWorld, model: String) {
    let provider = world
        .provider
        .clone()
        .expect("agent provider not built — add the provider Given step");
    let mock = world
        .openai_mock
        .as_ref()
        .expect("OpenAI endpoint behaviour not configured");
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
        model: &model,
        max_tokens: 1024,
        temperature: 0.0,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let result = mock
        .block_on(provider.chat(request))
        .map_err(|e| e.to_string());
    world.agent_turn_result = Some(result);
}

#[then("the agent turn should complete without an HTTP 400")]
fn then_turn_completes_without_400(world: &mut QuectoWorld) {
    match world
        .agent_turn_result
        .as_ref()
        .expect("no agent turn was sent")
    {
        Ok(_) => {}
        Err(e) => panic!(
            "agent turn failed (#1066 requires it to complete without HTTP 400): {}",
            e
        ),
    }
}

#[then(expr = "the turn should have been served via the {string} endpoint")]
fn then_turn_served_via_endpoint(world: &mut QuectoWorld, endpoint: String) {
    let suffix = endpoint_path_suffix(&endpoint);
    let paths = request_paths(world);
    assert!(
        paths.iter().any(|p| p.ends_with(suffix)),
        "expected the turn to be served via the {endpoint} endpoint (#1066), \
         but the requests were: {paths:?}"
    );
}

#[then(expr = "no request should have reached the {string} endpoint")]
fn then_no_request_reached_endpoint(world: &mut QuectoWorld, endpoint: String) {
    let suffix = endpoint_path_suffix(&endpoint);
    let paths = request_paths(world);
    assert!(
        !paths.iter().any(|p| p.ends_with(suffix)),
        "no request must reach the {endpoint} endpoint (#1066 documented \
         up-front routing, not error-driven fallback); requests: {paths:?}"
    );
}

fn responses_request(world: &QuectoWorld) -> wiremock::Request {
    world
        .openai_mock
        .as_ref()
        .expect("OpenAI endpoint behaviour not configured")
        .requests()
        .into_iter()
        .find(|r| r.url.path().ends_with("/responses"))
        .expect("no request reached the Responses endpoint")
}

#[then("the Responses request should authenticate with the API key only")]
fn then_responses_request_api_key_only(world: &mut QuectoWorld) {
    let req = responses_request(world);
    let auth = req
        .headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        auth,
        format!("Bearer {TEST_API_KEY}"),
        "API-key Responses requests must authenticate with the configured key (#1066)"
    );
    assert!(
        !req.headers.contains_key("chatgpt-account-id"),
        "API-key-authenticated Responses requests must not carry the OAuth-only \
         'chatgpt-account-id' header (#1066 auth decoupling)"
    );
}

#[then("the Responses request should carry the ChatGPT account identity")]
fn then_responses_request_carries_account_identity(world: &mut QuectoWorld) {
    let req = responses_request(world);
    let account = req
        .headers
        .get("chatgpt-account-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_string();
    assert_eq!(
        account, OAUTH_ACCOUNT_ID,
        "OAuth-authenticated Responses requests must keep carrying the \
         'chatgpt-account-id' header unchanged (#1066)"
    );
}

fn request_paths(world: &QuectoWorld) -> Vec<String> {
    world
        .openai_mock
        .as_ref()
        .expect("OpenAI endpoint behaviour not configured")
        .requests()
        .iter()
        .map(|r| r.url.path().to_string())
        .collect()
}

// --- Effort vocabulary at the CLI configuration surface ---

/// Sentinel flag placed *after* `--effort`: flag parsing short-circuits on
/// the first error, so its distinctive error only appears when the effort
/// value was accepted — a positive, network-free signal that parsing
/// progressed past the effort flag.
const SENTINEL_ARGS: [&str; 2] = ["--max-iterations", "0"];
const SENTINEL_ERROR: &str = "--max-iterations requires a positive integer";

#[when(expr = "I run the agent CLI with effort {string}")]
fn when_run_agent_cli_with_effort(world: &mut QuectoWorld, effort: String) {
    let mut args = vec![
        "quecto".to_string(),
        "agent".to_string(),
        "--effort".to_string(),
        effort,
    ];
    args.extend(SENTINEL_ARGS.iter().map(|s| s.to_string()));
    args.push("hello".to_string());
    let output = cli::run_with_output(args, &world.cli_context);
    world.exit_code = output.exit_code;
    world.stdout = output.stdout;
    world.stderr = output.stderr;
}

#[then(expr = "the CLI should accept the effort level {string}")]
fn then_cli_accepts_effort(world: &mut QuectoWorld, effort: String) {
    assert!(
        !world.stderr.contains("invalid effort level"),
        "effort level '{}' must be accepted at configuration time (#1066); \
         stderr: {}",
        effort,
        world.stderr
    );
    assert!(
        world.stderr.contains(SENTINEL_ERROR),
        "the CLI must progress past the accepted effort flag '{}' to the \
         sentinel flag (#1066); stderr: {}",
        effort,
        world.stderr
    );
}

#[then(expr = "the CLI should reject the effort level {string}")]
fn then_cli_rejects_effort(world: &mut QuectoWorld, effort: String) {
    assert!(
        world
            .stderr
            .contains(&format!("invalid effort level '{}'", effort)),
        "expected rejection of effort '{}'; stderr: {}",
        effort,
        world.stderr
    );
}

#[then(expr = "the error should name the valid effort values {string}")]
fn then_error_names_valid_effort_values(world: &mut QuectoWorld, valid: String) {
    assert!(
        world.stderr.contains(&valid),
        "the rejection must name the valid values \"{}\" (#1066); stderr: {}",
        valid,
        world.stderr
    );
}
