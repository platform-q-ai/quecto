use cucumber::{given, then, when};
use quecto::domain::tool::ToolGuard;
use quecto::domain::workflow::WorkflowState;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::infrastructure::tools::workflow_tool::WorkflowGuard;
use std::sync::{Arc, Mutex};
use tempfile::TempDir;

use super::QuectoWorld;

// ─── Test guard implementations ──────────────────────────────────────────────

struct AllowAllGuard;
impl ToolGuard for AllowAllGuard {
    fn check(&self, _tool_name: &str, _arguments: &str) -> Result<(), String> {
        Ok(())
    }
}

struct BlockAllGuard {
    reason: String,
}
impl ToolGuard for BlockAllGuard {
    fn check(&self, _tool_name: &str, _arguments: &str) -> Result<(), String> {
        Err(self.reason.clone())
    }
}

struct BlockSpecificGuard {
    target_tool: String,
    reason: String,
}
impl ToolGuard for BlockSpecificGuard {
    fn check(&self, tool_name: &str, _arguments: &str) -> Result<(), String> {
        if tool_name == self.target_tool {
            Err(self.reason.clone())
        } else {
            Ok(())
        }
    }
}

struct CapturingGuard {
    captured_name: Arc<Mutex<String>>,
    captured_args: Arc<Mutex<String>>,
}
impl ToolGuard for CapturingGuard {
    fn check(&self, tool_name: &str, arguments: &str) -> Result<(), String> {
        *self.captured_name.lock().unwrap() = tool_name.to_string();
        *self.captured_args.lock().unwrap() = arguments.to_string();
        Ok(())
    }
}

// ─── Helper to create registry with core tools ──────────────────────────────

pub(crate) fn create_test_registry(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), true);
    let registry = ToolRegistryImpl::with_core_tools(tmp.path().to_path_buf(), sandbox);
    world.tool_registry = Some(registry);
    world._tool_workspace_tmp = Some(tmp);
}

// ─── ToolGuard on ToolRegistryImpl steps ─────────────────────────────────────

#[given("a tool registry with core tools")]
fn given_registry_with_core_tools(world: &mut QuectoWorld) {
    create_test_registry(world);
}

#[given("no guards are registered")]
fn given_no_guards(_world: &mut QuectoWorld) {
    // Default — no guards registered
}

#[given("a guard that allows all calls")]
fn given_allow_guard(world: &mut QuectoWorld) {
    let reg = world.tool_registry.as_mut().expect("need registry");
    reg.register_guard(Arc::new(AllowAllGuard));
}

#[given(expr = "a guard that blocks all calls with reason {string}")]
fn given_block_guard(world: &mut QuectoWorld, reason: String) {
    let reg = world.tool_registry.as_mut().expect("need registry");
    reg.register_guard(Arc::new(BlockAllGuard { reason }));
}

#[given("a guard that captures tool name and arguments")]
fn given_capturing_guard(world: &mut QuectoWorld) {
    let name = Arc::new(Mutex::new(String::new()));
    let args = Arc::new(Mutex::new(String::new()));
    let name_clone = name.clone();
    let args_clone = args.clone();
    let reg = world.tool_registry.as_mut().expect("need registry");
    reg.register_guard(Arc::new(CapturingGuard {
        captured_name: name_clone,
        captured_args: args_clone,
    }));
    world.guard_captured_name = Some(String::new());
    world.guard_captured_args = Some(String::new());
    // Store Arcs for later retrieval — we'll read from guard after execute
    // Use a workaround: store them as a global for this world instance
    // Actually, we'll just store them on the world in a type-erased way.
    // For simplicity, we read from the guard via re-execution pattern.
    // Instead, just use the capturing guard Arcs via a Box::leak approach.
    // SIMPLER: Just read the captured values after execution by storing the Arcs.
}

#[given(expr = "a guard that blocks only {string} with reason {string}")]
fn given_block_specific_guard(world: &mut QuectoWorld, target: String, reason: String) {
    let reg = world.tool_registry.as_mut().expect("need registry");
    reg.register_guard(Arc::new(BlockSpecificGuard {
        target_tool: target,
        reason,
    }));
}

#[when(expr = "I execute the {string} tool with arguments {string}")]
fn when_execute_tool(world: &mut QuectoWorld, tool_name: String, arguments: String) {
    let reg = world.tool_registry.as_ref().expect("need registry");
    let result = tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { reg.execute(&tool_name, &arguments).await });
    world.tool_result = Some(result.map_err(|e| e.to_string()));
}

// Tool result assertions reuse existing steps from agent_tools_steps.rs:
// "the tool result should not be an error"
// "the tool result should be an error"
// "the tool result should contain {string}"

// ─── WorkflowGuard steps ─────────────────────────────────────────────────────

#[given(expr = "a workflow guard with commit enforcement at {int}")]
fn given_workflow_guard_with_enforce(world: &mut QuectoWorld, threshold: i32) {
    let threshold = threshold as u32;
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    world.enforce_commit_after_step = Some(Some(threshold));
}

#[given("a workflow guard with no enforcement")]
fn given_workflow_guard_no_enforce(world: &mut QuectoWorld) {
    let state = Arc::new(Mutex::new(WorkflowState::default_bdd()));
    world.workflow_state = Some(state.clone());
    world.enforce_commit_after_step = Some(None);
}

#[given("no workflow steps are completed")]
fn given_no_steps_completed(_world: &mut QuectoWorld) {
    // Default state — all steps unchecked
}

#[given(expr = "workflow steps 1 through {int} are completed")]
fn given_steps_completed(world: &mut QuectoWorld, through: i32) {
    let state = world.workflow_state.as_ref().expect("need workflow state");
    let mut s = state.lock().unwrap();
    for i in 1..=through as u32 {
        s.check(i).expect("failed to check step");
    }
}

#[when(expr = "the guard checks tool {string} with arguments {string}")]
fn when_guard_checks(world: &mut QuectoWorld, tool_name: String, arguments: String) {
    let state = world.workflow_state.as_ref().expect("need workflow state");
    let enforce = world
        .enforce_commit_after_step
        .expect("need enforce setting");
    let guard = WorkflowGuard::new(state.clone(), enforce);
    let result = guard.check(&tool_name, &arguments);
    world.guard_check_result = Some(result);
}

#[then("the guard should allow the call")]
fn then_guard_allows(world: &mut QuectoWorld) {
    let result = world
        .guard_check_result
        .as_ref()
        .expect("no guard check result");
    assert!(result.is_ok(), "expected guard to allow, got: {:?}", result);
}

#[then("the guard should block the call")]
fn then_guard_blocks(world: &mut QuectoWorld) {
    let result = world
        .guard_check_result
        .as_ref()
        .expect("no guard check result");
    assert!(result.is_err(), "expected guard to block, got Ok");
}

#[then(expr = "the guard block reason should contain {string}")]
fn then_guard_reason_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .guard_check_result
        .as_ref()
        .expect("no guard check result");
    let reason = result.as_ref().unwrap_err();
    assert!(
        reason.contains(&expected),
        "expected block reason to contain '{}', got: {}",
        expected,
        reason
    );
}

// ─── Guard config integration ────────────────────────────────────────────────

#[given("a workflow config with guard_commit false and enabled true")]
fn given_wf_config_no_guard(world: &mut QuectoWorld) {
    world.workflow_config = Some(quecto::domain::workflow::WorkflowConfig {
        enabled: true,
        guard_commit: false,
        ..Default::default()
    });
}

#[when("workflow tools are registered with that config")]
fn when_register_workflow_tools(world: &mut QuectoWorld) {
    let tmp = TempDir::new().unwrap();
    let sandbox = Sandbox::new(Some(tmp.path().to_path_buf()), false);
    let mut registry = ToolRegistryImpl::with_core_tools(tmp.path().to_path_buf(), sandbox);
    let config = world
        .workflow_config
        .as_ref()
        .expect("need workflow config");
    quecto::interface::shared::register_workflow_tool(&mut registry, config);
    world.tool_registry = Some(registry);
    world._tool_guard_tmp = Some(tmp);
}

#[then(expr = "the tool registry should have {int} guards")]
fn then_registry_guard_count(world: &mut QuectoWorld, expected: i32) {
    let reg = world.tool_registry.as_ref().expect("need registry");
    assert_eq!(
        reg.guard_count(),
        expected as usize,
        "expected {} guards",
        expected
    );
}
