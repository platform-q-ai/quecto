//! #876: parent-side non-blocking forward of control commands.
//!
//! A parent's `agent_cmd` prompt/steer/follow_up/abort must return on the
//! child's ACCEPTANCE ack within the short interactive timeout — never frozen
//! for the child's full turn (the 300s turn-completion deadline). These tests
//! cover the parent half: the `"ack":"accept"` marker is stamped on control
//! commands (only), and a child that acks acceptance promptly (but never sends a
//! turn-completion response) unblocks the parent fast.

use super::*;
use std::io::{BufRead, BufReader, Write};

fn parsed(tool: &AgentCmdTool, args: &str) -> serde_json::Value {
    let (_, cmd, _) = tool.parse_and_build(args).unwrap();
    serde_json::from_str(&cmd).unwrap()
}

#[test]
fn control_commands_carry_accept_marker() {
    let tool = AgentCmdTool::new(new_registry());
    for (args, ty) in [
        (
            r#"{"agent_id":"w1","command":"prompt","message":"m"}"#,
            "prompt",
        ),
        (
            r#"{"agent_id":"w1","command":"steer","message":"m"}"#,
            "steer",
        ),
        (
            r#"{"agent_id":"w1","command":"follow_up","message":"m"}"#,
            "follow_up",
        ),
        (r#"{"agent_id":"w1","command":"abort"}"#, "abort"),
    ] {
        let v = parsed(&tool, args);
        assert_eq!(v["type"], ty);
        assert_eq!(
            v["ack"], "accept",
            "{ty} must carry the acceptance marker (#876)"
        );
    }
}

#[test]
fn non_control_commands_do_not_carry_accept_marker() {
    let tool = AgentCmdTool::new(new_registry());
    for args in [
        r#"{"agent_id":"w1","command":"get_state"}"#,
        r#"{"agent_id":"w1","command":"get_messages"}"#,
        r#"{"agent_id":"w1","command":"get_session_stats"}"#,
        r#"{"agent_id":"w1","command":"clear_history"}"#,
    ] {
        let v = parsed(&tool, args);
        assert!(
            v.get("ack").is_none(),
            "read/query commands must NOT carry the marker: {args}"
        );
    }
}

#[test]
fn is_control_command_matches_only_the_four() {
    for c in ["prompt", "steer", "follow_up", "abort"] {
        assert!(AgentCmdTool::is_control_command(c), "{c} is control");
    }
    for c in ["get_state", "get_messages", "await", "kill", "set_model"] {
        assert!(!AgentCmdTool::is_control_command(c), "{c} is not control");
    }
}

/// Mock child whose reader acks ACCEPTANCE for any `"ack":"accept"` command
/// immediately (id-correlated) and NEVER sends a turn-completion response — the
/// production behaviour of a busy child's reader (#876). The connection is held
/// open, so a regression that waited for completion would ride the 300s deadline
/// rather than return.
fn busy_fast_ack_child(agent_id: &str) -> (AgentCmdTool, SubagentRegistry, tempfile::TempDir) {
    let registry = new_registry();
    let tmp = tempfile::TempDir::new().unwrap();
    let sock_path = tmp.path().join("fast-ack.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock_path).unwrap();
    listener.set_nonblocking(false).unwrap();

    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { break };
            std::thread::spawn(move || {
                // Busy child pushes a connect-time get_messages snapshot first
                // (no id) — the parent must skip it and match the real ack.
                let _ = writeln!(
                    stream,
                    r#"{{"type":"response","command":"get_messages","data":{{"messages":[]}}}}"#
                );
                let reader = BufReader::new(stream.try_clone().unwrap());
                for line in reader.lines() {
                    let Ok(line) = line else { break };
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
                    // Deliberately send NOTHING else: no turn-completion event.
                }
            });
        }
    });

    registry
        .lock()
        .unwrap()
        .insert(agent_id.to_string(), SubagentEntry::new(sock_path, 0));
    let tool = AgentCmdTool::new(registry.clone());
    (tool, registry, tmp)
}

async fn run_with_overall_cap(tool: &AgentCmdTool, args: &str) -> ToolResult {
    // 30s overall cap: far above the 5s interactive timeout but far below the
    // 300s turn-completion deadline, so a blocking-on-completion regression
    // fails here instead of hanging the suite.
    tokio::time::timeout(std::time::Duration::from_secs(30), tool.execute(args))
        .await
        .expect("agent_cmd must return well within 30s, never the 300s deadline")
        .expect("execute returns Ok")
}

#[tokio::test]
async fn busy_child_prompt_returns_on_acceptance() {
    let (tool, _reg, _tmp) = busy_fast_ack_child("busy-prompt");
    let result = run_with_overall_cap(
        &tool,
        r#"{"agent_id":"busy-prompt","command":"prompt","message":"do work"}"#,
    )
    .await;
    assert!(!result.is_error, "got: {}", result.content);
    assert!(
        result.content.contains("\"success\":true"),
        "acceptance ack expected, got: {}",
        result.content
    );
}

#[tokio::test]
async fn busy_child_steer_abort_follow_up_return_promptly() {
    for (cmd, body) in [
        ("steer", r#","message":"turn""#),
        ("follow_up", r#","message":"next""#),
        ("abort", ""),
    ] {
        let id = format!("busy-{cmd}");
        let (tool, _reg, _tmp) = busy_fast_ack_child(&id);
        let args = format!(r#"{{"agent_id":"{id}","command":"{cmd}"{body}}}"#);
        let result = run_with_overall_cap(&tool, &args).await;
        assert!(!result.is_error, "{cmd} got error: {}", result.content);
        assert!(
            result.content.contains("\"success\":true"),
            "{cmd} acceptance ack expected, got: {}",
            result.content
        );
    }
}
