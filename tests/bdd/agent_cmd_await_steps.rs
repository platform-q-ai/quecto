use super::*;

// agent_cmd await BDD Steps (#612)
// ===========================================================================

use quecto::domain::audit::AuditEvent;
use quecto::infrastructure::tools::agent_cmd::{AgentCmdTool, new_active_awaits};
use quecto::infrastructure::tools::subagent_registry::{
    SubagentEntry, SubagentStatus, new_registry,
};

// --- Given ---

#[given("an AgentCmdTool with a mock await registry")]
fn given_agent_cmd_await_registry(world: &mut QuectoWorld) {
    let registry = new_registry();
    let active_awaits = new_active_awaits();
    world.agent_cmd_tool = Some(AgentCmdTool::with_active_awaits(
        registry.clone(),
        active_awaits.clone(),
    ));
    world.agent_cmd_registry = Some(registry.clone());
    world.await_registry = Some(registry);
    world.await_active_awaits = Some(active_awaits);
    // Clear mock state from any prior scenario.
    world._await_mock_tmp = None;
    world._await_mock_listener = None;
    world.await_result = None;
}

#[given(expr = "the mock subagent {string} has status {string}")]
fn given_mock_subagent_status(world: &mut QuectoWorld, agent_id: String, status: String) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set");

    let tmp = world
        ._await_mock_tmp
        .get_or_insert_with(|| tempfile::TempDir::new().unwrap());
    let sock_path = tmp.path().join(format!("{agent_id}.sock"));

    // Create a real std UnixListener so the socket file exists and is connectable.
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    world._await_mock_listener = Some(listener);

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = match status.as_str() {
        "idle" => SubagentStatus::Idle,
        "running" => SubagentStatus::Running,
        "starting" => SubagentStatus::Starting,
        "error" => SubagentStatus::Error,
        "exited" => SubagentStatus::Exited,
        other => panic!("unknown status: {other}"),
    };
    registry.lock().unwrap().insert(agent_id, entry);
}

#[given(expr = "the mock subagent {string} has run error {string}")]
fn given_mock_subagent_run_error(world: &mut QuectoWorld, agent_id: String, error: String) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set");
    let mut entries = registry.lock().unwrap();
    let entry = entries
        .get_mut(&agent_id)
        .unwrap_or_else(|| panic!("no mock subagent {agent_id}"));
    // Set ONLY run_error (the run-level failure #752 surfaces); leave
    // last_error unset so the scenario pins behaviour to the correct field
    // and cannot pass off a recoverable tool error.
    entry.run_error = Some(error);
    entry.last_error = None;
}

#[given(expr = "the mock subagent {string} will exit with code {int} after {int}ms")]
fn given_mock_subagent_will_exit(
    world: &mut QuectoWorld,
    agent_id: String,
    exit_code: i32,
    delay_ms: i32,
) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set")
        .clone();

    let (exit_tx, _exit_rx) =
        quecto::infrastructure::tools::subagent_registry::new_exit_signal_channel();

    // Store the exit_tx in the registry entry.
    {
        let mut entries = registry.lock().unwrap();
        if let Some(entry) = entries.get_mut(&agent_id) {
            entry.exit_signal_tx = Some(exit_tx.clone());
        }
    }

    // Send the exit signal after the delay, then set status to Exited.
    // We send the signal and set the status atomically (while holding the
    // mutex) so the await loop always sees both together.
    let reg_clone = registry.clone();
    let agent_id_clone = agent_id.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        // Hold the mutex while sending the signal AND updating the status
        // to prevent the await loop from seeing Exited before the signal.
        let mut entries = reg_clone.lock().unwrap();
        // Send exit signal while holding the lock.
        let _ = exit_tx.send(Some(
            quecto::infrastructure::tools::subagent_registry::ExitSignal {
                exit_code: Some(exit_code),
                signal: None,
            },
        ));
        // Set status to Exited while still holding the lock.
        if let Some(entry) = entries.get_mut(&agent_id_clone) {
            entry.status = SubagentStatus::Exited;
        }
    });
}

#[given(
    expr = "the mock subagent {string} will go idle then resume after {int}ms then idle permanently"
)]
fn given_mock_subagent_idle_resume(world: &mut QuectoWorld, agent_id: String, delay_ms: i32) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set")
        .clone();

    // Start running, go idle briefly, resume running, then go idle permanently.
    let agent_id_clone = agent_id.clone();
    std::thread::spawn(move || {
        // Set to idle.
        {
            let mut entries = registry.lock().unwrap();
            if let Some(entry) = entries.get_mut(&agent_id_clone) {
                entry.status = SubagentStatus::Idle;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        // Resume running.
        {
            let mut entries = registry.lock().unwrap();
            if let Some(entry) = entries.get_mut(&agent_id_clone) {
                entry.status = SubagentStatus::Running;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(delay_ms as u64));
        // Go idle permanently.
        {
            let mut entries = registry.lock().unwrap();
            if let Some(entry) = entries.get_mut(&agent_id_clone) {
                entry.status = SubagentStatus::Idle;
            }
        }
    });
}

#[given(expr = "another await is already active for {string}")]
fn given_another_await_active(world: &mut QuectoWorld, agent_id: String) {
    let active_awaits = world
        .await_active_awaits
        .as_ref()
        .expect("await_active_awaits not set");
    active_awaits.lock().unwrap().insert(agent_id);
}

#[given(expr = "the mock subagent {string} has a stale socket")]
fn given_mock_subagent_stale_socket(world: &mut QuectoWorld, agent_id: String) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set");

    let tmp = world
        ._await_mock_tmp
        .get_or_insert_with(|| tempfile::TempDir::new().unwrap());
    let sock_path = tmp.path().join(format!("{agent_id}-stale.sock"));

    // Create a regular file (not a socket) to simulate a stale socket.
    std::fs::write(&sock_path, b"stale").unwrap();

    let entry = SubagentEntry::new(sock_path, 0);
    registry.lock().unwrap().insert(agent_id, entry);
}

#[given(expr = "the mock subagent {string} has workflow state complete with {int} of {int} steps")]
fn given_mock_subagent_workflow(
    world: &mut QuectoWorld,
    agent_id: String,
    completed: i32,
    total: i32,
) {
    let registry = world
        .await_registry
        .as_ref()
        .expect("await_registry not set");

    // Replace the socket with a mock UDS server that responds to get_state
    // with workflow data.
    let tmp = world
        ._await_mock_tmp
        .get_or_insert_with(|| tempfile::TempDir::new().unwrap());
    let sock_path = tmp.path().join(format!("{agent_id}-wf.sock"));

    // Remove old socket if exists.
    let _ = std::fs::remove_file(&sock_path);

    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let completed_clone = completed;
    let total_clone = total;

    std::thread::spawn(move || {
        // Accept connections in a loop.
        for stream in std_listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let c = completed_clone;
                    let t = total_clone;
                    std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader, Write};
                        let reader = BufReader::new(stream.try_clone().unwrap());
                        for line in reader.lines() {
                            let line = match line {
                                Ok(l) => l,
                                Err(_) => break,
                            };
                            // Echo the stamped request id so the command reader
                            // correlates the reply to its request (#831).
                            let sent_id = serde_json::from_str::<serde_json::Value>(&line)
                                .ok()
                                .and_then(|v| {
                                    v.get("id").and_then(|t| t.as_str()).map(str::to_owned)
                                })
                                .unwrap_or_default();
                            let mode = if c >= t { "complete" } else { "active" };
                            // Match the real get_state shape: a nested progress object.
                            let response = format!(
                                r#"{{"type":"response","id":"{}","command":"get_state","success":true,"data":{{"isStreaming":false,"workflow":{{"mode":"{}","progress":{{"done":{},"total":{}}}}}}}}}"#,
                                sent_id, mode, c, t
                            );
                            let _ = writeln!(stream, "{}", response);
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });

    // Update the registry entry to use the new socket.
    {
        let mut entries = registry.lock().unwrap();
        if let Some(entry) = entries.get_mut(&agent_id) {
            entry.socket_path = sock_path;
        }
    }
}

#[given(
    expr = "a SubagentAwait audit event with agent_id {string} status {string} reason {string} elapsed_ms {int}"
)]
fn given_subagent_await_audit_event(
    world: &mut QuectoWorld,
    agent_id: String,
    status: String,
    reason: String,
    elapsed_ms: i32,
) {
    world.audit_event = Some(AuditEvent::SubagentAwait {
        agent_id,
        status,
        reason: Some(reason),
        elapsed_ms: elapsed_ms as u64,
    });
}

// --- When ---

// The "When I execute agent_cmd with ..." step from agent_cmd_tool_steps.rs
// is reused. We parse the result as JSON for await-specific assertions.

#[when("I serialize and deserialize the audit event")]
fn when_serde_audit_event(world: &mut QuectoWorld) {
    let event = world.audit_event.as_ref().expect("audit_event not set");
    let json = serde_json::to_string(event).unwrap();
    world.audit_json = Some(json);
}

// --- Then ---

#[then("the agent_cmd result should not be a tool error")]
fn then_agent_cmd_not_tool_error(world: &mut QuectoWorld) {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    assert!(
        !result.is_error,
        "expected non-error tool result, got error: {}",
        result.content
    );
    // Parse the JSON content for await result assertions.
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap_or_else(|e| {
        panic!(
            "result content is not valid JSON: {} — content: {}",
            e, result.content
        )
    });
    world.await_result = Some(parsed);
}

#[then(expr = "the agent_cmd await result status should be {string}")]
fn then_await_result_status(world: &mut QuectoWorld, expected: String) {
    // Ensure we have the parsed result.
    if world.await_result.is_none() {
        let result = world
            .agent_cmd_result
            .as_ref()
            .expect("no agent_cmd result");
        let parsed: serde_json::Value = serde_json::from_str(&result.content)
            .unwrap_or_else(|e| panic!("result is not JSON: {} — content: {}", e, result.content));
        world.await_result = Some(parsed);
    }
    let await_result = world.await_result.as_ref().unwrap();
    assert_eq!(
        await_result["status"].as_str(),
        Some(expected.as_str()),
        "expected status '{}', got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result reason should be {string}")]
fn then_await_result_reason(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["reason"].as_str(),
        Some(expected.as_str()),
        "expected reason '{}', got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result error should be {string}")]
fn then_await_result_error(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["error"].as_str(),
        Some(expected.as_str()),
        "expected error '{}', got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result summary should contain {string}")]
fn then_await_result_summary_contains(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    let summary = await_result["result"]["summary"]
        .as_str()
        .unwrap_or_else(|| panic!("no result.summary, got: {await_result}"));
    assert!(
        summary.contains(&expected),
        "expected summary to contain '{expected}', got: '{summary}'"
    );
}

#[then("the agent_cmd await result reason should be null")]
fn then_await_result_reason_null(world: &mut QuectoWorld) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert!(
        await_result["reason"].is_null(),
        "expected null reason, got: {}",
        await_result["reason"]
    );
}

#[then(expr = "the agent_cmd await result agent_id should be {string}")]
fn then_await_result_agent_id(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["agent_id"].as_str(),
        Some(expected.as_str()),
        "expected agent_id '{}', got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result elapsed_ms should be {int}")]
fn then_await_result_elapsed_exact(world: &mut QuectoWorld, expected: i32) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["elapsed_ms"].as_u64(),
        Some(expected as u64),
        "expected elapsed_ms {}, got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result elapsed_ms should be at least {int}")]
fn then_await_result_elapsed_at_least(world: &mut QuectoWorld, min_ms: i32) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    let elapsed = await_result["elapsed_ms"]
        .as_u64()
        .expect("elapsed_ms is not a number");
    assert!(
        elapsed >= min_ms as u64,
        "expected elapsed_ms >= {}, got: {}",
        min_ms,
        elapsed
    );
}

#[then(expr = "the agent_cmd await result workflow mode should be {string}")]
fn then_await_result_workflow_mode(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["workflow"]["mode"].as_str(),
        Some(expected.as_str()),
        "expected workflow mode '{}', got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result verdict should be {string}")]
fn then_await_result_verdict(world: &mut QuectoWorld, expected: String) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["result"]["status"].as_str(),
        Some(expected.as_str()),
        "expected verdict '{}', got: {}",
        expected,
        await_result["result"]
    );
}

#[then(expr = "the agent_cmd await result workflow steps_completed should be {int}")]
fn then_await_result_workflow_steps_completed(world: &mut QuectoWorld, expected: i32) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["workflow"]["steps_completed"].as_u64(),
        Some(expected as u64),
        "expected steps_completed {}, got: {}",
        expected,
        await_result
    );
}

#[then(expr = "the agent_cmd await result workflow steps_total should be {int}")]
fn then_await_result_workflow_steps_total(world: &mut QuectoWorld, expected: i32) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert_eq!(
        await_result["workflow"]["steps_total"].as_u64(),
        Some(expected as u64),
        "expected steps_total {}, got: {}",
        expected,
        await_result
    );
}

#[then("the agent_cmd await result workflow should be null")]
fn then_await_result_workflow_null(world: &mut QuectoWorld) {
    let await_result = world.await_result.as_ref().expect("no await_result");
    assert!(
        await_result["workflow"].is_null(),
        "expected null workflow, got: {}",
        await_result["workflow"]
    );
}

#[then(expr = "the agent_cmd tool definition description should contain {string}")]
fn then_agent_cmd_desc_contains(world: &mut QuectoWorld, expected: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    assert!(
        def.description.contains(&expected),
        "expected description to contain '{}', got: {}",
        expected,
        def.description
    );
}

#[then(expr = "the agent_cmd tool definition schema should include {string} in command enum")]
fn then_agent_cmd_schema_includes_command(world: &mut QuectoWorld, expected: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let command_enum = schema["properties"]["command"]["enum"]
        .as_array()
        .expect("command enum not found");
    assert!(
        command_enum.iter().any(|v| v.as_str() == Some(&expected)),
        "expected '{}' in command enum: {:?}",
        expected,
        command_enum
    );
}

#[then(expr = "the agent_cmd tool definition schema should include property {string}")]
fn then_agent_cmd_schema_includes_property(world: &mut QuectoWorld, expected: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert!(
        schema["properties"][&expected].is_object(),
        "expected property '{}' in schema, got: {}",
        expected,
        schema["properties"]
    );
}

#[then("the deserialized audit event should match the original")]
fn then_audit_event_matches(world: &mut QuectoWorld) {
    let original = world.audit_event.as_ref().expect("audit_event not set");
    let json = world.audit_json.as_ref().expect("audit_json not set");
    let back: AuditEvent = serde_json::from_str(json).unwrap();
    assert_eq!(
        *original, back,
        "round-trip failed: original={:?}, deserialized={:?}",
        original, back
    );
}

// ===========================================================================
