use cucumber::{given, then, when};
use quecto::domain::tool::Tool;
use quecto::domain::workflow::{WorkflowConfig, WorkflowState};
use quecto::infrastructure::tools::workflow_tool::{WorkflowEventEmitter, WorkflowTool};
use std::sync::{Arc, Mutex};

use super::QuectoWorld;

// Helper to get or create workflow state
fn get_or_create_state(world: &mut QuectoWorld) -> Arc<Mutex<WorkflowState>> {
    if world.workflow_state.is_none() {
        world.workflow_state = Some(Arc::new(Mutex::new(WorkflowState::default_bdd())));
    }
    world.workflow_state.clone().unwrap()
}

// ─── Domain: WorkflowState ──────────────────────────────────────────────────

#[given("a default workflow state")]
fn given_default_state(world: &mut QuectoWorld) {
    world.workflow_state = Some(Arc::new(Mutex::new(WorkflowState::default_bdd())));
    world.workflow_error = None;
}

#[when(expr = "I check step {int}")]
fn when_check_step(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    world.workflow_error = s.check(num as u32).err().map(|e| e.to_string());
}

#[when(expr = "I try to check step {int}")]
fn when_try_check_step(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    world.workflow_error = s.check(num as u32).err().map(|e| e.to_string());
}

#[when(expr = "I uncheck step {int}")]
fn when_uncheck_step(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    world.workflow_error = s.uncheck(num as u32).err().map(|e| e.to_string());
}

#[when(expr = "I try to uncheck step {int}")]
fn when_try_uncheck_step(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    world.workflow_error = s.uncheck(num as u32).err().map(|e| e.to_string());
}

#[when(expr = "I skip step {int}")]
fn when_skip_step(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    world.workflow_error = s.skip(num as u32).err().map(|e| e.to_string());
}

#[when("I reset the workflow")]
fn when_reset(world: &mut QuectoWorld) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    s.reset();
}

#[when(expr = "I set issue {int} {string}")]
fn when_set_issue(world: &mut QuectoWorld, number: i32, title: String) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    s.set_issue(number as u32, title);
}

#[when("I clear the active issue")]
fn when_clear_issue(world: &mut QuectoWorld) {
    let state = get_or_create_state(world);
    let mut s = state.lock().unwrap();
    s.clear_issue();
}

#[then(expr = "the workflow state should have {int} steps")]
fn then_state_has_steps(world: &mut QuectoWorld, count: i32) {
    let count = count as usize;
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    assert_eq!(s.steps().len(), count);
}

#[then("all steps should be unchecked")]
fn then_all_unchecked(world: &mut QuectoWorld) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    assert!(s.done_flags().iter().all(|&d| !d));
}

#[then("the active issue should be None")]
fn then_no_active_issue(world: &mut QuectoWorld) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    assert!(s.active_issue().is_none());
}

#[then(expr = "step {int} should be checked")]
fn then_step_checked(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    assert!(
        s.is_done(num as u32).unwrap(),
        "step {} should be checked",
        num
    );
}

#[then(expr = "step {int} should be unchecked")]
fn then_step_unchecked(world: &mut QuectoWorld, num: i32) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    assert!(
        !s.is_done(num as u32).unwrap(),
        "step {} should be unchecked",
        num
    );
}

#[then(expr = "the check should fail with {string}")]
fn then_check_fail(world: &mut QuectoWorld, expected: String) {
    let err = world.workflow_error.as_ref().expect("expected an error");
    assert!(
        err.contains(&expected),
        "expected error containing '{}', got: {}",
        expected,
        err
    );
}

#[then(expr = "the uncheck should fail with {string}")]
fn then_uncheck_fail(world: &mut QuectoWorld, expected: String) {
    let err = world.workflow_error.as_ref().expect("expected an error");
    assert!(
        err.contains(&expected),
        "expected error containing '{}', got: {}",
        expected,
        err
    );
}

#[then(expr = "the active issue should be {int} {string}")]
fn then_active_issue(world: &mut QuectoWorld, number: i32, title: String) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    let issue = s.active_issue().expect("expected active issue");
    assert_eq!(issue.0, number as u32);
    assert_eq!(issue.1, title);
}

#[then(expr = "the progress should be {int} done out of {int} total with percent {int}")]
fn then_progress(world: &mut QuectoWorld, done: i32, total: i32, percent: i32) {
    let state = get_or_create_state(world);
    let s = state.lock().unwrap();
    let progress = s.progress();
    assert_eq!(progress.done, done as u32);
    assert_eq!(progress.total, total as u32);
    assert_eq!(progress.percent, percent as u32);
}

// ─── Domain: WorkflowConfig ────────────────────────────────────────────────

#[given("a default workflow config")]
fn given_default_config(world: &mut QuectoWorld) {
    world.workflow_config = Some(WorkflowConfig::default());
}

#[given("a workflow config with enabled false")]
fn given_disabled_config(world: &mut QuectoWorld) {
    world.workflow_config = Some(WorkflowConfig {
        enabled: false,
        ..Default::default()
    });
}

#[then("the workflow config should be enabled")]
fn then_config_enabled(world: &mut QuectoWorld) {
    if let Some(ref config) = world.workflow_config {
        assert!(config.enabled);
    } else if let Some(ref config) = world.config {
        assert!(config.workflow.enabled);
    } else {
        panic!("no workflow config or loaded config available");
    }
}

#[then("the workflow config should not be enabled")]
fn then_config_not_enabled(world: &mut QuectoWorld) {
    let config = world
        .workflow_config
        .as_ref()
        .expect("workflow config should exist");
    assert!(!config.enabled);
}

#[then(expr = "the workflow config should have {int} steps")]
fn then_config_has_steps(world: &mut QuectoWorld, count: i32) {
    let count = count as usize;
    if let Some(ref config) = world.workflow_config {
        assert_eq!(config.steps.len(), count);
    } else if let Some(ref config) = world.config {
        assert_eq!(config.workflow.steps.len(), count);
    } else {
        panic!("no workflow config or loaded config available");
    }
}

#[then(expr = "the first step should be id {int} label {string} phase {string}")]
fn then_first_step(world: &mut QuectoWorld, id: i32, label: String, phase: String) {
    let config = world
        .workflow_config
        .as_ref()
        .expect("workflow config should exist");
    let step = &config.steps[0];
    assert_eq!(step.id, id as u32);
    assert_eq!(step.label, label);
    assert_eq!(step.phase, phase);
}

#[then(expr = "the last step should be id {int} label {string} phase {string}")]
fn then_last_step(world: &mut QuectoWorld, id: i32, label: String, phase: String) {
    let config = world
        .workflow_config
        .as_ref()
        .expect("workflow config should exist");
    let step = config.steps.last().expect("should have steps");
    assert_eq!(step.id, id as u32);
    assert_eq!(step.label, label);
    assert_eq!(step.phase, phase);
}

// ─── Config integration ────────────────────────────────────────────────────

#[given("a config file with workflow enabled and custom steps")]
fn given_config_with_workflow(world: &mut QuectoWorld) {
    super::ensure_temp_dir(world);
    let base = super::base_path(world);
    let config_path = base.join("config.json");
    let config_json = r#"{
        "workflow": {
            "enabled": true,
            "steps": [
                {"id": 1, "label": "Test Step", "phase": "red"}
            ]
        },
        "providers": {
            "openai": { "api_key": "sk-test" }
        }
    }"#;
    std::fs::write(&config_path, config_json).unwrap();
    world.config_path = Some(config_path.to_string_lossy().to_string());
}

#[given("a config file without workflow section")]
fn given_config_without_workflow(world: &mut QuectoWorld) {
    super::ensure_temp_dir(world);
    let base = super::base_path(world);
    let config_path = base.join("config.json");
    let config_json = r#"{
        "providers": {
            "openai": { "api_key": "sk-test" }
        }
    }"#;
    std::fs::write(&config_path, config_json).unwrap();
    world.config_path = Some(config_path.to_string_lossy().to_string());
}

// ─── Tool: WorkflowTool ────────────────────────────────────────────────────

#[given("a workflow tool with default state")]
fn given_workflow_tool(world: &mut QuectoWorld) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    world.workflow_tool = Some(WorkflowTool::new(state));
}

#[given("a workflow tool with event emitter")]
fn given_workflow_tool_with_emitter(world: &mut QuectoWorld) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    let events: Arc<Mutex<Vec<serde_json::Value>>> = Arc::new(Mutex::new(vec![]));
    world.workflow_events = Some(events.clone());
    let emitter: WorkflowEventEmitter = Arc::new(move |event| {
        events.lock().unwrap().push(event);
    });
    world.workflow_tool = Some(WorkflowTool::with_event_emitter(state, emitter));
}

#[when(expr = "I execute the workflow tool with action {string}")]
async fn when_execute_action(world: &mut QuectoWorld, action: String) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let args = serde_json::json!({"action": action}).to_string();
    let result = tool.execute(&args).await;
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when(expr = "I execute the workflow tool with action {string} and step {int}")]
async fn when_execute_action_with_step(world: &mut QuectoWorld, action: String, num: i32) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let args = serde_json::json!({"action": action, "step": num}).to_string();
    let result = tool.execute(&args).await;
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when(expr = "I execute the workflow tool with action {string} and issue {int} {string}")]
async fn when_execute_action_with_issue(
    world: &mut QuectoWorld,
    action: String,
    number: i32,
    title: String,
) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let args = serde_json::json!({
        "action": action,
        "issueNumber": number,
        "issueTitle": title
    })
    .to_string();
    let result = tool.execute(&args).await;
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[when("I execute the workflow tool with empty arguments")]
async fn when_execute_empty(world: &mut QuectoWorld) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let result = tool.execute("{}").await;
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

#[then("the workflow tool result should not be an error")]
fn then_wf_not_error(world: &mut QuectoWorld) {
    let result = world
        .tool_result
        .as_ref()
        .expect("tool result should exist");
    let tr = result.as_ref().expect("tool should not return DomainError");
    assert!(!tr.is_error, "expected non-error, got: {}", tr.content);
}

#[then("the workflow tool result should be an error")]
fn then_wf_is_error(world: &mut QuectoWorld) {
    let result = world
        .tool_result
        .as_ref()
        .expect("tool result should exist");
    let tr = result.as_ref().expect("tool should not return DomainError");
    assert!(tr.is_error, "expected error, got: {}", tr.content);
}

#[then(expr = "the workflow tool result should contain {string}")]
fn then_wf_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .tool_result
        .as_ref()
        .expect("tool result should exist");
    let tr = result.as_ref().expect("tool should not return DomainError");
    assert!(
        tr.content.contains(&expected),
        "expected content containing '{}', got: {}",
        expected,
        tr.content
    );
}

#[then(expr = "the workflow tool definition name should be {string}")]
fn then_definition_name(world: &mut QuectoWorld, name: String) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let def = tool.definition();
    assert_eq!(def.name.as_ref(), name);
}

#[then("the workflow tool definition should have a parameters schema")]
fn then_definition_has_schema(world: &mut QuectoWorld) {
    let tool = world
        .workflow_tool
        .as_ref()
        .expect("workflow tool should exist");
    let def = tool.definition();
    assert!(!def.parameters_schema.is_empty());
    // Verify it's valid JSON
    let _: serde_json::Value = serde_json::from_str(&def.parameters_schema)
        .expect("parameters_schema should be valid JSON");
}

// ─── UDS event emission ─────────────────────────────────────────────────────

#[then("a workflow_state event should have been emitted")]
fn then_event_emitted(world: &mut QuectoWorld) {
    let events = world
        .workflow_events
        .as_ref()
        .expect("events should be tracked");
    let evts = events.lock().unwrap();
    assert!(
        !evts.is_empty(),
        "expected at least one workflow_state event"
    );
    assert_eq!(evts.last().unwrap()["type"], "workflow_state");
}

#[then(expr = "the event should contain {string}")]
fn then_event_contains(world: &mut QuectoWorld, key: String) {
    let events = world
        .workflow_events
        .as_ref()
        .expect("events should be tracked");
    let evts = events.lock().unwrap();
    let last = evts.last().expect("expected at least one event");
    let json_str = serde_json::to_string(last).unwrap();
    assert!(
        json_str.contains(&key),
        "expected event to contain '{}', got: {}",
        key,
        json_str
    );
}

// ─── System prompt injection ────────────────────────────────────────────────

#[given("a default workflow state with step 1 checked")]
fn given_state_with_step_1(world: &mut QuectoWorld) {
    let mut state = WorkflowState::default_bdd();
    state.check(1).unwrap();
    world.workflow_state = Some(Arc::new(Mutex::new(state)));
}

#[given(expr = "a default workflow state with issue {int} {string}")]
fn given_state_with_issue(world: &mut QuectoWorld, number: i32, title: String) {
    let mut state = WorkflowState::default_bdd();
    state.set_issue(number as u32, title);
    world.workflow_state = Some(Arc::new(Mutex::new(state)));
}

#[when("I build the workflow system prompt snippet")]
fn when_build_snippet(world: &mut QuectoWorld) {
    let state = world
        .workflow_state
        .as_ref()
        .expect("workflow state should exist");
    let s = state.lock().unwrap();
    world.workflow_snippet = Some(s.system_prompt_snippet());
}

#[then(expr = "the snippet should contain {string}")]
fn then_snippet_contains(world: &mut QuectoWorld, expected: String) {
    let snippet = world
        .workflow_snippet
        .as_ref()
        .expect("snippet should exist");
    assert!(
        snippet.contains(&expected),
        "expected snippet containing '{}', got:\n{}",
        expected,
        snippet
    );
}
