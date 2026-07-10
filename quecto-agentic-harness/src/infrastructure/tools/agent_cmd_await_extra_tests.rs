use super::*;

#[tokio::test]
async fn test_await_removed_agent_returns_exited() {
    use crate::infrastructure::tools::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Running;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let registry_clone = registry.clone();
    // After 200ms, remove the agent from the registry (simulating reaper).
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        let mut entries = registry_clone.lock().unwrap();
        entries.remove("w1");
    });

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "exited");
}

// ── Await result serde tests (#612) ──────────────────────────────

#[test]
fn test_await_result_serialization() {
    let result = AwaitResult::new(
        "idle",
        Some("idle"),
        "w1".into(),
        5000,
        Some(WorkflowSnapshot {
            mode: "complete".into(),
            steps_completed: 7,
            steps_total: 7,
        }),
    );
    let json = serde_json::to_string(&result).unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed["status"], "idle");
    assert_eq!(parsed["reason"], "idle");
    assert_eq!(parsed["agent_id"], "w1");
    assert_eq!(parsed["elapsed_ms"], 5000);
    assert_eq!(parsed["workflow"]["mode"], "complete");
    assert_eq!(parsed["workflow"]["steps_completed"], 7);
    assert_eq!(parsed["workflow"]["steps_total"], 7);
    // Typed verdict: a complete workflow at idle is "completed".
    assert_eq!(parsed["result"]["status"], "completed");
    assert_eq!(parsed["result"]["workflow_progress"]["done"], 7);
    assert_eq!(parsed["result"]["workflow_progress"]["total"], 7);
}

#[test]
fn test_await_result_round_trip() {
    let result = AwaitResult::new("timeout", None, "bot-1".into(), 120000, None);
    let json = serde_json::to_string(&result).unwrap();
    let back: AwaitResult = serde_json::from_str(&json).unwrap();
    assert_eq!(result, back);
}

#[test]
fn timeout_with_in_progress_workflow_reads_as_still_running_checkin() {
    // #925: timing out mid-workflow (3/4) must read as a CHECK-IN on a child
    // that is STILL RUNNING — not Incomplete (gave up) and not Failed (error).
    // The verdict and summary (a) name the progress, (b) say "still running",
    // (c) tell the parent the next action, and (d) say it is NOT an error. The
    // tool result itself stays non-error and serializes the timeout lifecycle.
    use crate::domain::workflow::VerdictStatus;
    let wf = WorkflowSnapshot {
        mode: "active".into(),
        steps_completed: 3,
        steps_total: 4,
    };
    let r = AwaitResult::new("timeout", None, "bot-1".into(), 90000, Some(wf));
    assert_eq!(r.status, "timeout");
    assert_eq!(r.result.status, VerdictStatus::Running);
    assert_ne!(r.result.status, VerdictStatus::Incomplete);
    assert_ne!(r.result.status, VerdictStatus::Failed);
    let progress = r.result.workflow_progress.as_ref().unwrap();
    assert_eq!((progress.done, progress.total), (3, 4));
    let s = r.result.summary.to_lowercase();
    assert!(
        s.contains("3/4"),
        "summary must name progress: {}",
        r.result.summary
    );
    assert!(s.contains("still running"), "summary: {}", r.result.summary);
    assert!(s.contains("not an error"), "summary: {}", r.result.summary);
    assert!(
        s.contains("re-await") || s.contains("steer") || s.contains("wait"),
        "summary must name next action: {}",
        r.result.summary
    );
}

// ── UDS transport tests (#557) ───────────────────────────────────

/// Mock UDS server that replicates the multi-client broadcast pattern:
/// - Reader task: reads commands until EOF, then aborts writer task
/// - Writer task: sends noise then response via a channel
///
/// This reproduces the real bug: if the client shuts down the write
/// half, the reader sees EOF immediately → aborts writer → response
/// is never delivered.
async fn mock_uds_server(listener: tokio::net::UnixListener) {
    use tokio::io::{AsyncWriteExt, BufReader};
    let (stream, _) = listener.accept().await.unwrap();
    let (reader, mut writer) = tokio::io::split(stream);
    let mut reader = BufReader::new(reader);

    // Channel simulates the broadcast: dispatch sends events here,
    // writer task drains them to the client.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(16);

    // Writer task: drain channel → client socket.
    let writer_task = tokio::spawn(async move {
        while let Some(line) = rx.recv().await {
            if writer.write_all(line.as_bytes()).await.is_err() {
                break;
            }
        }
    });

    // Reader loop: read commands until EOF, then abort writer.
    // This matches uds_multi.rs handle_client_task behaviour.
    let cmd_line =
        crate::infrastructure::test_support::read_framed_command_async(&mut reader).await;
    if let Some(cmd) = cmd_line {
        // Echo the stamped request id so the command reader correlates the
        // reply to its request (#831).
        let id = serde_json::from_str::<serde_json::Value>(&cmd)
            .ok()
            .and_then(|v| v.get("id").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_default();
        // Dispatch: send noise + response via channel.
        let _ = tx
            .send("{\"type\":\"token\",\"token\":\"hello\"}\n".into())
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _ = tx
                .send(format!("{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"isStreaming\":false}}}}\n"))
                .await;
    }

    // Wait for reader EOF after any additional framed commands.
    while crate::infrastructure::test_support::read_framed_command_async(&mut reader)
        .await
        .is_some()
    {}

    // Real server aborts writer when reader exits.
    writer_task.abort();
}

#[tokio::test]
async fn test_send_uds_command_skips_noise_finds_response() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let server = tokio::spawn(mock_uds_server(listener));

    let result = send_uds_command(&sock_path, r#"{"type":"get_state"}"#)
        .await
        .unwrap();

    let parsed: serde_json::Value = serde_json::from_str(&result).unwrap();
    assert_eq!(parsed["type"], "response");
    assert_eq!(parsed["command"], "get_state");
    assert_eq!(parsed["success"], true);

    server.await.unwrap();
}

#[tokio::test]
async fn test_send_uds_command_eof_without_response_errors() {
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    // Server that closes immediately without sending response.
    let server = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let mut reader = tokio::io::BufReader::new(stream);
        let _ = crate::infrastructure::test_support::read_framed_command_async(&mut reader).await;
        // Close without sending response.
    });

    let result = send_uds_command(&sock_path, r#"{"type":"get_state"}"#).await;
    assert!(result.is_err());
    assert!(
        result
            .unwrap_err()
            .to_string()
            .contains("closed connection")
    );

    server.await.unwrap();
}

#[tokio::test]
async fn execute_await_unknown_agent_is_structured_error() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool
        .execute(r#"{"agent_id":"nope","command":"await","idle_timeout":0}"#)
        .await
        .unwrap();
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "error");
    assert_eq!(v["reason"], "agent_not_found");
    assert_eq!(v["result"]["status"], "failed");
}

#[tokio::test]
async fn execute_await_missing_agent_id_is_error() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool.execute(r#"{"command":"await"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("agent_id"));
}

#[tokio::test]
async fn execute_await_invalid_json_is_error() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool.execute(r#"{"command":"await""#).await.unwrap();
    assert!(result.is_error);
}

async fn serve_get_state_once(listener: tokio::net::UnixListener) {
    use tokio::io::{AsyncWriteExt, BufReader};
    // One connection is used by the await preflight connect check and then dropped.
    let _ = listener.accept().await;
    if let Ok((stream, _)) = listener.accept().await {
        let (reader, mut writer) = tokio::io::split(stream);
        let mut reader = BufReader::new(reader);
        let cmd = crate::infrastructure::test_support::read_framed_command_async(&mut reader)
            .await
            .unwrap_or_default();
        let id = serde_json::from_str::<serde_json::Value>(&cmd)
            .ok()
            .and_then(|v| v.get("id").and_then(|t| t.as_str()).map(str::to_owned))
            .unwrap_or_default();
        let _ = writer
            .write_all(
                format!("{{\"type\":\"response\",\"id\":\"{id}\",\"command\":\"get_state\",\"success\":true,\"data\":{{}}}}\n").as_bytes(),
            )
            .await;
    }
}

#[tokio::test]
async fn execute_await_run_error_returns_structured_error() {
    use crate::infrastructure::tools::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("run-error.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(serve_get_state_once(listener));

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Error;
    entry.last_error = Some("provider rejected model".to_string());
    entry.run_error = Some("provider rejected model".to_string());
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5,"idle_timeout":0}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["reason"], "agent_error");
    assert_eq!(parsed["result"]["status"], "failed");
}

#[tokio::test]
async fn execute_await_run_error_surfaces_error_message() {
    // #752: the actual run error message/cause must be visible in the await
    // response, not just a generic "agent_error" reason.
    use crate::infrastructure::tools::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("run-error-surface.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(serve_get_state_once(listener));

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Error;
    // Set ONLY run_error (leave last_error None) so the test pins the behaviour
    // to the run-level field #752 cares about, not a recoverable tool error.
    entry.last_error = None;
    entry.run_error = Some("HTTP 429 from Codex: usage_limit_reached".to_string());
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5,"idle_timeout":0}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["reason"], "agent_error");
    // The actual error text must be present in a dedicated `error` field.
    assert_eq!(parsed["error"], "HTTP 429 from Codex: usage_limit_reached");
    // ...and reflected in the human-readable verdict summary.
    let summary = parsed["result"]["summary"].as_str().unwrap();
    assert!(
        summary.contains("usage_limit_reached"),
        "summary should carry the cause, got: {summary}"
    );
}

#[tokio::test]
async fn execute_await_tool_error_preserves_existing_idle_behavior() {
    use crate::infrastructure::tools::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("tool-error.sock");
    let listener = tokio::net::UnixListener::bind(&sock_path).unwrap();
    tokio::spawn(serve_get_state_once(listener));

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Error;
    entry.last_error = Some("tool 'bash' returned error".to_string());
    entry.run_error = None;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5,"idle_timeout":0}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "idle");
    assert_eq!(parsed["reason"], "idle");
}
