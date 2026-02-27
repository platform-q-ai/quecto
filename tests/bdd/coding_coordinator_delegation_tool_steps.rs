//! BDD step definitions for the CoordinatorDelegationTool.
//!
//! These steps test the thin IPC-based delegation tool that replaces
//! `CodingJobTool` in the main agent. The tool writes commands to the
//! coordinator inbox, polls the outbox for responses, and checks for
//! proactive notifications.

use cucumber::{given, then, when};
use std::sync::Arc;

use quecto::domain::coding_ipc::{
    CoordinatorIpcResponse, CoordinatorNotification, NotificationType,
};
use quecto::domain::tool::Tool;
use quecto::infrastructure::tools::coding_delegation::CoordinatorDelegationTool;

use crate::{BddDelegMockIpc, QuectoWorld};

// ============================================================================
// Given steps
// ============================================================================

#[given("a coordinator delegation tool with a mock IPC")]
fn given_delegation_tool(world: &mut QuectoWorld) {
    let mock = Arc::new(BddDelegMockIpc::new());
    world.deleg_mock_ipc = Some(mock.clone());
    world.deleg_tool = Some(Arc::new(CoordinatorDelegationTool::with_polling(
        mock, 1, 3,
    )));
}

#[given("a coordinator delegation tool with a mock IPC that times out")]
fn given_delegation_tool_timeout(world: &mut QuectoWorld) {
    let mock = Arc::new(BddDelegMockIpc::with_timeout());
    world.deleg_mock_ipc = Some(mock.clone());
    world.deleg_tool = Some(Arc::new(CoordinatorDelegationTool::with_polling(
        mock, 1, 2, // 2 attempts at 1ms = fast timeout
    )));
}

#[given(regex = r#"^the mock IPC will respond with ok true and body (.+)$"#)]
fn given_mock_ipc_ok_response(world: &mut QuectoWorld, body_str: String) {
    let body: serde_json::Value =
        serde_json::from_str(&body_str).expect("valid JSON body in feature");
    let mock = world.deleg_mock_ipc.as_ref().expect("mock ipc set");
    *mock.response.lock().unwrap() = Some(CoordinatorIpcResponse {
        command_id: String::new(), // will be overwritten when command is written
        ok: true,
        body: Some(body),
        error: None,
    });
}

#[given(regex = r#"^the mock IPC will respond with ok false and error "([^"]+)"$"#)]
fn given_mock_ipc_error_response(world: &mut QuectoWorld, error: String) {
    let mock = world.deleg_mock_ipc.as_ref().expect("mock ipc set");
    *mock.response.lock().unwrap() = Some(CoordinatorIpcResponse {
        command_id: String::new(),
        ok: false,
        body: None,
        error: Some(error),
    });
}

#[given(expr = "the mock IPC has pending notifications:")]
fn given_mock_ipc_notifications(world: &mut QuectoWorld, step: &cucumber::gherkin::Step) {
    let table = step.table.as_ref().expect("step should have a table");
    let mock = world.deleg_mock_ipc.as_ref().expect("mock ipc set");
    let mut notifs = mock.notifications.lock().unwrap();
    // Table columns: type | job_id | detail
    for row in &table.rows[1..] {
        // skip header row
        let ntype = match row[0].trim() {
            "worker_blocked" => NotificationType::WorkerBlocked,
            "job_failed" => NotificationType::JobFailed,
            "worker_stuck" => NotificationType::WorkerStuck,
            "batch_complete" => NotificationType::BatchComplete,
            "policy_violation" => NotificationType::PolicyViolation,
            other => panic!("unknown notification type: {other}"),
        };
        notifs.push(CoordinatorNotification {
            notification_type: ntype,
            job_id: Some(row[1].trim().to_string()),
            job_ids: vec![],
            detail: Some(row[2].trim().to_string()),
            no_progress_minutes: None,
            ts: "2026-01-15T10:00:00Z".to_string(),
        });
    }
}

// ============================================================================
// When steps
// ============================================================================

#[when(regex = r#"^I execute the delegation tool with action "(\w+)" and payload (.+)$"#)]
fn when_execute_delegation_action(world: &mut QuectoWorld, action: String, payload_str: String) {
    let payload: serde_json::Value =
        serde_json::from_str(&payload_str).expect("valid JSON payload");
    let mut merged = payload.as_object().cloned().unwrap_or_default();
    merged.insert("action".to_string(), serde_json::Value::String(action));
    let input = serde_json::to_string(&merged).expect("serialize");
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(tool.execute(&input)).expect("should not panic");
    world.deleg_result = Some(result);
}

#[when(regex = r#"^I execute the delegation tool with raw input "([^"]*)"$"#)]
fn when_execute_delegation_raw_double(world: &mut QuectoWorld, input: String) {
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(tool.execute(&input)).expect("should not panic");
    world.deleg_result = Some(result);
}

#[when(regex = r#"^I execute the delegation tool with raw input '([^']*)'$"#)]
fn when_execute_delegation_raw_single(world: &mut QuectoWorld, input: String) {
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    let result = rt.block_on(tool.execute(&input)).expect("should not panic");
    world.deleg_result = Some(result);
}

// ============================================================================
// Then steps
// ============================================================================

#[then(regex = r#"^the delegation tool name should be "([^"]+)"$"#)]
fn then_delegation_tool_name(world: &mut QuectoWorld, expected: String) {
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    assert_eq!(
        tool.definition().name,
        expected,
        "tool name should be '{expected}'"
    );
}

#[then("the delegation tool description should mention coding jobs")]
fn then_delegation_tool_description(world: &mut QuectoWorld) {
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    let desc = tool.definition().description;
    assert!(
        desc.to_lowercase().contains("coding") || desc.contains("WORKFLOW") || desc.contains("job"),
        "description should mention coding jobs, got: {desc}"
    );
}

#[then(regex = r#"^the delegation tool schema should require an "([^"]+)" field$"#)]
fn then_delegation_tool_schema_requires(world: &mut QuectoWorld, field: String) {
    let tool = world.deleg_tool.as_ref().expect("delegation tool set");
    let schema = tool.definition().parameters_schema;
    assert!(
        schema.contains(&field),
        "schema should contain '{field}', got: {schema}"
    );
}

#[then(regex = r#"^the mock IPC inbox should have received a command with action "(\w+)"$"#)]
fn then_mock_ipc_received_command(world: &mut QuectoWorld, expected_action: String) {
    let mock = world.deleg_mock_ipc.as_ref().expect("mock ipc set");
    let cmds = mock.commands.lock().unwrap();
    assert!(
        cmds.iter().any(|c| c.action == expected_action),
        "inbox should contain command with action '{expected_action}', got: {:?}",
        cmds.iter().map(|c| &c.action).collect::<Vec<_>>()
    );
}

#[then("the delegation tool result should not be an error")]
fn then_delegation_result_not_error(world: &mut QuectoWorld) {
    let result = world.deleg_result.as_ref().expect("delegation result set");
    assert!(
        !result.is_error,
        "result should not be an error, got: {}",
        result.content
    );
}

#[then("the delegation tool result should be an error")]
fn then_delegation_result_is_error(world: &mut QuectoWorld) {
    let result = world.deleg_result.as_ref().expect("delegation result set");
    assert!(
        result.is_error,
        "result should be an error, got: {}",
        result.content
    );
}

#[then(regex = r#"^the delegation tool result should contain "([^"]+)"$"#)]
fn then_delegation_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world.deleg_result.as_ref().expect("delegation result set");
    assert!(
        result.content.contains(&expected),
        "result should contain '{expected}', got: {}",
        result.content
    );
}
