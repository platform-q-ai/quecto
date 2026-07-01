use super::*;

// Subagent Steps
// ===========================================================================

#[given(expr = "a subagent spawn request with task {string}")]
fn given_subagent_spawn_request(world: &mut QuectoWorld, task: String) {
    world.subagent_config = Some(SubagentConfig {
        task: Some(task),
        agent_id: None,
        restrict_to_workspace: false,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        disable_tools: Vec::new(),
    });
}

#[given(expr = "a parent agent config with restrict_to_workspace {word}")]
fn given_parent_config_restrict(world: &mut QuectoWorld, value: String) {
    let restrict = value == "true";
    world.subagent_config = Some(SubagentConfig {
        task: Some("test task".to_string()),
        agent_id: None,
        restrict_to_workspace: restrict,
        system: None,
        config_path: None,
        workflow: false,
        workflow_guards: false,
        workflow_spec: None,
        model: None,
        disable_tools: Vec::new(),
    });
}

#[given(expr = "an agent allowlist containing {string} and {string}")]
fn given_agent_allowlist(world: &mut QuectoWorld, agent1: String, agent2: String) {
    world.agent_allowlist = vec![agent1, agent2];
}

#[when("the subagent context is created")]
fn when_subagent_context_created(world: &mut QuectoWorld) {
    let config = world
        .subagent_config
        .as_ref()
        .expect("subagent config not set");
    world.subagent_context = Some(SubagentContext::from_config(config));
}

#[when("a subagent context is created from the parent")]
fn when_subagent_context_from_parent(world: &mut QuectoWorld) {
    let config = world
        .subagent_config
        .as_ref()
        .expect("subagent config not set");
    world.subagent_context = Some(SubagentContext::from_config(config));
}

#[when(expr = "I validate agent_id {string}")]
fn when_validate_agent_id(world: &mut QuectoWorld, agent_id: String) {
    let result = validate_agent_id(&agent_id, &world.agent_allowlist);
    world.agent_id_validation = Some(result.map_err(|e| e.to_string()));
}

#[then(expr = "the subagent context should have task {string}")]
fn then_subagent_has_task(world: &mut QuectoWorld, expected: String) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    assert_eq!(
        ctx.task, expected,
        "expected task '{}', got '{}'",
        expected, ctx.task
    );
}

#[then("the subagent context should have an empty conversation history")]
fn then_subagent_empty_history(world: &mut QuectoWorld) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    assert!(
        ctx.messages.is_empty(),
        "expected empty conversation history, got {} messages",
        ctx.messages.len()
    );
}

#[then(expr = "the subagent should also have restrict_to_workspace {word}")]
fn then_subagent_restrict(world: &mut QuectoWorld, expected: String) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    let expected_bool = expected == "true";
    assert_eq!(
        ctx.restrict_to_workspace, expected_bool,
        "expected restrict_to_workspace {}, got {}",
        expected_bool, ctx.restrict_to_workspace
    );
}

#[then("the validation should succeed")]
fn then_validation_succeeds(world: &mut QuectoWorld) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(
        result.is_ok(),
        "expected validation to succeed, got: {}",
        result.as_ref().unwrap_err()
    );
}

#[then(expr = "the validation should fail with {string}")]
fn then_validation_fails_with(world: &mut QuectoWorld, expected: String) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected validation to fail");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains(&expected),
        "expected error to contain '{}', got: {}",
        expected,
        err
    );
}

// ===========================================================================
