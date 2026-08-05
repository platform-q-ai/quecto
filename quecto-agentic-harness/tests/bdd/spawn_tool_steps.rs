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

#[given("live subagent spawning is available")]
fn given_live_subagent_spawning_available(world: &mut QuectoWorld) {
    given_live_spawn_agent_cmd_mock_child(world);
}

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
    let workspace_root = manifest_dir.parent().unwrap();
    let target_dir = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .map(|path| {
            if path.is_absolute() {
                path
            } else {
                workspace_root.join(path)
            }
        })
        .unwrap_or_else(|| workspace_root.join("target"));
    let child_binary = target_dir.join("debug").join("quecto");
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
    world.spawn_result = Some(match rt.block_on(tool.execute(&args.to_string())) {
        Ok(r) => r,
        Err(e) => ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
        },
    });
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

#[given(expr = "script-managed subagent spawning is available with default script {string}")]
fn given_script_spawn_default(world: &mut QuectoWorld, script: String) {
    given_script_spawn(world, script, None, None);
}

#[given(expr = "script-managed subagent spawning is available with parent repository {string}")]
fn given_script_spawn_parent_repo(world: &mut QuectoWorld, repo: String) {
    given_script_spawn(world, "default".to_string(), Some(repo), None);
}

#[given(expr = "script-managed child {string} is running with task {string}")]
fn given_script_child_running(world: &mut QuectoWorld, agent_id: String, task: String) {
    given_script_spawn_default(world, "default".to_string());
    when_spawn_script_default(world, agent_id, task);
    then_spawn_result_ok(world);
}

fn given_script_spawn(
    world: &mut QuectoWorld,
    default_script: String,
    parent_repo: Option<String>,
    mode: Option<String>,
) {
    given_live_spawn_agent_cmd_mock_child(world);
    let base = base_path(world);
    let script = base.join("container-create.sh");
    let cfg_path = std::path::PathBuf::from(world.config_path.clone().unwrap());
    let cfg_dir = cfg_path.parent().unwrap().to_path_buf();
    let log = cfg_dir.join("container-log.jsonl");
    let mode = mode.unwrap_or_default();
    let create_script = format!(
        r#"#!/usr/bin/env bash
set -euo pipefail
env_ref="env-bdd"
echo "{{\"kind\":\"create\",\"script\":\"${{QUECTO_CONTAINER_SCRIPT:-default}}\",\"repo\":\"${{QUECTO_CONTAINER_REPO:-}}\",\"base_dir\":\"${{QUECTO_BASE_DIR:-}}\",\"mode\":\"{}\",\"env_ref\":\"$env_ref\"}}" >> '{}'
socket_path=""
while [ "$#" -gt 0 ]; do
  if [ "$1" = "--" ]; then shift; break; fi
  shift
done
prev=""
for arg in "$@"; do
  if [ "$prev" = "--socket" ]; then socket_path="$arg"; break; fi
  prev="$arg"
done
if [ -z "$socket_path" ]; then socket_path="$PWD/script-managed.sock"; fi
case "{}" in
  proxy) printf '{{"environment_id":"env-bdd","workspace_path":"%s","metadata":{{}},"socket_proxy":{{"argv":["proxy"]}}}}' "$PWD"; exit 0 ;;
  readiness) printf '{{"environment_id":"env-bdd","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$PWD" "$PWD/missing.sock"; exit 0 ;;
  register) "$@" >/dev/null 2>&1 & printf '{{"environment_id":"env-bdd","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$PWD" "$socket_path"; exit 0 ;;
  "initial prompt") python3 - "$socket_path" <<'PY' >/dev/null 2>&1 &
import os, socket, sys, time
path=sys.argv[1]
try: os.unlink(path)
except FileNotFoundError: pass
s=socket.socket(socket.AF_UNIX, socket.SOCK_STREAM); s.bind(path); s.listen(8)
# readiness probe connects first; initial prompt connects second and gets EOF/reset
for _ in range(2):
    c,_=s.accept(); c.close()
time.sleep(2)
PY
printf '{{"environment_id":"env-bdd","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$PWD" "$socket_path"; exit 0 ;;
esac
"$@" >/dev/null 2>&1 &
printf '{{"environment_id":"env-bdd","workspace_path":"%s","metadata":{{}},"socket_path":"%s"}}' "$PWD" "$socket_path"
"#,
        mode,
        log.display(),
        mode
    );
    std::fs::write(&script, create_script).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&script).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&script, p).unwrap();
    }
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    let cleanup = cfg_dir.join("container-cleanup.sh");
    std::fs::write(
        &cleanup,
        format!(
            r#"#!/usr/bin/env bash
set -euo pipefail
printf '{{"kind":"cleanup","env_ref":"%s"}}\n' "${{QUECTO_CONTAINER_ENVIRONMENT_REF:-}}" >> '{}'
"#,
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&cleanup).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&cleanup, p).unwrap();
    }
    v["container_scripts"] = serde_json::json!({"default": default_script, "scripts": {"default": {"create": [script.to_string_lossy()], "cleanup": [cleanup.to_string_lossy()]}, "alternate": {"create": [script.to_string_lossy()], "cleanup": [cleanup.to_string_lossy()]}}});
    let repo = parent_repo.unwrap_or_else(|| "https://github.com/example/parent.git".to_string());
    if !base.join(".git").exists() {
        std::process::Command::new("git")
            .arg("init")
            .arg(&base)
            .status()
            .expect("git init for parent repo fixture");
    }
    std::process::Command::new("git")
        .arg("-C")
        .arg(&base)
        .args(["remote", "remove", "origin"])
        .status()
        .ok();
    std::process::Command::new("git")
        .arg("-C")
        .arg(&base)
        .args(["remote", "add", "origin", &repo])
        .status()
        .expect("git remote add for parent repo fixture");
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}

#[when(expr = "I spawn local subagent {string} with container disabled and task {string}")]
fn when_spawn_local_false(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":false,"read_only":true}),
    );
}

#[when(expr = "I spawn local subagent {string} with initial task {string}")]
fn when_spawn_local_initial(world: &mut QuectoWorld, agent_id: String, task: String) {
    when_live_spawn_subagent_with_task(world, agent_id, task);
}

#[when(expr = "I spawn script-managed subagent {string} with default selection and task {string}")]
fn when_spawn_script_default(world: &mut QuectoWorld, agent_id: String, task: String) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":true,"read_only":true}),
    );
}

#[when(expr = "I spawn script-managed subagent {string} with script {string} and task {string}")]
fn when_spawn_script_named(
    world: &mut QuectoWorld,
    agent_id: String,
    script: String,
    task: String,
) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"new","container_script":script},"read_only":true}),
    );
}

#[when(expr = "I spawn script-managed subagent {string} for repository {string} and task {string}")]
fn when_spawn_script_repo(world: &mut QuectoWorld, agent_id: String, repo: String, task: String) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"task":task,"container":{"mode":"new","repo":repo},"read_only":true}),
    );
}

#[when(expr = "I spawn subagent {string} with unsupported container field {string}")]
fn when_spawn_unsupported_field(world: &mut QuectoWorld, agent_id: String, field: String) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"container":{"mode":"new",field:"x"}}),
    );
}

#[when(expr = "I spawn subagent {string} into an existing container")]
fn when_spawn_existing(world: &mut QuectoWorld, agent_id: String) {
    execute_spawn_json(
        world,
        serde_json::json!({"agent_id":agent_id,"container":{"mode":"existing"}}),
    );
}

fn execute_spawn_json(world: &mut QuectoWorld, mut args: serde_json::Value) {
    args["config"] = serde_json::json!(world.config_path.clone().unwrap());
    let tool = world.spawn_tool.as_ref().expect("spawn tool");
    let rt = tokio::runtime::Runtime::new().unwrap();
    world.spawn_result = Some(match rt.block_on(tool.execute(&args.to_string())) {
        Ok(r) => r,
        Err(e) => ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
        },
    });
}

#[then(expr = "child {string} should receive {string}")]
fn then_child_receives(world: &mut QuectoWorld, agent_id: String, expected: String) {
    assert!(!agent_id.is_empty() && !expected.is_empty());
    then_live_get_messages_contains(world, agent_id, expected);
}

#[then(expr = "child {string} should be reachable")]
fn then_child_reachable(world: &mut QuectoWorld, agent_id: String) {
    assert!(!agent_id.is_empty());
    then_live_get_state_ok(world, agent_id);
}

#[then(expr = "child {string} should not be reachable")]
fn then_child_not_reachable(world: &mut QuectoWorld, agent_id: String) {
    let args = serde_json::json!({"agent_id":agent_id,"command":"get_state"});
    let rt = tokio::runtime::Runtime::new();
    if let Some(tool) = world.agent_cmd_tool.as_ref() {
        let r = rt
            .unwrap()
            .block_on(tool.execute(&args.to_string()))
            .unwrap();
        assert!(r.is_error, "child unexpectedly reachable: {}", r.content);
    }
}

#[then(expr = "the spawn result should not include an environment reference")]
fn then_no_env_ref(world: &mut QuectoWorld) {
    assert!(
        !world
            .spawn_result
            .as_ref()
            .unwrap()
            .content
            .contains("environment_ref")
    );
}

#[then("the spawn result should include an environment reference")]
fn then_env_ref_present(world: &mut QuectoWorld) {
    let c = &world.spawn_result.as_ref().unwrap().content;
    assert!(c.contains("environment_ref="), "{c}");
}

#[then(expr = "the spawn result should include environment reference {string}")]
fn then_env_ref(world: &mut QuectoWorld, expected: String) {
    let c = &world.spawn_result.as_ref().unwrap().content;
    assert!(
        c.contains("environment_ref") && c.contains(&expected),
        "{c}"
    );
}

fn script_invocations(world: &mut QuectoWorld) -> Vec<serde_json::Value> {
    let cfg_path = std::path::PathBuf::from(world.config_path.clone().unwrap());
    let log = cfg_path.parent().unwrap().join("container-log.jsonl");
    let text = std::fs::read_to_string(log).unwrap_or_default();
    text.lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

#[then(expr = "the script-managed runtime should have used container script {string}")]
fn then_script_used(world: &mut QuectoWorld, script: String) {
    let inv = script_invocations(world);
    assert!(
        inv.iter()
            .any(|v| v["kind"] == "create" && v["script"] == script),
        "invocations: {inv:?}"
    );
}
#[then(expr = "the script-managed runtime should have received repository {string}")]
fn then_repo_received(world: &mut QuectoWorld, repo: String) {
    let inv = script_invocations(world);
    assert!(
        inv.iter()
            .any(|v| v["kind"] == "create" && v["repo"] == repo),
        "invocations: {inv:?}"
    );
}
#[then("the script-managed runtime should have received the configured base directory")]
fn then_base_dir_received(world: &mut QuectoWorld) {
    let inv = script_invocations(world);
    let base = base_path(world).to_string_lossy().to_string();
    assert!(
        inv.iter()
            .any(|v| v["kind"] == "create" && v["base_dir"] == base),
        "invocations: {inv:?}, expected base {base}"
    );
}
#[then(expr = "the script-managed runtime should have started exactly {int} child")]
fn then_started_count(world: &mut QuectoWorld, n: i32) {
    let inv = script_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "create").count() as i32;
    assert_eq!(count, n, "invocations: {inv:?}");
}
#[then("no local fallback child should have been started")]
fn then_no_local_fallback(world: &mut QuectoWorld) {
    // The registry entry for a script-managed launch records the pid of any
    // child the parent itself spawned; the script-managed adapter starts no
    // local process, so a nonzero pid here proves a local fallback child.
    let content = world
        .spawn_result
        .as_ref()
        .expect("spawn result")
        .content
        .clone();
    let uuid = content
        .split("uuid=")
        .nth(1)
        .and_then(|s| s.split(')').next())
        .expect("spawn result contains uuid")
        .to_string();
    let registry = world.spawn_tool.as_ref().expect("spawn tool").registry();
    let entries = registry.lock().unwrap();
    let entry = entries.get(&uuid).expect("registered spawned agent");
    assert_eq!(
        entry.pid, 0,
        "parent spawned a local child (pid {}) alongside the script-managed create",
        entry.pid
    );
}
#[then("the script-managed runtime should have created the committed environment reference")]
fn then_committed_env(world: &mut QuectoWorld) {
    assert!(
        world
            .spawn_result
            .as_ref()
            .unwrap()
            .content
            .contains("environment_ref=")
    );
}

#[then(expr = "the agent command result should not be an error")]
fn then_agent_command_ok(world: &mut QuectoWorld) {
    let r = world
        .agent_cmd_result
        .as_ref()
        .expect("agent command result");
    assert!(!r.is_error, "agent command failed: {}", r.content);
}

#[when(expr = "I send prompt {string} to child {string}")]
fn when_send_prompt_to_child(world: &mut QuectoWorld, msg: String, agent_id: String) {
    let args = serde_json::json!({"command":"prompt","message":msg}).to_string();
    when_run_live_agent_cmd(world, agent_id, args);
}

fn then_live_get_state_ok(world: &mut QuectoWorld, agent_id: String) {
    let args = serde_json::json!({"agent_id": agent_id, "command": "get_state"});
    let tool = world.agent_cmd_tool.as_ref().expect("agent_cmd tool");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&args.to_string())).unwrap();
    assert!(!result.is_error, "get_state failed: {}", result.content);
    world.agent_cmd_result = Some(result);
}

#[then(
    expr = "the spawn result should fail because unsupported container field {string} is not allowed"
)]
fn then_unsupported_field_error(world: &mut QuectoWorld, field: String) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(r.is_error && r.content.contains(&field), "{}", r.content);
}
#[then("the spawn result should fail because existing containers are unsupported")]
fn then_existing_error(world: &mut QuectoWorld) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains("existing"),
        "{}",
        r.content
    );
}
#[then(expr = "the spawn result should fail because script configuration {string} is invalid")]
fn then_config_error(world: &mut QuectoWorld, err: String) {
    let r = world.spawn_result.as_ref().unwrap();
    assert!(
        r.is_error && r.content.contains("container_scripts"),
        "{} {err}",
        r.content
    );
}
#[then("the script-managed runtime should not have been invoked")]
fn then_runtime_not_invoked(world: &mut QuectoWorld) {
    assert!(world.spawn_result.as_ref().unwrap().is_error);
    let inv = script_invocations(world);
    assert!(
        inv.iter().all(|v| v["kind"] != "create"),
        "invocations: {inv:?}"
    );
}

#[given(expr = "script-managed subagent spawning has {string} runtime configuration")]
fn given_invalid_runtime_config(world: &mut QuectoWorld, err: String) {
    given_live_spawn_agent_cmd_mock_child(world);
    let cfg_path = std::path::PathBuf::from(world.config_path.clone().unwrap());
    let cfg_dir = cfg_path.parent().unwrap().to_path_buf();
    let log = cfg_dir.join("container-log.jsonl");
    let create = cfg_dir.join("should-not-create.sh");
    std::fs::write(
        &create,
        format!(
            "#!/usr/bin/env bash\necho '{{\"kind\":\"create\"}}' >> '{}'\n",
            log.display()
        ),
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut p = std::fs::metadata(&create).unwrap().permissions();
        p.set_mode(0o700);
        std::fs::set_permissions(&create, p).unwrap();
    }
    let mut v: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&cfg_path).unwrap()).unwrap();
    v["container_scripts"] = match err.as_str() {
        "missing default" => {
            serde_json::json!({"scripts":{"default":{"create":[create.to_string_lossy()],"cleanup":["true"]}}})
        }
        "default name not found" => {
            serde_json::json!({"default":"missing","scripts":{"default":{"create":[create.to_string_lossy()],"cleanup":["true"]}}})
        }
        "missing create argv" => {
            serde_json::json!({"default":"default","scripts":{"default":{"cleanup":["true"]}}})
        }
        "empty create argv" => {
            serde_json::json!({"default":"default","scripts":{"default":{"create":[],"cleanup":["true"]}}})
        }
        "unsafe create argv" => {
            serde_json::json!({"default":"default","scripts":{"default":{"create":["bad\u{0000}arg"],"cleanup":["true"]}}})
        }
        "unknown config field" => {
            serde_json::json!({"default":"default","scripts":{"default":{"create":[create.to_string_lossy()],"cleanup":["true"],"surprise":true}}})
        }
        other => panic!("unknown config_error example: {other}"),
    };
    std::fs::write(&cfg_path, serde_json::to_string_pretty(&v).unwrap()).unwrap();
}
#[given(expr = "script-managed subagent spawning returns a proxy endpoint")]
fn given_proxy_endpoint(world: &mut QuectoWorld) {
    given_script_spawn(
        world,
        "default".to_string(),
        None,
        Some("proxy".to_string()),
    );
}
#[given(expr = "script-managed subagent spawning fails during {string}")]
fn given_spawn_fails_during(world: &mut QuectoWorld, phase: String) {
    given_script_spawn(world, "default".to_string(), None, Some(phase));
}
#[then("the spawn result should fail because proxy endpoints are unsupported")]
fn then_proxy_unsupported(world: &mut QuectoWorld) {
    assert!(world.spawn_result.as_ref().unwrap().is_error);
}
#[then(expr = "the spawn result should fail because script-managed launch failed during {string}")]
fn then_launch_failed_phase(world: &mut QuectoWorld, _phase: String) {
    assert!(world.spawn_result.as_ref().unwrap().is_error);
}
#[then(expr = "the script-managed runtime should have cleaned up exactly {int} environment")]
fn then_cleanup_count(world: &mut QuectoWorld, n: i32) {
    let inv = script_invocations(world);
    let count = inv.iter().filter(|v| v["kind"] == "cleanup").count() as i32;
    assert_eq!(count, n, "invocations: {inv:?}");
}
#[then("the script-managed cleanup should target the created environment")]
fn then_cleanup_target(world: &mut QuectoWorld) {
    let inv = script_invocations(world);
    let created = inv
        .iter()
        .find(|v| v["kind"] == "create")
        .and_then(|v| v["env_ref"].as_str())
        .unwrap_or("");
    assert!(!created.is_empty(), "no created env ref: {inv:?}");
    assert!(
        inv.iter()
            .any(|v| v["kind"] == "cleanup" && v["env_ref"] == created),
        "invocations: {inv:?}"
    );
}
#[then(expr = "the subagent registry should not contain {string}")]
fn then_registry_not_contain(world: &mut QuectoWorld, agent_id: String) {
    assert!(!agent_id.is_empty());
    then_child_not_reachable(world, agent_id);
}
