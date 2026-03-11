use cucumber::{given, then, when};
use quecto::domain::workflow::{GuardRule, WorkflowConfig, WorkflowPersistable, WorkflowState};
use quecto::infrastructure::tools::workflow_tool::WorkflowTool;
use std::sync::{Arc, Mutex};

use super::QuectoWorld;

// ─── Domain: WorkflowConfig extensions ──────────────────────────────────────

#[then("the workflow config auto_continue should be true")]
fn then_auto_continue_true(world: &mut QuectoWorld) {
    let config = resolve_workflow_config(world);
    assert!(config.auto_continue);
}

#[then("the workflow config auto_continue should be false")]
fn then_auto_continue_false(world: &mut QuectoWorld) {
    let config = resolve_workflow_config(world);
    assert!(!config.auto_continue);
}

#[then("the workflow config completion_nudge should be true")]
fn then_completion_nudge_true(world: &mut QuectoWorld) {
    let config = resolve_workflow_config(world);
    assert!(config.completion_nudge);
}

#[then("the workflow config completion_nudge should be false")]
fn then_completion_nudge_false(world: &mut QuectoWorld) {
    let config = resolve_workflow_config(world);
    assert!(!config.completion_nudge);
}

#[then(expr = "the workflow config should have {int} guards")]
fn then_config_guards_count(world: &mut QuectoWorld, count: i32) {
    let config = resolve_workflow_config(world);
    assert_eq!(
        config.guards.len(),
        count as usize,
        "expected {} guards, got {}",
        count,
        config.guards.len()
    );
}

fn resolve_workflow_config(world: &QuectoWorld) -> WorkflowConfig {
    if let Some(ref config) = world.workflow_config {
        config.clone()
    } else if let Some(ref config) = world.config {
        config.workflow.clone()
    } else {
        panic!("no workflow config or loaded config available");
    }
}

#[given("a workflow config with auto_continue false")]
fn given_config_auto_continue_false(world: &mut QuectoWorld) {
    world.workflow_config = Some(WorkflowConfig {
        auto_continue: false,
        ..Default::default()
    });
}

#[given("a workflow config with completion_nudge false")]
fn given_config_completion_nudge_false(world: &mut QuectoWorld) {
    world.workflow_config = Some(WorkflowConfig {
        completion_nudge: false,
        ..Default::default()
    });
}

#[given("a workflow config with guards:")]
fn given_config_with_guards(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("need data table");
    let rules: Vec<GuardRule> = table
        .rows
        .iter()
        .skip(1)
        .map(|row| {
            let commands: Vec<String> = row[0].split(',').map(|s| s.trim().to_string()).collect();
            GuardRule {
                commands,
                before_step: row[1].trim().parse().unwrap(),
                message: row[2].trim().to_string(),
            }
        })
        .collect();
    world.workflow_config = Some(WorkflowConfig {
        guards: rules,
        ..Default::default()
    });
}

#[given("a config JSON with workflow auto_continue false and completion_nudge false")]
fn given_config_json_all_fields(world: &mut QuectoWorld) {
    world.workflow_config_json =
        Some(r#"{"auto_continue":false,"completion_nudge":false}"#.to_string());
}

#[given("an empty config JSON")]
fn given_empty_config_json(world: &mut QuectoWorld) {
    world.workflow_config_json = Some("{}".to_string());
}

#[when("I deserialize the workflow config")]
fn when_deserialize_config(world: &mut QuectoWorld) {
    let json = world
        .workflow_config_json
        .as_ref()
        .expect("config JSON should exist");
    let config: WorkflowConfig = serde_json::from_str(json).expect("should deserialize");
    world.workflow_config = Some(config);
}

// ─── Domain: Auto-continue nudge ────────────────────────────────────────────

#[when("I generate the auto_continue nudge")]
fn when_auto_continue_nudge(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    world.workflow_nudge = s.auto_continue_nudge();
}

#[then(expr = "the nudge should contain {string}")]
fn then_nudge_contains(world: &mut QuectoWorld, expected: String) {
    let nudge = world
        .workflow_nudge
        .as_ref()
        .expect("nudge should exist (got None)");
    assert!(
        nudge.contains(&expected),
        "expected nudge containing '{}', got: {}",
        expected,
        nudge
    );
}

#[then("the nudge should be None")]
fn then_nudge_none(world: &mut QuectoWorld) {
    assert!(
        world.workflow_nudge.is_none(),
        "expected None, got: {:?}",
        world.workflow_nudge
    );
}

// ─── Domain: Completion nudge ───────────────────────────────────────────────

#[given("a workflow state with all 16 steps checked")]
fn given_all_checked(world: &mut QuectoWorld) {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=16 {
        state.check(i).unwrap();
    }
    world.workflow_state = Some(Arc::new(Mutex::new(state)));
}

#[when("I generate the completion nudge")]
fn when_completion_nudge(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    world.workflow_nudge = s.completion_nudge();
}

// ─── Domain: Step threshold check ───────────────────────────────────────────

#[given(expr = "a workflow state with steps 1 through {int} checked")]
fn given_steps_through(world: &mut QuectoWorld, n: i32) {
    let mut state = WorkflowState::default_bdd();
    for i in 1..=(n as u32) {
        state.check(i).unwrap();
    }
    world.workflow_state = Some(Arc::new(Mutex::new(state)));
}

#[when(expr = "I check steps complete before step {int}")]
fn when_check_steps_complete(world: &mut QuectoWorld, before_step: i32) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    world.commit_check_result = Some(
        s.check_steps_complete(before_step as u32)
            .map_err(|e| e.to_string()),
    );
}

#[then("the check should fail")]
fn then_check_fails(world: &mut QuectoWorld) {
    let result = world
        .commit_check_result
        .as_ref()
        .expect("commit check result should exist");
    assert!(result.is_err(), "expected check to fail, got Ok");
}

#[then("the check should pass")]
fn then_check_passes(world: &mut QuectoWorld) {
    let result = world
        .commit_check_result
        .as_ref()
        .expect("commit check result should exist");
    assert!(
        result.is_ok(),
        "expected check to pass, got Err: {}",
        result.as_ref().unwrap_err()
    );
}

#[then(expr = "the block reason should contain {string}")]
fn then_block_reason_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .commit_check_result
        .as_ref()
        .expect("commit check result should exist");
    let err = result.as_ref().unwrap_err();
    assert!(
        err.contains(&expected),
        "expected block reason containing '{}', got: {}",
        expected,
        err
    );
}

// ─── Domain: Persistence ────────────────────────────────────────────────────

#[given(expr = "the active issue is {int} {string}")]
fn given_active_issue(world: &mut QuectoWorld, number: i32, title: String) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let mut s = state.lock().unwrap();
    s.set_issue(number as u32, title);
}

#[when("I serialize the workflow state")]
fn when_serialize(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    let persistable = s.to_persistable();
    let json = serde_json::to_string(&persistable).expect("should serialize");
    world.workflow_serialized = Some(json);
}

#[when("I serialize and deserialize the workflow state")]
fn when_roundtrip(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    let steps = s.steps().to_vec();
    let persistable = s.to_persistable();
    let json = serde_json::to_string(&persistable).expect("should serialize");
    let restored: WorkflowPersistable = serde_json::from_str(&json).expect("should deserialize");
    let new_state = WorkflowState::from_persistable_with_steps(&restored, Some(steps));
    drop(s);
    world.workflow_state = Some(Arc::new(Mutex::new(new_state)));
}

#[then(expr = "the serialized state should contain step {int} as done")]
fn then_serialized_step_done(world: &mut QuectoWorld, num: i32) {
    let json = world
        .workflow_serialized
        .as_ref()
        .expect("serialized state should exist");
    let persistable: WorkflowPersistable = serde_json::from_str(json).expect("should parse");
    assert!(
        persistable.done[(num - 1) as usize],
        "step {} should be done in serialized state",
        num
    );
}

#[then(expr = "the serialized state should contain step {int} as not done")]
fn then_serialized_step_not_done(world: &mut QuectoWorld, num: i32) {
    let json = world
        .workflow_serialized
        .as_ref()
        .expect("serialized state should exist");
    let persistable: WorkflowPersistable = serde_json::from_str(json).expect("should parse");
    assert!(
        !persistable.done[(num - 1) as usize],
        "step {} should not be done in serialized state",
        num
    );
}

#[then(expr = "the serialized state should contain issue {int} {string}")]
fn then_serialized_issue(world: &mut QuectoWorld, number: i32, title: String) {
    let json = world
        .workflow_serialized
        .as_ref()
        .expect("serialized state should exist");
    let persistable: WorkflowPersistable = serde_json::from_str(json).expect("should parse");
    let issue = persistable
        .active_issue
        .as_ref()
        .expect("should have active issue");
    assert_eq!(issue.0, number as u32);
    assert_eq!(issue.1, title);
}

// ─── Tool: check_commit action ──────────────────────────────────────────────

#[given(expr = "a workflow tool with default state and guard before step {int}")]
fn given_tool_with_guard(world: &mut QuectoWorld, num: i32) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    let mut tool = WorkflowTool::new(state);
    tool.set_guards(vec![GuardRule {
        commands: vec!["git commit".into()],
        before_step: num as u32,
        message: "Complete steps first.".into(),
    }]);
    world.workflow_tool = Some(tool);
}

#[given("a workflow tool with default state and no guards")]
fn given_tool_with_no_guards(world: &mut QuectoWorld) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    world.workflow_tool = Some(WorkflowTool::new(state));
}

// ─── System prompt with guards ──────────────────────────────────────────────

#[when("I build the workflow system prompt snippet with guards")]
fn when_build_snippet_with_guards(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    let guards = vec![GuardRule {
        commands: vec!["git commit".into()],
        before_step: 7,
        message: "Complete steps first.".into(),
    }];
    world.workflow_snippet = Some(s.system_prompt_snippet_with_guards(&guards));
}

// ─── Config integration ─────────────────────────────────────────────────────

#[given("a config file with workflow auto_continue true and guards configured")]
fn given_config_with_auto_and_guards(world: &mut QuectoWorld) {
    super::ensure_temp_dir(world);
    let base = super::base_path(world);
    let config_path = base.join("config.json");
    let config_json = r#"{
        "workflow": {
            "enabled": true,
            "auto_continue": true,
            "guards": [{"commands": ["git commit"], "before_step": 7, "message": "Not yet."}]
        },
        "providers": { "openai": { "api_key": "sk-test" } }
    }"#;
    std::fs::write(&config_path, config_json).unwrap();
    world.config_path = Some(config_path.to_string_lossy().to_string());
}

// --- #405: Guard should ignore patterns inside quoted strings ---

#[given(expr = "a workflow guard blocking {string} before step 7")]
fn given_guard_blocking(world: &mut QuectoWorld, pattern: String) {
    world.env_overrides.insert("_guard_pattern".into(), pattern);
    world.env_overrides.insert("_guard_step".into(), "7".into());
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state);
}

#[when(expr = "the bash command is {string}")]
fn when_bash_command(world: &mut QuectoWorld, command: String) {
    use quecto::infrastructure::tools::workflow_tool::WorkflowGuard;
    let pattern = world.env_overrides["_guard_pattern"].clone();
    let step: u32 = world.env_overrides["_guard_step"].parse().unwrap();
    let state = world.workflow_state.clone().unwrap();
    let guard = WorkflowGuard::new(
        state,
        vec![GuardRule {
            commands: vec![pattern],
            before_step: step,
            message: "Blocked by guard.".into(),
        }],
    );
    let json_args = serde_json::json!({"command": command}).to_string();
    let result = quecto::domain::tool::ToolGuard::check(&guard, "bash", &json_args);
    world.guard_result = Some(result);
}

#[then("the guard should allow the command")]
fn then_guard_allows(world: &mut QuectoWorld) {
    let result = world.guard_result.take().expect("guard result");
    assert!(
        result.is_ok(),
        "guard should allow but blocked: {}",
        result.unwrap_err()
    );
}

#[given("a config file with only workflow enabled true")]
fn given_config_enabled_only(world: &mut QuectoWorld) {
    super::ensure_temp_dir(world);
    let base = super::base_path(world);
    let config_path = base.join("config.json");
    let config_json = r#"{
        "workflow": { "enabled": true },
        "providers": { "openai": { "api_key": "sk-test" } }
    }"#;
    std::fs::write(&config_path, config_json).unwrap();
    world.config_path = Some(config_path.to_string_lossy().to_string());
}
