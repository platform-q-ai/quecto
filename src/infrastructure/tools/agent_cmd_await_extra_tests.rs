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

// ── UDS transport tests (#557) ───────────────────────────────────

/// Mock UDS server that replicates the multi-client broadcast pattern:
/// - Reader task: reads commands until EOF, then aborts writer task
/// - Writer task: sends noise then response via a channel
///
/// This reproduces the real bug: if the client shuts down the write
/// half, the reader sees EOF immediately → aborts writer → response
/// is never delivered.
async fn mock_uds_server(listener: tokio::net::UnixListener) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let (stream, _) = listener.accept().await.unwrap();
    let (reader, mut writer) = tokio::io::split(stream);
    let mut lines = BufReader::new(reader).lines();

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
    let cmd_line = lines.next_line().await.unwrap();
    if cmd_line.is_some() {
        // Dispatch: send noise + response via channel.
        let _ = tx
            .send("{\"type\":\"token\",\"token\":\"hello\"}\n".into())
            .await;
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _ = tx
                .send("{\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{\"isStreaming\":false}}\n".into())
                .await;
    }

    // Wait for reader EOF (client closed write half → next_line returns None).
    while lines.next_line().await.unwrap_or(None).is_some() {}

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
        use tokio::io::AsyncBufReadExt;
        let (stream, _) = listener.accept().await.unwrap();
        let mut lines = tokio::io::BufReader::new(stream).lines();
        let _ = lines.next_line().await; // read command
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
