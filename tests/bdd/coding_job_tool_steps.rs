// BDD step definitions for coding_job_tool.feature
//
// These steps exercise the CodingJobTool through the real Tool trait,
// verifying that the infrastructure adapter correctly bridges JSON
// tool calls to the CodingCoordinator application layer.

use cucumber::{given, then, when};
use std::sync::{Arc, Mutex};

use quecto::application::coding_coordinator::{CodingCoordinator, CoordinatorPolicy};
use quecto::domain::coding_ports::CodingJobService;
use quecto::domain::tool::Tool;
use quecto::infrastructure::tools::coding_job::CodingJobTool;

use super::{BddRepoValidator, BddSkillResolver, QuectoWorld};

/// Build a CodingJobTool backed by a real CodingCoordinator with BDD mocks.
fn build_tool(world: &mut QuectoWorld) -> Arc<CodingJobTool> {
    let validator = BddRepoValidator {
        valid_repos: vec!["test-repo".to_string()],
        valid_refs: vec![("test-repo".to_string(), "main".to_string())],
    };
    let resolver = BddSkillResolver {
        available: vec!["default-skill".to_string()],
    };
    let coord = CodingCoordinator::new(validator, resolver, CoordinatorPolicy::default());
    let coord = Arc::new(Mutex::new(coord));
    world.coding_job_tool_coordinator = Some(coord.clone());
    let svc: Arc<Mutex<dyn CodingJobService>> = coord;
    Arc::new(CodingJobTool::new(svc))
}

fn tool_ref(world: &QuectoWorld) -> &Arc<CodingJobTool> {
    world
        .coding_job_tool
        .as_ref()
        .expect("coding_job tool should be set up")
}

fn exec_tool(world: &QuectoWorld, input: &str) -> quecto::domain::tool::ToolResult {
    let tool = tool_ref(world);
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(tool.execute(input))
        .expect("execute should not panic")
}

fn last_result(world: &QuectoWorld) -> &quecto::domain::tool::ToolResult {
    world
        .coding_job_tool_last_result
        .as_ref()
        .expect("a tool result should exist")
}

// ============================================================================
// Given steps
// ============================================================================

#[given("a coding_job tool with a mock coordinator")]
fn given_coding_job_tool(world: &mut QuectoWorld) {
    let tool = build_tool(world);
    world.coding_job_tool = Some(tool);
}

#[given("a coding job exists via the tool")]
fn given_job_exists(world: &mut QuectoWorld) {
    let result = exec_tool(
        world,
        r#"{"action":"run","goal":"test task","repo":"test-repo","base_ref":"main"}"#,
    );
    assert!(!result.is_error, "run failed: {}", result.content);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    world.coding_job_tool_last_job_id = Some(v["job_id"].as_str().unwrap().to_string());
}

#[given(expr = "a coding job exists via the tool in state {string}")]
fn given_job_in_state(world: &mut QuectoWorld, target_state: String) {
    given_job_exists(world);
    let jid = world
        .coding_job_tool_last_job_id
        .clone()
        .expect("job_id should exist");
    let coord = world
        .coding_job_tool_coordinator
        .as_ref()
        .expect("coordinator should exist");
    let mut c = coord.lock().unwrap();
    match target_state.as_str() {
        "queued" => {}
        "preparing" => {
            c.begin_preparation(&jid).unwrap();
        }
        "running" => {
            c.begin_preparation(&jid).unwrap();
            c.mark_ready(&jid, 9999, None).unwrap();
        }
        "canceled" => {
            c.cancel(&jid).unwrap();
        }
        "succeeded" => {
            c.begin_preparation(&jid).unwrap();
            c.mark_ready(&jid, 9999, None).unwrap();
            c.mark_succeeded(quecto::application::coding_coordinator::SuccessInfo {
                job_id: &jid,
                summary: "done",
                artifacts: vec![],
                duration_ms: None,
            })
            .unwrap();
        }
        "failed" => {
            c.begin_preparation(&jid).unwrap();
            c.mark_ready(&jid, 9999, None).unwrap();
            c.mark_failed(quecto::application::coding_coordinator::FailureInfo {
                job_id: &jid,
                error_code: quecto::domain::coding_job::ErrorCode::Internal,
                error_detail: "test failure",
                is_retriable: None,
                duration_ms: None,
            })
            .unwrap();
        }
        other => panic!("unsupported target state: {other}"),
    }
}

#[given(expr = "{int} coding jobs exist via the tool")]
fn given_n_jobs(world: &mut QuectoWorld, count: usize) {
    for _ in 0..count {
        let result = exec_tool(
            world,
            r#"{"action":"run","goal":"batch task","repo":"test-repo","base_ref":"main"}"#,
        );
        assert!(!result.is_error, "run failed: {}", result.content);
    }
}

// ============================================================================
// When steps
// ============================================================================

#[when(expr = "I execute the coding_job tool with run goal {string} repo {string} ref {string}")]
fn when_run(world: &mut QuectoWorld, goal: String, repo: String, base_ref: String) {
    let input = serde_json::json!({
        "action": "run",
        "goal": goal,
        "repo": repo,
        "base_ref": base_ref,
    })
    .to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with run priority {string}")]
fn when_run_with_priority(world: &mut QuectoWorld, priority: String) {
    let input = serde_json::json!({
        "action": "run",
        "goal": "priority test",
        "repo": "test-repo",
        "base_ref": "main",
        "priority": priority,
    })
    .to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when("I execute the coding_job tool with status for current job")]
fn when_status_current(world: &mut QuectoWorld) {
    let jid = world
        .coding_job_tool_last_job_id
        .clone()
        .expect("a job should exist");
    let input = serde_json::json!({"action": "status", "job_id": jid}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with status for job {string}")]
fn when_status_specific(world: &mut QuectoWorld, job_id: String) {
    let input = serde_json::json!({"action": "status", "job_id": job_id}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when("I execute the coding_job tool with cancel for current job")]
fn when_cancel_current(world: &mut QuectoWorld) {
    let jid = world
        .coding_job_tool_last_job_id
        .clone()
        .expect("a job should exist");
    let input = serde_json::json!({"action": "cancel", "job_id": jid}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with cancel for job {string}")]
fn when_cancel_specific(world: &mut QuectoWorld, job_id: String) {
    let input = serde_json::json!({"action": "cancel", "job_id": job_id}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when("I execute the coding_job tool with cleanup for current job")]
fn when_cleanup_current(world: &mut QuectoWorld) {
    let jid = world
        .coding_job_tool_last_job_id
        .clone()
        .expect("a job should exist");
    let input = serde_json::json!({"action": "cleanup", "job_id": jid}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with action {string}")]
fn when_action_only(world: &mut QuectoWorld, action: String) {
    let input = serde_json::json!({"action": action}).to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with list filter {string}")]
fn when_list_filter(world: &mut QuectoWorld, state: String) {
    let input = serde_json::json!({
        "action": "list",
        "state_filter": [state],
    })
    .to_string();
    world.coding_job_tool_last_result = Some(exec_tool(world, &input));
}

#[when(expr = "I execute the coding_job tool with raw input {string}")]
fn when_raw_input(world: &mut QuectoWorld, raw: String) {
    world.coding_job_tool_last_result = Some(exec_tool(world, &raw));
}

// ============================================================================
// Then steps
// ============================================================================

#[then(expr = "the coding_job tool name should be {string}")]
fn then_tool_name(world: &mut QuectoWorld, expected: String) {
    let def = tool_ref(world).definition();
    assert_eq!(def.name, expected);
}

#[then("the coding_job tool description should mention coding jobs")]
fn then_tool_description(world: &mut QuectoWorld) {
    let def = tool_ref(world).definition();
    assert!(
        def.description.to_lowercase().contains("coding job"),
        "expected description to mention coding jobs, got: {}",
        def.description
    );
}

#[then(expr = "the coding_job tool schema should require an {string} field")]
fn then_schema_requires(world: &mut QuectoWorld, field: String) {
    let def = tool_ref(world).definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let required = schema["required"]
        .as_array()
        .expect("schema should have required array");
    assert!(
        required.iter().any(|v| v.as_str() == Some(field.as_str())),
        "schema required should include '{field}'"
    );
}

#[then("the coding_job result should not be an error")]
fn then_not_error(world: &mut QuectoWorld) {
    let r = last_result(world);
    assert!(!r.is_error, "expected success, got error: {}", r.content);
}

#[then("the coding_job result should be an error")]
fn then_is_error(world: &mut QuectoWorld) {
    let r = last_result(world);
    assert!(r.is_error, "expected error, got success: {}", r.content);
}

#[then(expr = "the coding_job result should contain {string}")]
fn then_result_contains(world: &mut QuectoWorld, substr: String) {
    let r = last_result(world);
    assert!(
        r.content.contains(&substr),
        "expected '{}' in result: {}",
        substr,
        r.content
    );
}
