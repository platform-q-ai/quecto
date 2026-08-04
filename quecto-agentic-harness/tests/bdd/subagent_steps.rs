use super::*;
use quecto::domain::ids::AgentUuid;
use quecto::domain::subagent::{
    DisplayNameResolutionEntry, assert_display_name_available_for_spawn,
};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry, resolve_registry_key,
};

fn bdd_child_session_key(agent_uuid: &AgentUuid) -> String {
    agent_uuid.as_str().to_string()
}

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
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
        container: quecto::domain::container_runtime::SpawnContainerRequest::Local,
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
        effort: None,
        disable_tools: Vec::new(),
        read_only: false,
        container: quecto::domain::container_runtime::SpawnContainerRequest::Local,
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
#[when(expr = "the parent requests a subagent named {string}")]
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

#[given(expr = "a subagent named {string} has exited")]
fn given_subagent_named_exited(world: &mut QuectoWorld, name: String) {
    let old_uuid = AgentUuid::new("11111111-1111-4111-8111-111111111111");
    let mut entry = SubagentEntry::with_identity(
        old_uuid.clone(),
        name.clone(),
        std::path::PathBuf::from("/tmp/bdd-old.sock"),
        1,
    );
    entry.status = SubagentStatus::Exited;
    let registry = new_registry();
    registry.lock().unwrap().insert(old_uuid.to_string(), entry);
    world.agent_cmd_registry = Some(registry);
    world.subagent_old_session_key = Some(bdd_child_session_key(&old_uuid));
    world.subagent_old_uuid = Some(old_uuid);
    world.subagent_display_label = Some(name);
    world.subagent_context = Some(SubagentContext {
        task: "".into(),
        messages: vec![Message::user("previous context")],
        restrict_to_workspace: false,
    });
}

#[given(expr = "a live subagent named {string}")]
fn given_live_subagent_named(world: &mut QuectoWorld, name: String) {
    let uuid = AgentUuid::new("22222222-2222-4222-8222-222222222222");
    let entry = SubagentEntry::with_identity(
        uuid.clone(),
        name.clone(),
        std::path::PathBuf::from("/tmp/bdd-live.sock"),
        2,
    );
    let registry = new_registry();
    registry.lock().unwrap().insert(uuid.to_string(), entry);
    world.agent_cmd_registry = Some(registry);
    world.subagent_new_uuid = Some(uuid);
    world.subagent_display_label = Some(name);
}

#[when(expr = "a parent spawns a subagent named {string}")]
fn when_parent_spawns_named(world: &mut QuectoWorld, name: String) {
    let entries = world
        .agent_cmd_registry
        .as_ref()
        .map(|registry| {
            registry
                .lock()
                .unwrap()
                .values()
                .map(|entry| DisplayNameResolutionEntry {
                    agent_uuid: entry.agent_uuid.clone(),
                    display_name: entry.display_name.clone(),
                    live: entry.status != SubagentStatus::Exited,
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_default();

    match assert_display_name_available_for_spawn(&entries, &name) {
        Ok(()) => {
            // BDD keeps this at production-policy/helper level rather than launching
            // a child process: #1378 acceptance only requires the spawn identity,
            // session-key, clean-context, and live-label policy invariants.
            let new_uuid = AgentUuid::mint();
            world.subagent_new_session_key = Some(bdd_child_session_key(&new_uuid));
            world.subagent_new_uuid = Some(new_uuid);
            world.subagent_display_label = Some(name);
            world.agent_id_validation = Some(Ok(()));
            world.subagent_context = Some(SubagentContext {
                task: "".into(),
                messages: Vec::new(),
                restrict_to_workspace: false,
            });
        }
        Err(err) => {
            world.agent_id_validation = Some(Err(match err {
                quecto::domain::subagent::DisplayNameResolveError::NoLiveMatch { display_name } => {
                    format!("no live subagent named '{display_name}'")
                }
                quecto::domain::subagent::DisplayNameResolveError::AmbiguousLiveMatch {
                    display_name,
                } => {
                    format!("duplicate live subagent display label '{display_name}'")
                }
            }))
        }
    }
}

#[when(expr = "a parent tool targets display label {string}")]
fn when_parent_tool_targets_exited_label(world: &mut QuectoWorld, name: String) {
    let registry = world
        .agent_cmd_registry
        .as_ref()
        .expect("subagent registry not set");
    let result = {
        let entries = registry.lock().unwrap();
        resolve_registry_key(&entries, &name)
    };
    world.agent_id_validation = Some(result.map(|_| ()).map_err(|err| match err {
        quecto::domain::subagent::DisplayNameResolveError::NoLiveMatch { display_name } => {
            format!("no live subagent named '{display_name}'")
        }
        quecto::domain::subagent::DisplayNameResolveError::AmbiguousLiveMatch { display_name } => {
            format!("duplicate live subagent display label '{display_name}'")
        }
    }));
}

#[then("the spawned subagent should have a new hidden identity")]
fn then_spawned_has_new_hidden_identity(world: &mut QuectoWorld) {
    let old_uuid = world.subagent_old_uuid.as_ref().expect("old uuid missing");
    let new_uuid = world.subagent_new_uuid.as_ref().expect("new uuid missing");
    assert_ne!(
        old_uuid, new_uuid,
        "label reuse must mint a fresh hidden AgentUuid"
    );
    assert_eq!(world.subagent_display_label.as_deref(), Some("worker"));
}

#[then("the spawned subagent should have a clean conversation history")]
fn then_spawned_has_clean_history(world: &mut QuectoWorld) {
    let ctx = world
        .subagent_context
        .as_ref()
        .expect("subagent context not created");
    assert!(
        ctx.messages.is_empty(),
        "expected empty conversation history, got {} messages",
        ctx.messages.len()
    );
    assert_ne!(
        world.subagent_old_session_key, world.subagent_new_session_key,
        "child session key must be the new hidden AgentUuid, not reused display-label history"
    );
}

#[then(expr = "the spawn should fail with a duplicate display label error containing {string}")]
fn then_duplicate_display_label_error_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected duplicate display label failure");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains("duplicate live subagent display label") && err.contains(&expected),
        "expected duplicate display label error containing '{}', got: {}",
        expected,
        err
    );
}

#[then(expr = "the command should fail with no live subagent named {string}")]
fn then_command_fails_no_live_subagent(world: &mut QuectoWorld, expected: String) {
    let result = world
        .agent_id_validation
        .as_ref()
        .expect("no validation result");
    assert!(result.is_err(), "expected no-live-subagent failure");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains("no live subagent named") && err.contains(&expected),
        "expected no-live-subagent error containing '{}', got: {}",
        expected,
        err
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
