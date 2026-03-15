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
                            *last_cmd_inner.lock().unwrap() = line;
                            let response = r#"{"type":"response","command":"mock","success":true}"#;
                            let _ = writeln!(stream, "{}", response);
                        }
                    });
                }
                Err(_) => break,
            }
        }
    });

    registry.lock().unwrap().insert(
        agent_id,
        SubagentEntry {
            socket_path: sock_path,
            pid: 0,
        },
    );

    world.agent_cmd_tool = Some(AgentCmdTool::new(registry.clone()));
    world.agent_cmd_registry = Some(registry);
    // Keep tmp dir alive so socket file persists.
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
