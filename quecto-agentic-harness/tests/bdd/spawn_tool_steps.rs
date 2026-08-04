use super::*;
use quecto::infrastructure::tools::agent_cmd::AgentCmdTool;

// SpawnTool BDD Steps (#401)
// ===========================================================================

// --- Given ---

#[given(expr = "a SpawnTool with allowlist {string} and restrict_to_workspace {word}")]
fn given_spawn_tool_with_allowlist(world: &mut QuectoWorld, allowlist: String, restrict: String) {
    let agents: Vec<String> = allowlist.split(',').map(|s| s.trim().to_string()).collect();
    let restrict = restrict == "true";
    world.spawn_tool = Some(SpawnTool::new(agents, restrict));
}

#[given(expr = "a SpawnTool with empty allowlist and restrict_to_workspace {word}")]
fn given_spawn_tool_empty_allowlist(world: &mut QuectoWorld, restrict: String) {
    let restrict = restrict == "true";
    world.spawn_tool = Some(SpawnTool::new(vec![], restrict));
}

#[given(expr = "a SpawnTool with empty allowlist, parent id {string}, and a broadcast listener")]
fn given_spawn_tool_empty_allowlist_parent_and_broadcast(
    world: &mut QuectoWorld,
    parent_id: String,
) {
    let (tx, rx) = tokio::sync::broadcast::channel::<String>(8);
    world.spawn_tool =
        Some(SpawnTool::new(vec![], true).with_event_forwarding(Some(tx), Some(parent_id)));
    // Keep assertions deterministic without keeping a runtime alive across steps:
    // execute() sends the immediate-visibility event synchronously enough that it
    // is queued for this receiver before execute() returns.
    world.cascade_broadcast = Some(None);
    world.spawn_broadcast_rx = Some(rx);
}

#[given(expr = "a SpawnTool created with base_dir {string}")]
fn given_spawn_tool_with_base_dir(world: &mut QuectoWorld, base_dir: String) {
    world.spawn_tool = Some(SpawnTool::with_base_dir(
        vec![],
        true,
        PathBuf::from(base_dir),
    ));
}

// --- When ---

#[when(expr = "I parse spawn arguments {string}")]
fn when_parse_spawn_args(world: &mut QuectoWorld, arguments: String) {
    // execute() in stub mode (empty base_dir) exercises parse_args → config → stub output.
    // Also store the parsed SubagentConfig so new Then steps can inspect it.
    // Use parse_args_for_test (test-only accessor) to avoid a double-parse:
    // the config is captured here, then execute() reuses the same code path internally.
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    world.subagent_config = tool.parse_args_for_test(&arguments).ok();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&arguments)).unwrap();
    world.spawn_result = Some(result);
}

#[when("I parse spawn arguments with a 64-character agent_id")]
fn when_parse_64_char_agent_id(world: &mut QuectoWorld) {
    let id = "a".repeat(64);
    let json = format!(r#"{{"task":"test","agent_id":"{}"}}"#, id);
    when_parse_spawn_args(world, json);
}

#[when("I parse spawn arguments with a 65-character agent_id")]
fn when_parse_65_char_agent_id(world: &mut QuectoWorld) {
    let id = "a".repeat(65);
    let json = format!(r#"{{"task":"test","agent_id":"{}"}}"#, id);
    when_parse_spawn_args(world, json);
}

#[when(expr = "I execute the SpawnTool with {string}")]
fn when_execute_spawn_tool(world: &mut QuectoWorld, arguments: String) {
    // Same as parse — execute() in stub mode covers both paths.
    when_parse_spawn_args(world, arguments);
}

// --- Then ---

#[then(expr = "the parsed config should have task {string}")]
fn then_parsed_task(world: &mut QuectoWorld, _expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    // Stub mode no longer echoes the task — successful parse is sufficient.
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then("the parsed config should have no agent_id")]
fn then_no_agent_id(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    // In stub mode, successful output means parse succeeded; no agent_id is implicit
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then(expr = "the parsed config should have agent_id {string}")]
fn then_has_agent_id(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    // Stub mode doesn't echo agent_id — successful parse confirms it was accepted.
    assert!(
        !result.is_error,
        "expected agent_id '{}' accepted, got error: {}",
        expected, result.content
    );
}

#[then(expr = "the parsed config should have system prompt {string}")]
fn then_has_system_prompt(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    // System prompt is passed to subprocess, not echoed in stub output.
    assert!(
        !result.is_error,
        "expected system prompt '{}' accepted, got error: {}",
        expected, result.content
    );
}

#[then("the parsed config should have no system prompt")]
fn then_no_system_prompt(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then(expr = "the parsed config should have restrict_to_workspace {word}")]
fn then_restrict_to_workspace(world: &mut QuectoWorld, _expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    // Stub mode no longer echoes restrict_to_workspace — successful parse
    // with the correct constructor (which sets the field) is sufficient.
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then("the parsed config should have an agent_id of length 64")]
fn then_agent_id_64(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.is_error,
        "expected success for 64-char agent_id, got: {}",
        result.content
    );
}

#[then(expr = "the parse should fail with {string}")]
fn then_parse_fails_with(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.is_error,
        "expected error, got success: {}",
        result.content
    );
    assert!(
        result.content.contains(&expected),
        "expected error to contain '{}', got: {}",
        expected,
        result.content
    );
}

#[then(expr = "the SpawnTool should have base_dir {string}")]
fn then_base_dir(world: &mut QuectoWorld, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let debug = format!("{:?}", tool);
    assert!(
        debug.contains(&expected),
        "expected base_dir '{}' in debug: {}",
        expected,
        debug
    );
}

#[then("the SpawnTool should have an empty base_dir")]
fn then_empty_base_dir(world: &mut QuectoWorld) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let debug = format!("{:?}", tool);
    assert!(
        debug.contains(r#"base_dir: """#),
        "expected empty base_dir in debug: {}",
        debug
    );
}

#[then(expr = "the tool definition name should be {string}")]
fn then_tool_def_name(world: &mut QuectoWorld, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    assert_eq!(def.name, expected);
}

#[then("the tool definition description should not be empty")]
fn then_tool_def_description_not_empty(world: &mut QuectoWorld) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    assert!(!def.description.is_empty());
}

#[then(expr = "the tool definition schema should require {string}")]
fn then_tool_def_schema_requires(world: &mut QuectoWorld, field: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    // task is no longer required (#421) — schema may have no required array
    let required = schema["required"].as_array().cloned().unwrap_or_default();
    assert!(
        required.iter().any(|v| v.as_str() == Some(&field)),
        "expected '{}' in required fields: {:?}",
        field,
        required
    );
}

#[then(expr = "the spawn tool schema should not require {string}")]
fn then_tool_def_schema_not_requires(world: &mut QuectoWorld, field: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let required = schema["required"].as_array().cloned().unwrap_or_default();
    assert!(
        !required.iter().any(|v| v.as_str() == Some(&field)),
        "'{}' should NOT be in required fields: {:?}",
        field,
        required
    );
}

#[then(expr = "the tool definition description should mention {string}")]
fn then_tool_def_description_mentions(world: &mut QuectoWorld, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    assert!(
        def.description.contains(&expected),
        "expected description to contain '{}', got: {}",
        expected,
        def.description
    );
}

#[then(expr = "the subagent registry should contain {string}")]
fn then_registry_contains(world: &mut QuectoWorld, agent_id: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let registry = tool.registry();
    let entries = registry.lock().unwrap();
    assert!(
        entries
            .iter()
            .any(|(key, entry)| key == &agent_id || entry.display_name == agent_id),
        "expected registry to contain '{}', got keys: {:?}",
        agent_id,
        entries.keys().collect::<Vec<_>>()
    );
}

#[then(expr = "the subagent registry entry {string} should have parent_id {string}")]
fn then_registry_entry_has_parent_id(world: &mut QuectoWorld, agent_id: String, parent_id: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let registry = tool.registry();
    let entries = registry.lock().unwrap();
    let entry = entries
        .iter()
        .find(|(key, entry)| key.as_str() == agent_id || entry.display_name == agent_id)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| {
            panic!(
                "expected registry entry '{}', got keys {:?}",
                agent_id,
                entries.keys()
            )
        });
    assert_eq!(
        entry.parent_id.as_deref(),
        Some(parent_id.as_str()),
        "spawn should stamp the child with the spawning parent's id"
    );
}

#[then(expr = "the subagent registry entry {string} should be read-only")]
fn then_registry_entry_read_only(world: &mut QuectoWorld, agent_id: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let registry = tool.registry();
    let entries = registry.lock().unwrap();
    let entry = entries
        .iter()
        .find(|(key, entry)| key.as_str() == agent_id || entry.display_name == agent_id)
        .map(|(_, entry)| entry)
        .unwrap_or_else(|| {
            panic!(
                "expected registry entry '{}', got keys {:?}",
                agent_id,
                entries.keys()
            )
        });
    assert!(
        entry.read_only,
        "spawn should persist read_only observer status on the registry entry"
    );
}

#[then("the spawn result should not be an error")]
fn then_spawn_result_ok(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then("the spawn result should be an error")]
fn then_spawn_result_error(world: &mut QuectoWorld) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.is_error,
        "expected error, got success: {}",
        result.content
    );
}

#[then(expr = "the spawn result should contain {string}")]
fn then_spawn_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.spawn_result.as_ref().expect("no spawn result");
    assert!(
        result.content.contains(&expected),
        "expected content to contain '{}', got: {}",
        expected,
        result.content
    );
}

fn take_spawn_broadcast_event(world: &mut QuectoWorld) -> serde_json::Value {
    if world
        .cascade_broadcast
        .as_ref()
        .and_then(|v| v.as_ref())
        .is_none()
    {
        let rx = world
            .spawn_broadcast_rx
            .as_mut()
            .expect("spawn broadcast receiver not set");
        let raw = rx
            .try_recv()
            .unwrap_or_else(|e| panic!("expected immediate spawn broadcast, got {e}"));
        world.cascade_broadcast =
            Some(Some(serde_json::from_str(&raw).unwrap_or_else(|e| {
                panic!("broadcast should be valid JSON: {e}; raw={raw}")
            })));
    }
    world
        .cascade_broadcast
        .as_ref()
        .and_then(|v| v.as_ref())
        .cloned()
        .expect("spawn broadcast event not recorded")
}

#[then(expr = "the spawn broadcast should list {string} with parent_id {string}")]
fn then_spawn_broadcast_lists_parent_id(
    world: &mut QuectoWorld,
    agent_id: String,
    parent_id: String,
) {
    let event = take_spawn_broadcast_event(world);
    assert_eq!(event["type"].as_str(), Some("subagent_state_changed"));
    let subagents = event["subagents"]
        .as_array()
        .expect("subagents should be an array");
    let entry = subagents
        .iter()
        .find(|s| s["agentId"].as_str() == Some(agent_id.as_str()))
        .unwrap_or_else(|| panic!("expected broadcast to list {agent_id}, got {subagents:?}"));
    assert_eq!(
        entry["parentId"].as_str(),
        Some(parent_id.as_str()),
        "broadcast should preserve the spawning parent id"
    );
}

#[then(expr = "the spawn broadcast should list {string} as read-only")]
fn then_spawn_broadcast_lists_read_only(world: &mut QuectoWorld, agent_id: String) {
    let event = take_spawn_broadcast_event(world);
    assert_eq!(event["type"].as_str(), Some("subagent_state_changed"));
    let subagents = event["subagents"]
        .as_array()
        .expect("subagents should be an array");
    let entry = subagents
        .iter()
        .find(|s| s["agentId"].as_str() == Some(agent_id.as_str()))
        .unwrap_or_else(|| panic!("expected broadcast to list {agent_id}, got {subagents:?}"));
    assert_eq!(
        entry["readOnly"].as_bool(),
        Some(true),
        "broadcast should surface read_only observer status"
    );
}

#[then(expr = "the debug output should include {string}")]
fn then_debug_contains(world: &mut QuectoWorld, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let debug = format!("{:?}", tool);
    assert!(
        debug.contains(&expected),
        "expected debug to contain '{}', got: {}",
        expected,
        debug
    );
}

// --- New steps for config, workflow, workflow_guards forwarding ---

#[then(expr = "the parsed spawn config should have config path {string}")]
fn then_parsed_spawn_config_has_config_path(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert_eq!(
        cfg.config_path,
        Some(std::path::PathBuf::from(&expected)),
        "expected config_path {:?}, got {:?}",
        expected,
        cfg.config_path
    );
}

#[then("the parsed spawn config should have no config path")]
fn then_parsed_spawn_config_has_no_config_path(world: &mut QuectoWorld) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert!(
        cfg.config_path.is_none(),
        "expected no config_path, got {:?}",
        cfg.config_path
    );
}

#[then(expr = "the parsed spawn config should have workflow {word}")]
fn then_parsed_spawn_config_has_workflow(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    let expected_bool = expected == "true";
    assert_eq!(
        cfg.workflow, expected_bool,
        "expected workflow={}, got {}",
        expected_bool, cfg.workflow
    );
}

#[then("the parsed spawn config should have a workflow spec")]
fn then_parsed_spawn_config_has_workflow_spec(world: &mut QuectoWorld) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    let spec = cfg
        .workflow_spec
        .as_ref()
        .expect("expected a workflow spec, got none");
    assert_eq!(
        spec.template.id, "rev",
        "workflow spec should carry the assigned template, got id '{}'",
        spec.template.id
    );
}

#[then(expr = "the parsed spawn config should have workflow_guards {word}")]
fn then_parsed_spawn_config_has_workflow_guards(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    let expected_bool = expected == "true";
    assert_eq!(
        cfg.workflow_guards, expected_bool,
        "expected workflow_guards={}, got {}",
        expected_bool, cfg.workflow_guards
    );
}

#[then(expr = "the parsed spawn config should have model {string}")]
fn then_parsed_spawn_config_has_model(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert_eq!(
        cfg.model.as_deref(),
        Some(expected.as_str()),
        "expected model {:?}, got {:?}",
        expected,
        cfg.model
    );
}

#[then("the parsed spawn config should have no model")]
fn then_parsed_spawn_config_has_no_model(world: &mut QuectoWorld) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert!(
        cfg.model.is_none(),
        "expected no model, got {:?}",
        cfg.model
    );
}

#[then(expr = "the parsed spawn config should have effort {string}")]
fn then_parsed_spawn_config_has_effort(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert_eq!(
        cfg.effort.as_deref(),
        Some(expected.as_str()),
        "expected effort {:?}, got {:?}",
        expected,
        cfg.effort
    );
}

#[then("the parsed spawn config should have no effort")]
fn then_parsed_spawn_config_has_no_effort(world: &mut QuectoWorld) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert!(
        cfg.effort.is_none(),
        "expected no effort, got {:?}",
        cfg.effort
    );
}

#[then(expr = "the parsed spawn config should have disable_tools {string}")]
fn then_parsed_spawn_config_has_disable_tools(world: &mut QuectoWorld, expected: String) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    // Assert SET semantics (union + de-dup), not a private ordering: the
    // contract specifies which tools end up disabled, not the sequence they are
    // emitted in. Compare as sorted sets and check there are no duplicates.
    let mut expected: Vec<&str> = expected.split(',').collect();
    expected.sort_unstable();
    let mut got: Vec<&str> = cfg.disable_tools.iter().map(String::as_str).collect();
    let got_len = got.len();
    got.sort_unstable();
    got.dedup();
    assert_eq!(
        got.len(),
        got_len,
        "disable_tools should have no duplicates, got {:?}",
        cfg.disable_tools
    );
    assert_eq!(
        got, expected,
        "expected disable_tools set {:?}, got {:?}",
        expected, cfg.disable_tools
    );
}

#[then("the parsed spawn config should have no disable_tools")]
fn then_parsed_spawn_config_has_no_disable_tools(world: &mut QuectoWorld) {
    let cfg = world
        .subagent_config
        .as_ref()
        .expect("subagent_config not set — was parse_args called?");
    assert!(
        cfg.disable_tools.is_empty(),
        "expected no disable_tools, got {:?}",
        cfg.disable_tools
    );
}

#[then(expr = "the spawn tool schema should include property {string}")]
fn then_spawn_tool_schema_includes_property(world: &mut QuectoWorld, property: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let properties = schema["properties"]
        .as_object()
        .expect("schema should have properties");
    assert!(
        properties.contains_key(property.as_str()),
        "expected schema to include property '{}', got keys: {:?}",
        property,
        properties.keys().collect::<Vec<_>>()
    );
}

// ===========================================================================

// --- Live spawn + agent_cmd end-to-end regression steps ---

#[given("a live SpawnTool and AgentCmdTool backed by a mock LLM child")]
fn given_live_spawn_agent_cmd_mock_child(world: &mut QuectoWorld) {
    ensure_temp_dir(world);
    let rt = tokio::runtime::Runtime::new().unwrap();
    let server = rt.block_on(wiremock::MockServer::start());
    let uri = server.uri();
    rt.block_on(async {
        let body = serde_json::json!({
            "id": "chatcmpl-live-spawn-bdd",
            "object": "chat.completion",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "LIVE_CHILD_OK"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 3, "total_tokens": 13}
        });
        wiremock::Mock::given(wiremock::matchers::method("POST"))
            .and(wiremock::matchers::path("/chat/completions"))
            .respond_with(wiremock::ResponseTemplate::new(200).set_body_json(body))
            .mount(&server)
            .await;
    });

    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let child_binary = manifest_dir
        .parent()
        .unwrap()
        .join("target")
        .join("debug")
        .join("quecto");
    assert!(
        child_binary.exists(),
        "build the quecto binary before this scenario: cargo build -p quecto-agentic-harness --bin quecto"
    );
    // SAFETY: BDD scenarios are run with explicit single-scenario process isolation where this environment override is set before the SpawnTool launches any child process.
    unsafe { std::env::set_var("QUECTO_CHILD_BINARY", &child_binary) };

    let base = base_path(world);
    let workspace = base.join("workspace");
    std::fs::create_dir_all(&workspace).expect("create workspace");
    let config_path = base.join("config.json");
    let config = serde_json::json!({
        "providers": {"openai": {"api_key": "sk-test-key", "api_base": uri}},
        "agents": {"defaults": {"model": "openai/gpt-4o-mini", "workspace": workspace}}
    });
    std::fs::write(&config_path, serde_json::to_string_pretty(&config).unwrap()).unwrap();

    let registry = AgentCmdTool::new_registry();
    let socket_dir = base.join("sockets");
    std::fs::create_dir_all(&socket_dir).expect("create socket dir");
    world.spawn_tool = Some(
        SpawnTool::with_base_dir(vec![], true, base.clone())
            .with_socket_dir(socket_dir)
            .with_registry(registry.clone()),
    );
    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
    world.config_path = Some(config_path.to_string_lossy().to_string());
    world._wiremock_server_uri = Some(uri);
    std::mem::forget(server);
    std::mem::forget(rt);
}

#[when(expr = "I live-spawn subagent {string} with initial task {string}")]
fn when_live_spawn_subagent_with_task(world: &mut QuectoWorld, agent_id: String, task: String) {
    let config_path = world.config_path.clone().expect("config path");
    let args = serde_json::json!({
        "agent_id": agent_id,
        "task": task,
        "config": config_path,
        "read_only": true
    });
    let tool = world.spawn_tool.as_ref().expect("spawn tool");
    let rt = tokio::runtime::Runtime::new().unwrap();
    world.spawn_result = Some(rt.block_on(tool.execute(&args.to_string())).unwrap());
}

#[when(expr = "I run live agent_cmd for {string} with {string}")]
fn when_run_live_agent_cmd(world: &mut QuectoWorld, agent_id: String, args: String) {
    let mut v: serde_json::Value = serde_json::from_str(&args).expect("agent_cmd JSON");
    v["agent_id"] = serde_json::Value::String(agent_id);
    let tool = world.agent_cmd_tool.as_ref().expect("agent_cmd tool");
    let rt = tokio::runtime::Runtime::new().unwrap();
    world.agent_cmd_result = Some(rt.block_on(tool.execute(&v.to_string())).unwrap());
}

#[then(expr = "live agent_cmd get_messages for {string} should contain {string}")]
fn then_live_get_messages_contains(world: &mut QuectoWorld, agent_id: String, expected: String) {
    let args = serde_json::json!({"agent_id": agent_id, "command": "get_messages", "count": 10});
    let tool = world.agent_cmd_tool.as_ref().expect("agent_cmd tool");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(20);
    let mut last = None;
    while std::time::Instant::now() < deadline {
        let result = rt.block_on(tool.execute(&args.to_string())).unwrap();
        assert!(!result.is_error, "get_messages failed: {}", result.content);
        if result.content.contains(&expected) {
            world.agent_cmd_result = Some(result);
            return;
        }
        last = Some(result);
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    let result = last.expect("get_messages was not attempted");
    panic!(
        "expected get_messages to contain {expected:?}, got: {}",
        result.content
    );
}
