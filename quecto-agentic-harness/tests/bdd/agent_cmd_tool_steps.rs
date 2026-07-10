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
                        use std::io::Write;
                        while let Some(line) =
                            quecto::infrastructure::test_support::read_framed_command(&stream)
                        {
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
                use std::io::Write;
                // Connect-time snapshot pushed BEFORE the client's command reply.
                let snapshot = r#"{"type":"response","command":"get_messages","data":[{"role":"assistant","content":"FIRST MESSAGE ONLY"}]}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
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

#[given(expr = "an AgentCmdTool with a busy snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_snapshot_entry(world: &mut QuectoWorld, agent_id: String) {
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-snapshot-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_messages","data":{"messages":[{"role":"assistant","content":"FIRST MESSAGE ONLY"}]}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(expr = "an AgentCmdTool with a busy multi-message snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_multi_snapshot_entry(world: &mut QuectoWorld, agent_id: String) {
    // Like the busy snapshot entry, but the connect-time snapshot carries MORE
    // than one message and the child NEVER sends an id-matched reply (it is
    // mid-turn). A counted/tail get_messages must therefore be served from the
    // snapshot, with the parent applying `count` locally (#842).
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-multi-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_messages","data":{"messages":[{"role":"user","content":"OLDEST MESSAGE"},{"role":"assistant","content":"NEWEST MESSAGE"}],"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(expr = "an AgentCmdTool with a fast-ack busy registry entry {string}")]
fn given_agent_cmd_with_fast_ack_busy_entry(world: &mut QuectoWorld, agent_id: String) {
    // Simulates a BUSY child running the #876/#880 reader: it pushes the
    // connect-time get_messages snapshot (no id), then acks ACCEPTANCE of any
    // `"ack":"accept"` agent_cmd forward immediately (id-correlated) but NEVER
    // sends a turn-completion response and holds the connection open. A parent
    // that (wrongly) waited for completion would ride the 300s deadline; the
    // scenario completing proves it returned on acceptance.
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("fast-ack-busy.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    std_listener.set_nonblocking(false).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot =
                    r#"{"type":"response","command":"get_messages","data":{"messages":[]}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line.clone();
                    let v: serde_json::Value = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if v.get("ack").and_then(|a| a.as_str()) != Some("accept") {
                        continue;
                    }
                    let id = v.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let ty = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
                    let _ = writeln!(
                        stream,
                        r#"{{"type":"response","id":"{id}","command":"{ty}","success":true}}"#
                    );
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

#[given(expr = "an AgentCmdTool with a busy state snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_state_snapshot_entry(world: &mut QuectoWorld, agent_id: String) {
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-state-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_state","data":{"isStreaming":true,"messageCount":2,"model":"mock"}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(expr = "an AgentCmdTool with a busy subagents snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_subagents_snapshot_entry(world: &mut QuectoWorld, agent_id: String) {
    // A BUSY child pushes a connect-time `get_subagents` SNAPSHOT as the FIRST
    // line on every new connection (#874, mirroring #842's get_messages/get_state
    // busy-serve). The mock NEVER sends an id-matched reply (it is mid-turn), so
    // a blocking-on-reply regression would ride the ~300s deadline — completing
    // proves the snapshot was accepted and served promptly.
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-subagents-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_subagents","data":{"subagents":[{"agentId":"grandchild-worker","status":"running","pid":4321}],"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(expr = "an AgentCmdTool with a busy subagents snapshot and echo registry entry {string}")]
fn given_agent_cmd_with_busy_subagents_snapshot_and_echo_entry(
    world: &mut QuectoWorld,
    agent_id: String,
) {
    // A BUSY child pushes a connect-time `get_subagents` SNAPSHOT (#874) AND
    // echoes the real command with an id-matched reply. A DIFFERENT command
    // (get_session_stats) must SKIP the get_subagents snapshot and accept only
    // its own correlated reply — preserving the #835 id-correlation guarantee.
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-subagents-skip-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_subagents","data":{"subagents":[{"agentId":"grandchild-worker","status":"running","pid":4321}],"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
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

#[given(expr = "an AgentCmdTool with a busy session stats snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_session_stats_snapshot_entry(
    world: &mut QuectoWorld,
    agent_id: String,
) {
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-stats-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_session_stats","data":{"sessionKey":"cli:busy-stats","userMessages":1,"assistantMessages":1,"totalMessages":2,"toolCalls":0,"toolResults":0,"promptTokens":0,"completionTokens":0,"totalTokens":0,"contextTokens":0,"maxContextTokens":0,"costUsd":0.0,"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(expr = "an AgentCmdTool with a busy extensions snapshot registry entry {string}")]
fn given_agent_cmd_with_busy_extensions_snapshot_entry(world: &mut QuectoWorld, agent_id: String) {
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-extensions-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let snapshot = r#"{"type":"response","command":"get_extensions","data":{"extensions":[{"name":"mock_ext_tool","description":"mock extension"}],"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", snapshot);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
                    *last_cmd_inner.lock().unwrap() = line;
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

#[given(
    expr = "an AgentCmdTool with busy stats and extensions snapshots plus echo registry entry {string}"
)]
fn given_agent_cmd_with_busy_remaining_snapshots_and_echo_entry(
    world: &mut QuectoWorld,
    agent_id: String,
) {
    let registry = AgentCmdTool::new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("busy-remaining-skip-agent.sock");
    let last_cmd: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let std_listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    let last_cmd_clone = last_cmd.clone();

    let _handle = std::thread::spawn(move || {
        for stream in std_listener.incoming() {
            let Ok(mut stream) = stream else { break };
            let last_cmd_inner = last_cmd_clone.clone();
            std::thread::spawn(move || {
                use std::io::Write;
                let stats = r#"{"type":"response","command":"get_session_stats","data":{"sessionKey":"cli:busy-stats","userMessages":1,"assistantMessages":1,"totalMessages":2,"snapshot":true}}"#;
                let exts = r#"{"type":"response","command":"get_extensions","data":{"extensions":[{"name":"mock_ext_tool"}],"snapshot":true}}"#;
                let _ = writeln!(stream, "{}", stats);
                let _ = writeln!(stream, "{}", exts);
                while let Some(line) =
                    quecto::infrastructure::test_support::read_framed_command(&stream)
                {
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
                        r#"{{"type":"response","id":"{}","command":"{}","data":{{"subagents":[{{"agentId":"grandchild-worker","status":"running","pid":4321}}]}}}}"#,
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
    let result = rt
        .block_on(async {
            tokio::time::timeout(std::time::Duration::from_secs(5), tool.execute(&arguments)).await
        })
        .expect("agent_cmd should return promptly")
        .unwrap();
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

#[then(expr = "the agent_cmd tool definition description should not contain {string}")]
fn then_agent_cmd_def_desc_not_contains(world: &mut QuectoWorld, unexpected: String) {
    let tool = world
        .agent_cmd_tool
        .as_ref()
        .expect("agent_cmd_tool not set");
    let def = tool.definition();
    assert!(
        !def.description.contains(&unexpected),
        "expected description not to contain '{unexpected}', got: {}",
        def.description
    );
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

fn last_agent_cmd_sent_json(world: &QuectoWorld) -> serde_json::Value {
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
    serde_json::from_str(&cmd_str)
        .unwrap_or_else(|e| panic!("invalid JSON sent to mock: {} — raw: {}", e, cmd_str))
}

#[then(expr = "the agent_cmd should have sent command type {string}")]
fn then_agent_cmd_sent_type(world: &mut QuectoWorld, expected_type: String) {
    let cmd = last_agent_cmd_sent_json(world);
    assert_eq!(
        cmd["type"].as_str(),
        Some(expected_type.as_str()),
        "expected command type '{}', got: {}",
        expected_type,
        cmd
    );
}

#[then(expr = "the agent_cmd should have sent ack {string}")]
fn then_agent_cmd_sent_ack(world: &mut QuectoWorld, expected_ack: String) {
    let cmd = last_agent_cmd_sent_json(world);
    assert_eq!(
        cmd["ack"].as_str(),
        Some(expected_ack.as_str()),
        "expected ack '{}', got: {}",
        expected_ack,
        cmd
    );
}

// ===========================================================================

#[then(expr = "the agent_cmd should have sent count {int}")]
fn then_agent_cmd_sent_count(world: &mut QuectoWorld, expected: u64) {
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
    assert_eq!(cmd["count"].as_u64(), Some(expected), "sent command: {cmd}");
}

fn parse_agent_cmd_response(world: &QuectoWorld, command: &str) -> serde_json::Value {
    let result = world
        .agent_cmd_result
        .as_ref()
        .expect("no agent_cmd result");
    let json: serde_json::Value = serde_json::from_str(result.content.trim())
        .unwrap_or_else(|e| panic!("agent_cmd result is not JSON: {e}; raw: {}", result.content));
    assert_eq!(json["type"].as_str(), Some("response"), "response: {json}");
    assert_eq!(json["command"].as_str(), Some(command), "response: {json}");
    json
}

#[then(expr = "the agent_cmd response command {string} should include a {string} array")]
fn then_agent_cmd_response_array(world: &mut QuectoWorld, command: String, field: String) {
    let json = parse_agent_cmd_response(world, &command);
    assert!(
        json["data"][&field].is_array(),
        "expected data.{field} array in response: {json}"
    );
}

#[then(expr = "the agent_cmd response command {string} should include boolean field {string}")]
fn then_agent_cmd_response_bool(world: &mut QuectoWorld, command: String, field: String) {
    let json = parse_agent_cmd_response(world, &command);
    assert!(
        json["data"][&field].is_boolean(),
        "expected data.{field} boolean in response: {json}"
    );
}

#[then(
    expr = "the agent_cmd response command {string} should include boolean field {string} set to {string}"
)]
fn then_agent_cmd_response_bool_value(
    world: &mut QuectoWorld,
    command: String,
    field: String,
    expected: String,
) {
    let json = parse_agent_cmd_response(world, &command);
    let want = expected == "true";
    assert_eq!(
        json["data"][&field].as_bool(),
        Some(want),
        "expected data.{field} == {want} in response: {json}"
    );
}

#[then(expr = "the agent_cmd response command {string} should include integer field {string}")]
fn then_agent_cmd_response_integer(world: &mut QuectoWorld, command: String, field: String) {
    let json = parse_agent_cmd_response(world, &command);
    assert!(
        json["data"][&field].as_i64().is_some(),
        "expected data.{field} integer in response: {json}"
    );
}
