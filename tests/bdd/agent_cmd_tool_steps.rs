use super::*;

// AgentCmdTool BDD Steps (#421)
// ===========================================================================

use quecto::infrastructure::tools::agent_cmd::AgentCmdTool;
use quecto::infrastructure::tools::subagent_registry::SubagentEntry;

// --- Given ---

#[given("an AgentCmdTool with an empty registry")]
fn given_agent_cmd_empty_registry(world: &mut QuectoWorld) {
    let registry = AgentCmdTool::new_registry();
    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
}

#[given(expr = "an AgentCmdTool with a mock registry entry {string}")]
fn given_agent_cmd_with_mock_entry(world: &mut QuectoWorld, agent_id: String) {
    // SubagentEntry imported at module top

    let registry = AgentCmdTool::new_registry();

    // Create a mock UDS server using std UnixListener so it persists
    // across async runtimes used by the When steps.
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("mock-agent.sock");

    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    // Bind using std, then spawn an acceptor thread.
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    std_listener.set_nonblocking(false).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        // Accept connections in a loop.
        for stream in std_listener.incoming() {
            match stream {
                Ok(mut stream) => {
                    let last_cmd_inner = last_cmd_clone.clone();
                    std::thread::spawn(move || {
                        use std::io::{BufRead, BufReader, Write};
                        let reader = BufReader::new(stream.try_clone().unwrap());
                        for line in reader.lines() {
                            let line = match line {
                                Ok(l) => l,
                                Err(_) => break,
                            };
                            // Echo the request `id` (and command type) in the
                            // response, mirroring the real child dispatch
                            // (`AgentEvent::ok(id, type_name, data)`), so the
                            // command reader can correlate the reply to its
                            // request and skip unsolicited responses (#831).
                            let parsed = serde_json::from_str::<serde_json::Value>(&line).ok();
                            let sent_type = parsed
                                .as_ref()
                                .and_then(|v| v.get("type").and_then(|t| t.as_str()))
                                .unwrap_or("mock")
                                .to_owned();
                            let sent_id = parsed
                                .as_ref()
                                .and_then(|v| v.get("id").and_then(|t| t.as_str()))
                                .unwrap_or("")
                                .to_owned();
                            *last_cmd_inner.lock().unwrap() = line;
                            let response = format!(
                                r#"{{"type":"response","id":"{}","command":"{}","success":true}}"#,
                                sent_id, sent_type
                            );
                            let _ = writeln!(stream, "{}", response);
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });

    registry
        .lock()
        .unwrap()
        .insert(agent_id, SubagentEntry::new(sock_path, 0));

    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
    // Keep tmp dir alive so socket file persists.
    world._agent_cmd_mock_tmp = Some(tmp);
    world.agent_cmd_last_command = Some(last_cmd);
}

#[given(expr = "an AgentCmdTool with a busy mock registry entry {string}")]
fn given_agent_cmd_with_busy_mock_entry(world: &mut QuectoWorld, agent_id: String) {
    // A BUSY child pushes an unsolicited connect-time `get_messages` SNAPSHOT as
    // the FIRST line on every new connection (#828). This mock reproduces that:
    // it writes the snapshot first, THEN echoes the real command (#831).
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));

    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    std_listener.set_nonblocking(false).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::{BufRead, BufReader, Write};
                // Connect-time snapshot pushed BEFORE the client's command reply.
                let snapshot = r#"{"type":"response","command":"get_messages","data":[{"role":"assistant","content":"FIRST MESSAGE ONLY"}]}"#;
                let _ = writeln!(stream, "{}", snapshot);
                let reader = BufReader::new(stream.try_clone().unwrap());
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    let parsed = serde_json::from_str::<serde_json::Value>(&line).ok();
                    let sent_type = parsed
                        .as_ref()
                        .and_then(|v| v.get("type").and_then(|t| t.as_str()))
                        .unwrap_or("mock")
                        .to_owned();
                    // Echo the stamped request id so the reader correlates the
                    // reply; the snapshot above carries no id and is skipped.
                    let sent_id = parsed
                        .as_ref()
                        .and_then(|v| v.get("id").and_then(|t| t.as_str()))
                        .unwrap_or("")
                        .to_owned();
                    *last_cmd_inner.lock().unwrap() = line;
                    let response = format!(
                        r#"{{"type":"response","id":"{}","command":"{}","data":[{{"role":"assistant","content":"LATEST TURNS"}}]}}"#,
                        sent_id, sent_type
                    );
                    let _ = writeln!(stream, "{}", response);
                }
            });
        }
    });

    registry
        .lock()
        .unwrap()
        .insert(agent_id, SubagentEntry::new(sock_path, 0));

    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
    world._agent_cmd_mock_tmp = Some(tmp);
    world.agent_cmd_last_command = Some(last_cmd);
}

// --- When ---

#[when(expr = "I execute agent_cmd with {string}")]
fn when_execute_agent_cmd(world: &mut QuectoWorld, arguments: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let rt = tokio::runtime::Runtime::new().unwrap();
    let result = rt.block_on(tool.execute(&arguments)).unwrap();
    world.agent_cmd_result = Some(result);
}

// --- Then ---

#[then(expr = "the agent_cmd tool definition name should be {string}")]
fn then_agent_cmd_def_name(world: &mut QuectoWorld, expected: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    assert_eq!(def.name, expected);
}

#[then("the agent_cmd tool definition description should not be empty")]
fn then_agent_cmd_def_desc_not_empty(world: &mut QuectoWorld) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    assert!(!def.description.is_empty());
}

#[then(expr = "the agent_cmd tool definition schema should require {string}")]
fn then_agent_cmd_schema_requires(world: &mut QuectoWorld, field: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let required = schema["required"].as_array().unwrap();
    assert!(
        required.iter().any(|v| v.as_str() == Some(&field)),
        "expected '{}' in required fields: {:?}",
        field,
        required
    );
}

#[then("the agent_cmd result should not be an error")]
fn then_agent_cmd_ok(world: &mut QuectoWorld) {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    assert!(
        !result.is_error,
        "expected success, got error: {}",
        result.content
    );
}

#[then("the agent_cmd result should be an error")]
fn then_agent_cmd_error(world: &mut QuectoWorld) {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    assert!(
        result.is_error,
        "expected error, got success: {}",
        result.content
    );
}

#[then(expr = "the agent_cmd result should contain {string}")]
fn then_agent_cmd_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    assert!(
        result.content.contains(&expected),
        "expected content to contain '{}', got: {}",
        expected,
        result.content
    );
}

#[then(expr = "the agent_cmd result should not contain {string}")]
fn then_agent_cmd_not_contains(world: &mut QuectoWorld, unexpected: String) {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    assert!(
        !result.content.contains(&unexpected),
        "expected content to NOT contain '{}', got: {}",
        unexpected,
        result.content
    );
}

#[then(expr = "the agent_cmd should have sent command type {string}")]
fn then_agent_cmd_sent_type(world: &mut QuectoWorld, expected_type: String) {
    // Give the mock server a moment to process.
    std::thread::sleep(std::time::Duration::from_millis(100));
    let last_cmd = world
        .agent_cmd_last_command
        .as_ref()
        .expect("no last command captured");
    let cmd_str = last_cmd.lock().unwrap().clone();
    assert!(
        !cmd_str.is_empty(),
        "no command was sent to mock UDS server"
    );
    let cmd: serde_json::Value = serde_json::from_str(&cmd_str)
        .unwrap_or_else(|e| panic!("invalid JSON sent to mock: {} — raw: {}", e, cmd_str));
    assert_eq!(
        cmd["type"].as_str(),
        Some(expected_type.as_str()),
        "expected command type '{}', got: {}",
        expected_type,
        cmd
    );
}

// ===========================================================================
