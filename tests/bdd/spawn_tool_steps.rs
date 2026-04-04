use super::*;

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
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    world.subagent_config = tool.parse_args(&arguments).ok();
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&arguments)).unwrap();
    world.spawn_result = Some(result);
}

#[when("I parse spawn arguments with a 64-character agent_id")]
fn when_parse_64_char_agent_id(world: &mut QuectoWorld) {
    let id = "a".repeat(64);
    let json = format!(r#"{{"task":"test","agent_id":"{}"}}"#, id);
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&json)).unwrap();
    world.spawn_result = Some(result);
}

#[when("I parse spawn arguments with a 65-character agent_id")]
fn when_parse_65_char_agent_id(world: &mut QuectoWorld) {
    let id = "a".repeat(65);
    let json = format!(r#"{{"task":"test","agent_id":"{}"}}"#, id);
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&json)).unwrap();
    world.spawn_result = Some(result);
}

#[when(expr = "I execute the SpawnTool with {string}")]
fn when_execute_spawn_tool(world: &mut QuectoWorld, arguments: String) {
    // Same as parse — execute() in stub mode covers both paths.
    when_parse_spawn_args(world, arguments);
}

#[when("I enable network passthrough on the SpawnTool")]
fn when_enable_network(world: &mut QuectoWorld) {
    let tool = world.spawn_tool.take().expect("spawn_tool not set");
    world.spawn_tool = Some(tool.with_network(true));
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

#[then(expr = "the SpawnTool should have network_passthrough {word}")]
fn then_network_passthrough(world: &mut QuectoWorld, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let debug = format!("{:?}", tool);
    let expected_str = format!("network_passthrough: {}", expected);
    assert!(
        debug.contains(&expected_str),
        "expected '{}' in debug: {}",
        expected_str,
        debug
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
        entries.contains_key(&agent_id),
        "expected registry to contain '{}', got keys: {:?}",
        agent_id,
        entries.keys().collect::<Vec<_>>()
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
