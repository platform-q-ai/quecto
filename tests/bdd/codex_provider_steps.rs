use super::*;

// ===========================================================================
// Codex Provider BDD Steps
// ===========================================================================

// --- Request body formation steps ---

#[given(expr = "a Codex request body for model {string} with tools")]
fn given_codex_request_body_with_tools(world: &mut QuectoWorld, model: String) {
    let messages = vec![Message::system("You are helpful."), Message::user("Run ls")];
    let tools = vec![ToolDefinition {
        name: "exec".to_string(),
        description: "Execute a command".to_string(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#
            .to_string(),
    }];
    let request = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: &model,
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
    };
    let body = quecto::infrastructure::providers::codex::CodexProvider::build_request_body_public(
        &request,
    );
    world
        .env_overrides
        .insert("_codex_body".to_string(), body.to_string());
}

#[given(expr = "a Codex request body for model {string} with session ID {string}")]
fn given_codex_request_body_with_session_id(
    world: &mut QuectoWorld,
    model: String,
    session_id: String,
) {
    let messages = vec![Message::user("Hi")];
    let request = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 4096,
        temperature: 0.7,
        session_id: Some(session_id),
    };
    let body = quecto::infrastructure::providers::codex::CodexProvider::build_request_body_public(
        &request,
    );
    world
        .env_overrides
        .insert("_codex_body".to_string(), body.to_string());
}

#[given(expr = "a Codex request body for model {string} without a session ID")]
fn given_codex_request_body_without_session_id(world: &mut QuectoWorld, model: String) {
    let messages = vec![Message::user("Hi")];
    let request = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &[],
        model: &model,
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
    };
    let body = quecto::infrastructure::providers::codex::CodexProvider::build_request_body_public(
        &request,
    );
    world
        .env_overrides
        .insert("_codex_body".to_string(), body.to_string());
}

#[then(expr = "the request body should contain {string} set to {string}")]
fn then_body_contains_string_value(world: &mut QuectoWorld, key: String, expected: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(
        body[&key].as_str(),
        Some(expected.as_str()),
        "expected body[\"{}\"] to be \"{}\", got {:?}",
        key,
        expected,
        body[&key]
    );
}

#[then(expr = "the request body should contain {string} set to true")]
fn then_body_contains_bool_true(world: &mut QuectoWorld, key: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(
        body[&key].as_bool(),
        Some(true),
        "expected body[\"{}\"] to be true, got {:?}",
        key,
        body[&key]
    );
}

#[then(expr = "the request body should contain a {string} object with {string} set to {string}")]
fn then_body_contains_nested_string(
    world: &mut QuectoWorld,
    parent: String,
    key: String,
    expected: String,
) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert_eq!(
        body[&parent][&key].as_str(),
        Some(expected.as_str()),
        "expected body[\"{}\"][\"{}\"] to be \"{}\", got {:?}",
        parent,
        key,
        expected,
        body[&parent][&key]
    );
}

#[then(expr = "the request body should contain {string} with {string}")]
fn then_body_contains_array_value(world: &mut QuectoWorld, key: String, expected: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let arr = body[&key]
        .as_array()
        .unwrap_or_else(|| panic!("expected body[\"{}\"] to be an array", key));
    let found = arr.iter().any(|v| v.as_str() == Some(expected.as_str()));
    assert!(
        found,
        "expected body[\"{}\"] to contain \"{}\", got {:?}",
        key, expected, arr
    );
}

#[then(expr = "the request body should not contain {string}")]
fn then_body_not_contains_key(world: &mut QuectoWorld, key: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    assert!(
        body.get(&key).is_none(),
        "expected body not to contain \"{}\", but found {:?}",
        key,
        body[&key]
    );
}

#[then(expr = "each tool definition should have {string} set to false")]
fn then_tools_have_strict_false(world: &mut QuectoWorld, key: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let tools = body["tools"]
        .as_array()
        .expect("expected tools array in body");
    for (i, tool) in tools.iter().enumerate() {
        assert_eq!(
            tool[&key].as_bool(),
            Some(false),
            "tool[{}][\"{}\"] should be false, got {:?}",
            i,
            key,
            tool[&key]
        );
    }
}

// --- SSE parsing steps ---

#[given(
    "a Codex SSE stream with a reasoning item at output_index 0 and a function call at output_index 1"
)]
fn given_codex_sse_with_reasoning_then_tool(world: &mut QuectoWorld) {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_abc","name":"exec","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"command\""}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":":\"ls\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}
data: [DONE]
"#;
    world
        .env_overrides
        .insert("_codex_sse".to_string(), sse.to_string());
}

#[given(
    "a Codex SSE stream with a reasoning item at output_index 0 and function calls at output_index 1 and 2"
)]
fn given_codex_sse_with_reasoning_then_two_tools(world: &mut QuectoWorld) {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"reasoning"}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_1","name":"read","arguments":""}}
data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","call_id":"call_2","name":"exec","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"path\":\"main.rs\"}"}
data: {"type":"response.function_call_arguments.delta","output_index":2,"delta":"{\"command\":\"cargo build\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":10,"output_tokens":5}}}
data: [DONE]
"#;
    world
        .env_overrides
        .insert("_codex_sse".to_string(), sse.to_string());
}

#[given("a Codex SSE stream with function calls at output_index 0 and 1 without reasoning")]
fn given_codex_sse_tools_no_reasoning(world: &mut QuectoWorld) {
    let sse = r#"data: {"type":"response.output_item.added","output_index":0,"item":{"type":"function_call","call_id":"call_a","name":"read","arguments":""}}
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_b","name":"write","arguments":""}}
data: {"type":"response.function_call_arguments.delta","output_index":0,"delta":"{\"path\":\"file.txt\"}"}
data: {"type":"response.function_call_arguments.delta","output_index":1,"delta":"{\"content\":\"output\"}"}
data: {"type":"response.completed","response":{"usage":{"input_tokens":5,"output_tokens":3}}}
data: [DONE]
"#;
    world
        .env_overrides
        .insert("_codex_sse".to_string(), sse.to_string());
}

#[when("I parse the Codex SSE stream")]
fn when_parse_codex_sse(world: &mut QuectoWorld) {
    let sse = world
        .env_overrides
        .get("_codex_sse")
        .expect("SSE stream not set")
        .clone();
    let response =
        quecto::infrastructure::providers::codex::CodexProvider::parse_sse_response_public(&sse)
            .expect("SSE parse should succeed");
    world.fallback_response = Some(response);
}

#[then(expr = "the parsed response should have {int} tool call(s)")]
fn then_parsed_has_n_tool_calls(world: &mut QuectoWorld, count: usize) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no parsed response");
    assert_eq!(
        response.tool_calls.len(),
        count,
        "expected {} tool calls, got {}",
        count,
        response.tool_calls.len()
    );
}

#[then(expr = "the tool call should have name {string}")]
fn then_tool_call_has_name(world: &mut QuectoWorld, expected_name: String) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no parsed response");
    assert_eq!(
        response.tool_calls[0].name, expected_name,
        "expected tool call name '{}', got '{}'",
        expected_name, response.tool_calls[0].name
    );
}

#[then(expr = "the tool call should have arguments containing {string}")]
fn then_tool_call_has_args(world: &mut QuectoWorld, expected_arg: String) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no parsed response");
    assert!(
        response.tool_calls[0].arguments.contains(&expected_arg),
        "expected tool call arguments to contain '{}', got '{}'",
        expected_arg,
        response.tool_calls[0].arguments
    );
}

#[then(expr = "tool call {int} should have name {string} and arguments containing {string}")]
fn then_tool_call_n_has_name_and_args(
    world: &mut QuectoWorld,
    index: usize,
    expected_name: String,
    expected_arg: String,
) {
    let response = world
        .fallback_response
        .as_ref()
        .expect("no parsed response");
    assert!(
        response.tool_calls.len() > index,
        "expected at least {} tool calls, got {}",
        index + 1,
        response.tool_calls.len()
    );
    assert_eq!(
        response.tool_calls[index].name, expected_name,
        "tool call {} name: expected '{}', got '{}'",
        index, expected_name, response.tool_calls[index].name
    );
    assert!(
        response.tool_calls[index].arguments.contains(&expected_arg),
        "tool call {} arguments: expected to contain '{}', got '{}'",
        index,
        expected_arg,
        response.tool_calls[index].arguments
    );
}
