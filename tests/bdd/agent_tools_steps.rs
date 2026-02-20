use super::*;

// Agent Tools Steps
// ===========================================================================

#[given("a tool workspace")]
fn given_tool_workspace(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("failed to create temp dir");
    let ws = td.path().to_path_buf();
    let sandbox = Sandbox::new(Some(ws.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(ws.clone(), sandbox);
    world.tool_workspace = Some(ws);
    world.tool_registry = Some(registry);
    world._temp_dir = Some(td);
}

#[given(expr = "a file {string} exists with content {string}")]
fn given_file_exists(world: &mut QuectoWorld, filename: String, content: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).expect("create parent dirs");
    }
    std::fs::write(&path, &content).expect("write file");
}

#[when(expr = "the agent executes tool {string} with args:")]
fn when_agent_executes_tool(world: &mut QuectoWorld, tool_name: String, step: &gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    // Build JSON from table: first column is key, second is value
    let mut map = serde_json::Map::new();
    for row in &table.rows {
        if row.len() >= 2 {
            map.insert(
                row[0].trim().to_string(),
                serde_json::Value::String(row[1].trim().to_string()),
            );
        }
    }
    let args_json = serde_json::Value::Object(map).to_string();

    let registry = world.tool_registry.as_ref().expect("tool registry not set");

    // Run the tool using a tokio runtime
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(registry.execute(&tool_name, &args_json));

    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the tool result should contain {string}")]
fn then_tool_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            tr.content.contains(&expected),
            "expected tool result to contain '{}', got: {}",
            expected,
            tr.content
        ),
        Err(e) => panic!("tool returned error: {}", e),
    }
}

#[then("the tool result should not be an error")]
fn then_tool_result_not_error(world: &mut QuectoWorld) {
    let result = world.tool_result.as_ref().expect("no tool result");
    match result {
        Ok(tr) => assert!(
            !tr.is_error,
            "expected tool result to not be an error, content: {}",
            tr.content
        ),
        Err(e) => panic!("tool returned DomainError: {}", e),
    }
}

#[then(expr = "the file {string} should exist in the workspace")]
fn then_file_exists_in_workspace(world: &mut QuectoWorld, filename: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    assert!(
        path.exists(),
        "file '{}' should exist at {}",
        filename,
        path.display()
    );
}

#[then(expr = "the file {string} should contain {string}")]
fn then_file_contains(world: &mut QuectoWorld, filename: String, expected: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("tool workspace not set");
    let path = ws.join(&filename);
    let content = std::fs::read_to_string(&path)
        .unwrap_or_else(|_| panic!("failed to read {}", path.display()));
    assert!(
        content.contains(&expected),
        "expected '{}' to contain '{}', got: {}",
        filename,
        expected,
        content
    );
}

#[then(expr = "the tool registry should contain {string}")]
fn then_registry_contains(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let names = registry.names();
    assert!(
        names.contains(&tool_name),
        "registry should contain '{}', has: {:?}",
        tool_name,
        names
    );
}

// ===========================================================================
// Security (Subagent/Heartbeat Inheritance) Steps
// ===========================================================================

#[given("a subagent context inheriting restrict_to_workspace")]
fn given_subagent_inheriting_sandbox(world: &mut QuectoWorld) {
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Create a subagent config that inherits the sandbox's restrict_to_workspace
    world.subagent_config = Some(SubagentConfig {
        task: "test task".to_string(),
        agent_id: None,
        restrict_to_workspace: sb.restrict_to_workspace,
        deliver_to: None,
    });
    let ctx = SubagentContext::from_config(world.subagent_config.as_ref().unwrap());
    world.subagent_context = Some(ctx);
}

#[when(expr = "the subagent sandbox validates path {string}")]
fn when_subagent_validates_path(world: &mut QuectoWorld, path: String) {
    // The subagent inherits the same sandbox config; validate using it
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    // Verify the subagent context also has restrict_to_workspace set
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not set");
    assert_eq!(ctx.restrict_to_workspace, sb.restrict_to_workspace);
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}

#[when(expr = "a heartbeat sandbox validates path {string}")]
fn when_heartbeat_validates_path(world: &mut QuectoWorld, path: String) {
    // Heartbeat tasks run within the same sandbox restrictions
    let sb = world.sandbox.as_ref().expect("sandbox not configured");
    world.validation_result = Some(
        sb.validate_path(&path)
            .map(|_| ())
            .map_err(|e| e.to_string()),
    );
}
