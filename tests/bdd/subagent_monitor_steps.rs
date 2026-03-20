use super::*;
use quecto::infrastructure::tools::subagent_monitor::{apply_event, mark_exited};
use quecto::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};

// ===========================================================================
// Subagent Monitor BDD Steps (#522)
// ===========================================================================

// --- Given ---

#[given("a new SubagentEntry with status Starting")]
fn given_new_entry_starting(world: &mut QuectoWorld) {
    world.monitor_entry = Some(SubagentEntry::new(
        std::path::PathBuf::from("/stub/test.sock"),
        0,
    ));
}

#[given("subagent status variants Starting, Idle, Running, Error, Exited")]
fn given_all_status_variants(world: &mut QuectoWorld) {
    // Store all variants for display check in the Then step
    world.monitor_status_variants = Some(vec![
        SubagentStatus::Starting,
        SubagentStatus::Idle,
        SubagentStatus::Running,
        SubagentStatus::Error,
        SubagentStatus::Exited,
    ]);
}

#[given(expr = "a SubagentEntry with status {word}")]
fn given_entry_with_status(world: &mut QuectoWorld, status: String) {
    let s = match status.as_str() {
        "Starting" => SubagentStatus::Starting,
        "Idle" => SubagentStatus::Idle,
        "Running" => SubagentStatus::Running,
        "Error" => SubagentStatus::Error,
        "Exited" => SubagentStatus::Exited,
        _ => panic!("unknown status: {}", status),
    };
    let mut entry = SubagentEntry::new(std::path::PathBuf::from("/stub/test.sock"), 0);
    entry.status = s;
    world.monitor_entry = Some(entry);
}

#[given(expr = "a SubagentEntry with socket_path {string} and pid {int}")]
fn given_entry_with_socket_and_pid(world: &mut QuectoWorld, socket_path: String, pid: i32) {
    let entry = SubagentEntry::new(std::path::PathBuf::from(socket_path), pid as u32);
    world.monitor_entry = Some(entry);
}

#[given("a monitor abort handle")]
fn given_abort_handle(world: &mut QuectoWorld) {
    // Create a JoinHandle for a long-running task we can abort
    let rt = tokio::runtime::Runtime::new().unwrap();
    let handle = rt.spawn(async {
        tokio::time::sleep(std::time::Duration::from_secs(3600)).await;
    });
    world.monitor_abort_handle = Some(handle);
    world._monitor_rt = Some(rt);
}

// --- When ---

#[when(expr = "the monitor receives an {string} event")]
fn when_monitor_receives_event(world: &mut QuectoWorld, event_type: String) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let event_json = match event_type.as_str() {
        "agent_start" => r#"{"type":"agent_start"}"#.to_string(),
        "agent_end" => r#"{"type":"agent_end","messages":[]}"#.to_string(),
        _ => panic!("unknown event type: {}", event_type),
    };
    apply_event(entry, &event_json);
}

#[when(expr = "the monitor receives a {string} event with tool_name {string}")]
fn when_monitor_receives_tool_start(
    world: &mut QuectoWorld,
    event_type: String,
    tool_name: String,
) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let event_json = match event_type.as_str() {
        "tool_execution_start" => format!(
            r#"{{"type":"tool_execution_start","toolCallId":"c1","toolName":"{}","args":{{}}}}"#,
            tool_name
        ),
        _ => panic!("unexpected event type: {}", event_type),
    };
    apply_event(entry, &event_json);
}

#[when(expr = "the monitor receives a {string} event with is_error {word} and tool_name {string}")]
fn when_monitor_receives_tool_end(
    world: &mut QuectoWorld,
    _event_type: String,
    is_error: String,
    tool_name: String,
) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    let is_error_bool = is_error == "true";
    let event_json = format!(
        r#"{{"type":"tool_execution_end","toolCallId":"c1","toolName":"{}","result":{{"content":[]}},"isError":{}}}"#,
        tool_name, is_error_bool
    );
    apply_event(entry, &event_json);
}

#[when("the monitor detects connection closed")]
fn when_monitor_connection_closed(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    mark_exited(entry);
}

#[when(expr = "apply_event is called with {string}")]
fn when_apply_event_with_json(world: &mut QuectoWorld, json: String) {
    let entry = world.monitor_entry.as_mut().expect("no monitor entry");
    apply_event(entry, &json);
}

#[when("the abort handle is triggered")]
fn when_abort_handle_triggered(world: &mut QuectoWorld) {
    let handle = world.monitor_abort_handle.take().expect("no abort handle");
    handle.abort();
    let rt = world._monitor_rt.as_ref().expect("no runtime");
    let result = rt.block_on(handle);
    world.monitor_abort_result = Some(result.is_err()); // Should be JoinError (cancelled)
}

// --- Then ---

#[then(expr = "the subagent status should be {string}")]
fn then_subagent_status(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    let actual = format!("{}", entry.status);
    assert_eq!(
        actual, expected,
        "expected status '{}', got '{}'",
        expected, actual
    );
}

#[then("each variant should have a distinct display string")]
fn then_all_variants_distinct(world: &mut QuectoWorld) {
    let variants = world
        .monitor_status_variants
        .as_ref()
        .expect("no status variants");
    let displays: Vec<String> = variants.iter().map(|v| format!("{}", v)).collect();
    let unique: std::collections::HashSet<&String> = displays.iter().collect();
    assert_eq!(
        displays.len(),
        unique.len(),
        "expected all display strings to be unique, got: {:?}",
        displays
    );
}

#[then(expr = "the last_tool should be {string}")]
fn then_last_tool(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(
        entry.last_tool.as_deref(),
        Some(expected.as_str()),
        "expected last_tool '{}', got: {:?}",
        expected,
        entry.last_tool
    );
}

#[then("the last_tool should be None")]
fn then_last_tool_none(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert!(
        entry.last_tool.is_none(),
        "expected last_tool to be None, got: {:?}",
        entry.last_tool
    );
}

#[then(expr = "the last_error should contain {string}")]
fn then_last_error_contains(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    let err = entry.last_error.as_deref().expect("last_error is None");
    assert!(
        err.contains(&expected),
        "expected last_error to contain '{}', got: {}",
        expected,
        err
    );
}

#[then("the last_error should be None")]
fn then_last_error_none(world: &mut QuectoWorld) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert!(
        entry.last_error.is_none(),
        "expected last_error to be None, got: {:?}",
        entry.last_error
    );
}

#[then(expr = "the subagent entry should have socket_path {string}")]
fn then_entry_socket_path(world: &mut QuectoWorld, expected: String) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(
        entry.socket_path.to_str().unwrap(),
        expected,
        "expected socket_path '{}'",
        expected
    );
}

#[then(expr = "the subagent entry should have pid {int}")]
fn then_entry_pid(world: &mut QuectoWorld, expected: i32) {
    let entry = world.monitor_entry.as_ref().expect("no monitor entry");
    assert_eq!(entry.pid, expected as u32, "expected pid {}", expected);
}

#[then(expr = "the subagent registry entry {string} should have status {string}")]
fn then_registry_entry_status(world: &mut QuectoWorld, agent_id: String, expected: String) {
    let tool = world.spawn_tool.as_ref().expect("spawn_tool not set");
    let registry = tool.registry();
    let entries = registry.lock().unwrap();
    let entry = entries
        .get(&agent_id)
        .unwrap_or_else(|| panic!("agent '{}' not found in registry", agent_id));
    let actual = format!("{}", entry.status);
    assert_eq!(
        actual, expected,
        "expected registry entry '{}' status '{}', got '{}'",
        agent_id, expected, actual
    );
}

#[then("the monitor task should be cancelled")]
fn then_monitor_cancelled(world: &mut QuectoWorld) {
    let was_err = world.monitor_abort_result.expect("no abort result");
    assert!(
        was_err,
        "expected JoinHandle to return JoinError (cancelled)"
    );
}
