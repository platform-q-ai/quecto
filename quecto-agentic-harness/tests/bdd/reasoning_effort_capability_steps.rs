//! Steps for `reasoning_effort_capability.feature` (#1996, #1848).

use super::uds_steps::find_agent_response;
use super::*;
use quecto::domain::message::Message;

// ─── chat-completions request builder ───────────────────────────────────────

#[given(expr = "a chat-completions request for model {string} with effort {string}")]
fn given_chat_request_with_effort(world: &mut QuectoWorld, model: String, effort: String) {
    world.env_overrides.insert("_cc_model".into(), model);
    world.env_overrides.insert("_cc_effort".into(), effort);
}

#[given(expr = "a chat-completions request for model {string} with no effort")]
fn given_chat_request_without_effort(world: &mut QuectoWorld, model: String) {
    world.env_overrides.insert("_cc_model".into(), model);
    world.env_overrides.remove("_cc_effort");
}

#[when("the OpenAI-compatible provider builds the chat-completions request")]
fn when_provider_builds_chat_request(world: &mut QuectoWorld) {
    let model = world
        .env_overrides
        .get("_cc_model")
        .cloned()
        .expect("no model — add a 'Given a chat-completions request' step");
    let effort = world.env_overrides.get("_cc_effort").map(|e| {
        quecto::domain::provider::EffortLevel::parse(e)
            .unwrap_or_else(|| panic!("effort level '{e}' must parse"))
    });
    let messages = vec![Message::user("hi")];
    let request = quecto::application::providers::ports::ChatRequest {
        trace: None,
        admission: None,
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 256,
        temperature: 0.2,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort,
    };
    let body =
        quecto::infrastructure::providers::openai::OpenAiProvider::build_chat_completions_body_for_test(
            &request,
        );
    world
        .env_overrides
        .insert("_cc_body".into(), body.to_string());
}

fn chat_body(world: &QuectoWorld) -> serde_json::Value {
    serde_json::from_str(
        world
            .env_overrides
            .get("_cc_body")
            .expect("the request was not built"),
    )
    .expect("body is JSON")
}

#[then(expr = "the chat-completions body should set {string} to {string}")]
fn then_chat_body_sets(world: &mut QuectoWorld, key: String, value: String) {
    let body = chat_body(world);
    assert_eq!(body[&key], serde_json::Value::String(value), "{body}");
}

#[then(expr = "the chat-completions body should not contain {string}")]
fn then_chat_body_lacks(world: &mut QuectoWorld, key: String) {
    let body = chat_body(world);
    assert!(body.get(&key).is_none(), "{key} must be absent: {body}");
}

// ─── UDS end to end ─────────────────────────────────────────────────────────

#[given(expr = "a models registry with reasoning Fireworks model {string}")]
fn given_reasoning_fireworks_model(world: &mut QuectoWorld, model_id: String) {
    super::uds_steps::given_models_registry_with_fireworks_model(world, model_id.clone());
    let base = world
        .cli_context
        .base_dir
        .clone()
        .expect("temp base directory not set");
    let path = base.join("models.json");
    let mut registry: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).expect("read models.json"))
            .expect("parse models.json");
    registry["providers"]["fireworks"]["models"][0]["reasoning"] = serde_json::json!(true);
    std::fs::write(&path, serde_json::to_string_pretty(&registry).unwrap())
        .expect("write models.json");
}

#[then(expr = "the get_state response effort levels should be {string}")]
fn then_get_state_effort_levels(world: &mut QuectoWorld, expected_csv: String) {
    let resp = find_agent_response(world, "get_state").expect("no get_state response");
    let levels: Vec<&str> = resp["data"]["effortLevels"]
        .as_array()
        .expect("effortLevels array")
        .iter()
        .map(|v| v.as_str().unwrap())
        .collect();
    let expected: Vec<&str> = expected_csv
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect();
    assert_eq!(levels, expected, "data: {}", resp["data"]);
}

#[then(expr = "the set_effort response should fail mentioning {string}")]
fn then_set_effort_fails_mentioning(world: &mut QuectoWorld, needle: String) {
    let found = world.agent_events.iter().any(|l| {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(l) else {
            return false;
        };
        v["type"] == "response"
            && v["command"] == "set_effort"
            && v["success"] == false
            && v["error"].as_str().unwrap_or("").contains(&needle)
    });
    assert!(
        found,
        "expected a failed set_effort response mentioning {needle:?}\nlines: {:#?}",
        world.agent_events
    );
}

fn fireworks_chat_bodies(world: &QuectoWorld) -> Vec<serde_json::Value> {
    let server = world
        .fireworks_mock_server_ref
        .expect("fireworks mock server not configured");
    let rt = tokio::runtime::Runtime::new().expect("runtime");
    let requests = rt
        .block_on(server.received_requests())
        .expect("received requests should be available");
    std::mem::forget(rt);
    requests
        .iter()
        .filter(|request| {
            request.method.as_str() == "POST" && request.url.path() == "/chat/completions"
        })
        .map(|request| serde_json::from_slice(&request.body).expect("chat body is JSON"))
        .collect()
}

#[then(
    expr = "the Fireworks provider should have received a chat completion request with reasoning_effort {string}"
)]
fn then_fireworks_received_effort(world: &mut QuectoWorld, expected: String) {
    let bodies = fireworks_chat_bodies(world);
    assert!(
        bodies.iter().any(|b| b["reasoning_effort"] == expected),
        "no chat completion carried reasoning_effort {expected:?}: {bodies:#?}\nevents: {:#?}",
        world.agent_events
    );
}

#[then(
    expr = "the Fireworks provider should have received a chat completion request without {string}"
)]
fn then_fireworks_received_without(world: &mut QuectoWorld, key: String) {
    let bodies = fireworks_chat_bodies(world);
    assert!(
        !bodies.is_empty(),
        "no chat completion received\nevents: {:#?}",
        world.agent_events
    );
    assert!(
        bodies.iter().all(|b| b.get(&key).is_none()),
        "{key} must be absent from every request: {bodies:#?}"
    );
}

#[then(
    expr = "the Fireworks provider's chat completion requests should carry reasoning_effort {string} in order"
)]
fn then_fireworks_effort_sequence(world: &mut QuectoWorld, expected_csv: String) {
    let bodies = fireworks_chat_bodies(world);
    let got: Vec<String> = bodies
        .iter()
        .map(|b| {
            b["reasoning_effort"]
                .as_str()
                .unwrap_or("<none>")
                .to_string()
        })
        .collect();
    let expected: Vec<&str> = expected_csv.split(',').map(str::trim).collect();
    assert_eq!(
        got, expected,
        "bodies: {bodies:#?}\nevents: {:#?}",
        world.agent_events
    );
}

#[given(expr = "the config default effort is {string}")]
fn given_config_default_effort(world: &mut QuectoWorld, effort: String) {
    let base = world.cli_context.base_dir.clone().expect("no base dir");
    let config_path = base.join("config.json");
    let mut config: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&config_path).expect("read config"))
            .expect("parse config");
    config["agents"]["defaults"]["effort"] = serde_json::json!(effort);
    std::fs::write(
        &config_path,
        serde_json::to_string_pretty(&config).expect("serialize config"),
    )
    .expect("write config");
}
