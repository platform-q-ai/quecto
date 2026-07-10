use super::*;

// ===========================================================================
// Codex Provider BDD Steps
// ===========================================================================

// --- Issue #192: Orphaned function_call/function_call_output repair ---

#[given("a message list with an assistant function_call \"call_orphan\" but no matching output")]
fn given_orphaned_function_call(world: &mut QuectoWorld) {
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![quecto::domain::message::ToolCall {
        id: "call_orphan".to_string(),
        name: "bash".into(),
        arguments: "{}".to_string(),
    }];
    world.context_messages = Some(vec![Message::user("go"), assistant_msg]);
}

#[given("a message list with a tool result for \"call_orphan\" but no matching function_call")]
fn given_orphaned_function_call_output(world: &mut QuectoWorld) {
    let tool_msg = Message::tool("call_orphan", "some result");
    world.context_messages = Some(vec![Message::user("go"), tool_msg]);
}

#[given("a message list with a matched function_call \"call_valid\" and its output")]
fn given_matched_pair(world: &mut QuectoWorld) {
    let mut assistant_msg = Message::assistant("", vec![]);
    assistant_msg.tool_calls = vec![quecto::domain::message::ToolCall {
        id: "call_valid".to_string(),
        name: "read".into(),
        arguments: r#"{"path":"foo.rs"}"#.to_string(),
    }];
    let tool_msg = Message::tool("call_valid", "file content");
    world.context_messages = Some(vec![Message::user("read it"), assistant_msg, tool_msg]);
}

#[given(
    "a message list with a matched pair \"call_good\" and an orphaned function_call \"call_bad\""
)]
fn given_mixed_valid_and_orphaned(world: &mut QuectoWorld) {
    let mut good_assistant = Message::assistant("", vec![]);
    good_assistant.tool_calls = vec![quecto::domain::message::ToolCall {
        id: "call_good".to_string(),
        name: "read".into(),
        arguments: "{}".to_string(),
    }];
    let good_tool = Message::tool("call_good", "result");
    let mut bad_assistant = Message::assistant("", vec![]);
    bad_assistant.tool_calls = vec![quecto::domain::message::ToolCall {
        id: "call_bad".to_string(),
        name: "bash".into(),
        arguments: "{}".to_string(),
    }];
    world.context_messages = Some(vec![
        Message::user("start"),
        good_assistant,
        good_tool,
        bad_assistant,
    ]);
}

#[when("I build the Codex input")]
fn when_build_codex_input(world: &mut QuectoWorld) {
    let messages = world
        .context_messages
        .as_ref()
        .expect("message list not set by Given step");
    let (_instructions, input) =
        quecto::infrastructure::providers::codex::CodexProvider::build_input_public(messages);
    world.env_overrides.insert(
        "_codex_input".to_string(),
        serde_json::to_string(&input).unwrap(),
    );
}

#[then(expr = "the input should not contain any item with call_id {string}")]
fn then_input_not_contain_call_id(world: &mut QuectoWorld, call_id: String) {
    let input_str = world
        .env_overrides
        .get("_codex_input")
        .expect("codex input not set");
    let input: serde_json::Value = serde_json::from_str(input_str).expect("invalid json");
    let arr = input.as_array().expect("input should be an array");
    let found = arr
        .iter()
        .any(|item| item.get("call_id").and_then(|v| v.as_str()) == Some(call_id.as_str()));
    assert!(
        !found,
        "expected input not to contain call_id '{}', but found it in: {:?}",
        call_id, arr
    );
}

#[then(expr = "the input should contain an item with call_id {string} of type {string}")]
fn then_input_contain_call_id_type(world: &mut QuectoWorld, call_id: String, item_type: String) {
    let input_str = world
        .env_overrides
        .get("_codex_input")
        .expect("codex input not set");
    let input: serde_json::Value = serde_json::from_str(input_str).expect("invalid json");
    let arr = input.as_array().expect("input should be an array");
    let found = arr.iter().any(|item| {
        item.get("call_id").and_then(|v| v.as_str()) == Some(call_id.as_str())
            && item.get("type").and_then(|v| v.as_str()) == Some(item_type.as_str())
    });
    assert!(
        found,
        "expected input to contain call_id '{}' of type '{}', got: {:?}",
        call_id, item_type, arr
    );
}

// --- Request body formation steps ---

#[given(expr = "a Codex request body for model {string} with tools")]
fn given_codex_request_body_with_tools(world: &mut QuectoWorld, model: String) {
    let messages = vec![Message::system("You are helpful."), Message::user("Run ls")];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let request = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: &model,
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let body = quecto::infrastructure::providers::codex::CodexProvider::build_request_body_public(
        &request,
    );
    world
        .env_overrides
        .insert("_codex_body".to_string(), body.to_string());
}

// --- Issue #1066: Given context / When action steps for effort scenarios ---

#[given(expr = "an OpenAI reasoning model {string} with function tools")]
fn given_openai_reasoning_model_with_tools(world: &mut QuectoWorld, model: String) {
    world
        .env_overrides
        .insert("_codex_model".to_string(), model);
}

#[given(expr = "a configured reasoning effort {string}")]
fn given_configured_reasoning_effort(world: &mut QuectoWorld, effort: String) {
    world
        .env_overrides
        .insert("_codex_effort".to_string(), effort);
}

#[given("no reasoning effort is configured")]
fn given_no_reasoning_effort_configured(world: &mut QuectoWorld) {
    world.env_overrides.remove("_codex_effort");
}

#[when("the provider builds the Responses request")]
fn when_provider_builds_responses_request(world: &mut QuectoWorld) {
    let model = world
        .env_overrides
        .get("_codex_model")
        .cloned()
        .expect("no model — add 'Given an OpenAI reasoning model ... with function tools'");
    // Issue #1066: the full OpenAI-documented effort scale must be
    // configurable; parse() rejecting a documented level is a failure.
    let effort = world.env_overrides.get("_codex_effort").map(|e| {
        quecto::domain::provider::EffortLevel::parse(e).unwrap_or_else(|| {
            panic!("effort level '{e}' must be a valid configurable level (#1066)")
        })
    });
    let messages = vec![Message::system("You are helpful."), Message::user("Run ls")];
    let tools = vec![ToolDefinition {
        name: "bash".into(),
        description: "Execute a command".into(),
        parameters_schema: r#"{"type":"object","properties":{"command":{"type":"string"}}}"#.into(),
    }];
    let request = quecto::domain::provider::ChatRequest {
        messages: &messages,
        tools: &tools,
        model: &model,
        max_tokens: 4096,
        temperature: 0.7,
        session_id: None,
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort,
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
        session_id: Some(&session_id),
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
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
        tool_choice: None,
        metadata: None,
        thinking_level: None,
        cancel_flag: None,
        effort: None,
    };
    let body = quecto::infrastructure::providers::codex::CodexProvider::build_request_body_public(
        &request,
    );
    world
        .env_overrides
        .insert("_codex_body".to_string(), body.to_string());
}

#[then(expr = "the request body should contain a sanitized {string} with prefix {string}")]
fn then_body_contains_sanitized_cache_key(world: &mut QuectoWorld, field: String, prefix: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let value = body[&field]
        .as_str()
        .unwrap_or_else(|| panic!("expected body[\"{}\"] to be a string", field));
    let expected_prefix = format!("{}:", prefix);
    assert!(
        value.starts_with(&expected_prefix),
        "expected body[\"{}\"] to start with '{}', got: {}",
        field,
        expected_prefix,
        value
    );
    // Digest part should be 8 hex characters
    let digest = &value[expected_prefix.len()..];
    assert_eq!(
        digest.len(),
        8,
        "expected 8-char hex digest after prefix, got '{}' in: {}",
        digest,
        value
    );
    assert!(
        digest.chars().all(|c| c.is_ascii_hexdigit()),
        "expected digest to be hex chars, got: {}",
        digest
    );
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

#[then(expr = "the request body should contain a {string} object without {string}")]
fn then_body_nested_object_without_key(world: &mut QuectoWorld, parent: String, key: String) {
    let body_str = world
        .env_overrides
        .get("_codex_body")
        .expect("codex body not set");
    let body: serde_json::Value = serde_json::from_str(body_str).expect("invalid json");
    let obj = body
        .get(&parent)
        .and_then(|v| v.as_object())
        .unwrap_or_else(|| panic!("expected body[\"{}\"] to be an object", parent));
    assert!(
        !obj.contains_key(&key),
        "expected body[\"{}\"] not to contain \"{}\" (server default must apply, #1066), got {:?}",
        parent,
        key,
        obj.get(&key)
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
data: {"type":"response.output_item.added","output_index":1,"item":{"type":"function_call","call_id":"call_abc","name":"bash","arguments":""}}
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
data: {"type":"response.output_item.added","output_index":2,"item":{"type":"function_call","call_id":"call_2","name":"bash","arguments":""}}
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
    world.router_response = Some(response);
}

#[then(expr = "the parsed response should have {int} tool call(s)")]
fn then_parsed_has_n_tool_calls(world: &mut QuectoWorld, count: usize) {
    let response = world.router_response.as_ref().expect("no parsed response");
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
    let response = world.router_response.as_ref().expect("no parsed response");
    assert_eq!(
        response.tool_calls[0].name, expected_name,
        "expected tool call name '{}', got '{}'",
        expected_name, response.tool_calls[0].name
    );
}

#[then(expr = "the tool call should have arguments containing {string}")]
fn then_tool_call_has_args(world: &mut QuectoWorld, expected_arg: String) {
    let response = world.router_response.as_ref().expect("no parsed response");
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
    let response = world.router_response.as_ref().expect("no parsed response");
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
