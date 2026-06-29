// Additional region-coverage tests for the `await` command logic (#612).
//
// These cover the deterministic, no-server branches of `execute_await` and
// `fetch_workflow_snapshot` that are reachable without standing up a live UDS
// server:
//   - malformed JSON when `execute_await` is invoked directly,
//   - the "socket path exists but is not a real socket" connection-failed path,
//   - the workflow-snapshot fetch giving up when the agent is unknown or the
//     socket cannot deliver a response.
//
// The interior await loop (idle/exit/timeout/error state transitions) and the
// successful workflow-snapshot decode require a live UDS server that streams a
// JSON-lines `response` event; those are intentionally left to the existing
// socket-based integration tests, per the no-real-socket constraint.

use super::*;

/// `execute_await` is normally guarded by `is_await_command`, so its own
/// malformed-JSON arm is only reachable by calling it directly.
#[tokio::test]
async fn execute_await_direct_invalid_json_is_structured_error() {
    let tool = AgentCmdTool::new(new_registry());
    let result = tool.execute_await("{not valid json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON"));
}

/// A registry entry whose socket path exists but is a regular file: the
/// synchronous connectivity probe runs (path exists) and fails, so `await`
/// reports `connection_failed` while the agent is still registered.
#[tokio::test]
async fn execute_await_existing_nonsocket_path_is_connection_failed() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_a_socket");
    std::fs::write(&path, b"x").unwrap();
    registry
        .lock()
        .unwrap()
        .insert("w1".to_string(), SubagentEntry::new(path, 0));

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute_await(r#"{"agent_id":"w1","command":"await","timeout":99999}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let v: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(v["status"], "error");
    assert_eq!(v["reason"], "connection_failed");
    assert_eq!(v["result"]["status"], "failed");
}

/// Workflow-snapshot fetch returns `None` when the agent is not in the registry
/// (socket lookup fails).
#[tokio::test]
async fn fetch_workflow_snapshot_unknown_agent_is_none() {
    let tool = AgentCmdTool::new(new_registry());
    assert!(tool.fetch_workflow_snapshot("ghost").await.is_none());
}

/// Workflow-snapshot fetch returns `None` when the socket cannot deliver a
/// response (the path is a regular file, so the UDS command errors out).
#[tokio::test]
async fn fetch_workflow_snapshot_unconnectable_socket_is_none() {
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("not_a_socket");
    std::fs::write(&path, b"x").unwrap();
    registry
        .lock()
        .unwrap()
        .insert("w1".to_string(), SubagentEntry::new(path, 0));

    let tool = AgentCmdTool::new(registry);
    assert!(tool.fetch_workflow_snapshot("w1").await.is_none());
}
