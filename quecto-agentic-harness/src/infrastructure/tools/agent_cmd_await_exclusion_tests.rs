//! Per-agent await mutual exclusion (#612): only one awaiter per agent at a
//! time. A second *concurrent* awaiter is rejected with `another_await_active`.
//! (Sibling tool calls in one turn serialize via the agent loop and so never
//! overlap — see `docs/docs-tool-embeds/subagents.md`.)

use super::*;

/// A second await while one is already active on the same agent is rejected
/// immediately with `another_await_active`.
#[tokio::test]
async fn test_await_duplicate_returns_error() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Running;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let active_awaits = new_active_awaits();
    // Pre-register an active await.
    active_awaits.lock().unwrap().insert("w1".to_string());

    let tool = AgentCmdTool::with_active_awaits(registry, active_awaits);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["reason"], "another_await_active");
    assert_eq!(parsed["elapsed_ms"], 0);
}

/// Two GENUINELY concurrent awaits on the same agent: exactly one wins and runs,
/// the other is rejected with `another_await_active`. The lock is per-agent and
/// shared (not per-call), so real overlap is mutually exclusive. (Sibling tool
/// calls in a single turn serialize and never overlap, so they don't hit this —
/// this drives the actual concurrency the lock guards.)
#[tokio::test]
async fn test_two_concurrent_awaits_one_is_rejected() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");
    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Running; // stays busy so the winner waits to its timeout
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::with_active_awaits(registry, new_active_awaits());

    // Both fired at once on the SAME agent: the first-polled acquires the per-agent
    // slot and runs to its short wall timeout; the other is rejected immediately.
    let (a, b) = tokio::join!(
        tool.execute(r#"{"agent_id":"w1","command":"await","timeout":1}"#),
        tool.execute(r#"{"agent_id":"w1","command":"await","timeout":1}"#),
    );
    let pa: serde_json::Value = serde_json::from_str(&a.unwrap().content).unwrap();
    let pb: serde_json::Value = serde_json::from_str(&b.unwrap().content).unwrap();

    let rejected = [pa["reason"].as_str(), pb["reason"].as_str()]
        .iter()
        .filter(|r| **r == Some("another_await_active"))
        .count();
    assert_eq!(
        rejected, 1,
        "exactly one concurrent awaiter must be rejected; got {pa:?} and {pb:?}"
    );
    assert!(
        pa["status"] == "timeout" || pb["status"] == "timeout",
        "the winning awaiter should have run to its timeout: {pa:?} {pb:?}"
    );
}
