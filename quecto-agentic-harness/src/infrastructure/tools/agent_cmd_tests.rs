use super::*;
use std::path::PathBuf;

fn empty_tool() -> AgentCmdTool {
    AgentCmdTool::new(new_registry())
}

#[test]
fn test_definition_name() {
    let tool = empty_tool();
    assert_eq!(tool.definition().name, "agent_cmd");
}

#[test]
fn test_definition_requires_agent_id_and_command() {
    let tool = empty_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let required = schema["required"].as_array().unwrap();
    assert!(required.iter().any(|v| v.as_str() == Some("agent_id")));
    assert!(required.iter().any(|v| v.as_str() == Some("command")));
}

#[test]
fn test_parse_missing_agent_id() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"command":"get_state"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("agent_id"));
}

#[test]
fn test_parse_missing_command() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("command"));
}

#[test]
fn test_parse_invalid_json() {
    let tool = empty_tool();
    let result = tool.parse_and_build("not json");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("invalid JSON"));
}

#[test]
fn test_parse_unsupported_command() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"delete_all"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("unsupported command"));
}

#[test]
fn test_parse_invalid_agent_id_format() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"bad id!","command":"get_state"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
}

#[test]
fn test_parse_get_state_builds_json() {
    let tool = empty_tool();
    let (agent_id, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_state"}"#)
        .unwrap();
    assert_eq!(agent_id, "w1");
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_state");
}

#[test]
fn test_parse_prompt_requires_message() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"prompt"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("message"));
}

#[test]
fn test_parse_prompt_with_message() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"prompt","message":"hello"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "prompt");
    assert_eq!(parsed["message"], "hello");
}

#[test]
fn test_parse_steer_requires_message() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"steer"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("message"));
}

#[test]
fn test_parse_get_messages_count_builds_tail_json() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages","count":5}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_messages");
    assert_eq!(parsed["count"], 5);
}

#[test]
fn test_parse_get_messages_before_cursor_reaches_child_command() {
    // Paged history (#1061): the tool must forward the paging cursor so a
    // caller can walk older pages beyond the newest bounded one.
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(
            r#"{"agent_id":"w1","command":"get_messages","before":"11111111-2222-3333-4444-555555555555"}"#,
        )
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_messages");
    assert_eq!(parsed["before"], "11111111-2222-3333-4444-555555555555");
    assert!(
        parsed.get("count").is_none(),
        "cursor-only paging must not invent a count: {parsed}"
    );

    // count combined with before forwards both.
    let (_, cmd, _) = tool
        .parse_and_build(
            r#"{"agent_id":"w1","command":"get_messages","count":4,"before":"cursor-id"}"#,
        )
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["count"], 4);
    assert_eq!(parsed["before"], "cursor-id");
}

#[test]
fn test_agent_cmd_schema_declares_before_cursor() {
    let def = empty_tool().definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert_eq!(schema["properties"]["before"]["type"], "string");
    let description = schema["properties"]["before"]["description"].as_str();
    assert!(
        description.is_some_and(|d| d.contains("older page")),
        "schema must explain the cursor pages older history: {schema}"
    );
}

#[test]
fn test_parse_get_messages_tail_aliases_to_get_messages_count() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages_tail","count":5}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_messages");
    assert_eq!(parsed["count"], 5);
}

#[test]
fn test_parse_abort() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"abort"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "abort");
}

#[test]
fn test_parse_get_session_stats() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_session_stats"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_session_stats");
}

#[test]
fn test_lookup_unknown_agent() {
    let tool = empty_tool();
    let result = tool.lookup_socket("nonexistent");
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("not found"));
}

#[test]
fn test_lookup_known_agent() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    let tool = AgentCmdTool::new(registry);
    let path = tool.lookup_socket("w1").unwrap();
    assert_eq!(path, PathBuf::from("/tmp/test.sock"));
}

#[tokio::test]
async fn test_execute_unknown_agent_returns_error() {
    let tool = empty_tool();
    let result = tool
        .execute(r#"{"agent_id":"nonexistent","command":"get_state"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_execute_invalid_json_returns_error() {
    let tool = empty_tool();
    let result = tool.execute("not json").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("invalid JSON"));
}

#[tokio::test]
async fn test_execute_missing_fields_returns_error() {
    let tool = empty_tool();
    let result = tool.execute(r#"{"agent_id":"w1"}"#).await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("command"));
}

#[tokio::test]
async fn test_execute_invalid_agent_id_format_returns_error() {
    let tool = empty_tool();
    let result = tool
        .execute(r#"{"agent_id":"bad id!","command":"get_state"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("[a-zA-Z0-9_-]"));
}

// ── New commands (#547) ──────────────────────────────────────────

#[test]
fn test_parse_follow_up_requires_message() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"follow_up"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("message"));
}

#[test]
fn test_parse_follow_up_with_message() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"follow_up","message":"After done"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "follow_up");
    assert_eq!(parsed["message"], "After done");
}

#[test]
fn test_parse_set_model_requires_model() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"set_model"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("model"));
}

#[test]
fn test_parse_set_model_with_model() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(
            r#"{"agent_id":"w1","command":"set_model","model":"anthropic/claude-sonnet-4-6"}"#,
        )
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "set_model");
    assert_eq!(parsed["model"], "anthropic/claude-sonnet-4-6");
}

#[test]
fn test_parse_set_model_with_provider_and_model_id() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"set_model","provider":"anthropic","model_id":"claude-sonnet-4-6"}"#)
            .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "set_model");
    assert_eq!(parsed["provider"], "anthropic");
    assert_eq!(parsed["modelId"], "claude-sonnet-4-6");
}

#[test]
fn test_parse_clear_history() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"clear_history"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "clear_history");
}

#[test]
fn test_parse_get_subagents() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_subagents"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_subagents");
}

#[test]
fn test_parse_get_extensions() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_extensions"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_extensions");
}

#[test]
fn test_parse_reload_extensions() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"reload_extensions"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "reload_extensions");
}

#[test]
fn test_parse_set_model_empty_string_rejected() {
    let tool = empty_tool();
    let result = tool.parse_and_build(r#"{"agent_id":"w1","command":"set_model","model":""}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("model"));
}

#[test]
fn test_parse_set_model_provider_without_model_id() {
    let tool = empty_tool();
    let result =
        tool.parse_and_build(r#"{"agent_id":"w1","command":"set_model","provider":"anthropic"}"#);
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("model_id"));
}

#[test]
fn test_parse_set_model_model_id_without_provider() {
    let tool = empty_tool();
    let result = tool.parse_and_build(
        r#"{"agent_id":"w1","command":"set_model","model_id":"claude-sonnet-4-6"}"#,
    );
    assert!(result.is_err());
    assert!(result.unwrap_err().contains("provider"));
}

#[test]
fn test_definition_lists_new_commands() {
    let tool = empty_tool();
    let def = tool.definition();
    assert!(def.description.contains("follow_up"));
    assert!(def.description.contains("set_model"));
    assert!(def.description.contains("clear_history"));
    assert!(def.description.contains("get_messages"));
    assert!(def.description.contains("get_subagents"));
    assert!(def.description.contains("kill"));
}

// ── Kill command tests (#559) ────────────────────────────────────

#[test]
fn test_parse_kill_command() {
    let _tool = empty_tool();
    // kill is handled locally — parse_and_build should not produce a JSON command.
    // Instead, execute() handles it directly.
    // Just verify it's in SUPPORTED_COMMANDS.
    assert!(SUPPORTED_COMMANDS.contains(&"kill"));
}

#[tokio::test]
async fn test_kill_unknown_agent_returns_error() {
    let tool = empty_tool();
    let result = tool
        .execute(r#"{"agent_id":"nonexistent","command":"kill"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("not found"));
}

#[tokio::test]
async fn test_kill_known_agent_removes_from_registry() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
    );
    let tool = AgentCmdTool::new(registry.clone());
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"kill"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "kill should succeed: {}", result.content);
    assert!(result.content.contains("killed"));
    assert!(
        registry.lock().unwrap().get("w1").is_none(),
        "agent should be removed from registry"
    );
}

// ── Await command tests (#612) ────────────────────────────────────

#[test]
fn test_await_is_in_supported_commands() {
    assert!(SUPPORTED_COMMANDS.contains(&"await"));
}

#[test]
fn test_is_await_command_true() {
    assert!(AgentCmdTool::is_await_command(
        r#"{"agent_id":"w1","command":"await"}"#
    ));
}

#[test]
fn test_is_await_command_false() {
    assert!(!AgentCmdTool::is_await_command(
        r#"{"agent_id":"w1","command":"get_state"}"#
    ));
}

#[test]
fn test_is_await_command_invalid_json() {
    assert!(!AgentCmdTool::is_await_command("not json"));
}

#[tokio::test]
async fn test_await_unknown_agent_returns_structured_error() {
    let tool = empty_tool();
    let result = tool
        .execute(r#"{"agent_id":"nonexistent","command":"await"}"#)
        .await
        .unwrap();
    assert!(
        !result.is_error,
        "await errors should be in the structured result, not tool errors"
    );
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["reason"], "agent_not_found");
    assert_eq!(parsed["agent_id"], "nonexistent");
    assert_eq!(parsed["elapsed_ms"], 0);
}

#[tokio::test]
async fn test_await_invalid_agent_id_format() {
    let tool = empty_tool();
    let result = tool
        .execute(r#"{"agent_id":"bad id!","command":"await"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("[a-zA-Z0-9_-]"));
}

#[tokio::test]
async fn test_await_idle_agent_with_zero_idle_timeout() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    // Create a mock server on the socket so connection check succeeds.
    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","idle_timeout":0}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "idle");
    assert_eq!(parsed["reason"], "idle");
    assert_eq!(parsed["agent_id"], "w1");
}

#[tokio::test]
async fn test_await_timeout_when_running() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Running;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":1}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "timeout");
    assert!(parsed["reason"].is_null());
}

// Awaiter mutual-exclusion tests live in `agent_cmd_await_exclusion_tests.rs`
// (kept separate to respect the per-file line cap).

#[tokio::test]
async fn test_await_stale_socket_returns_connection_failed() {
    let registry = new_registry();
    // Create a socket path that doesn't have a listener.
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("stale.sock");
    // Create the file so it "exists" but can't be connected to.
    std::fs::write(&sock_path, b"").unwrap();

    let entry = SubagentEntry::new(sock_path, 0);
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await"}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "error");
    assert_eq!(parsed["reason"], "connection_failed");
}

#[tokio::test]
async fn test_await_exited_agent_returns_immediately() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Exited;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "exited");
}

#[tokio::test]
async fn test_await_exit_signal_returns_exit_code() {
    use super::super::subagent_registry::{ExitSignal, SubagentStatus, new_exit_signal_channel};
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let (exit_tx, _exit_rx) = new_exit_signal_channel();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Running;
    entry.exit_signal_tx = Some(exit_tx.clone());
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);

    // Send exit signal after a short delay.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(100)).await;
        let _ = exit_tx.send(Some(ExitSignal {
            exit_code: Some(1),
            signal: None,
        }));
    });

    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","timeout":5}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "exited");
    assert_eq!(parsed["reason"], "exit_code_1");
}

#[tokio::test]
async fn test_await_guard_cleanup_on_completion() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let _ = tool
        .execute(r#"{"agent_id":"w1","command":"await","idle_timeout":0}"#)
        .await
        .unwrap();

    // After completion, the active_awaits should be cleaned up.
    let active = tool.active_awaits().lock().unwrap();
    assert!(
        !active.contains("w1"),
        "active_awaits should be cleaned up after completion"
    );
}

#[tokio::test]
async fn test_await_idle_timeout_waits_correct_duration() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let tool = AgentCmdTool::new(registry);
    let start = std::time::Instant::now();
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","idle_timeout":1}"#)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "idle");
    // Should have waited at least 1 second.
    assert!(
        elapsed >= Duration::from_millis(900),
        "expected at least ~1s wait, got {:?}",
        elapsed
    );
}

#[tokio::test]
async fn test_await_idle_resets_on_running() {
    use super::super::subagent_registry::SubagentStatus;
    let registry = new_registry();
    let dir = tempfile::tempdir().unwrap();
    let sock_path = dir.path().join("test.sock");

    let _listener = tokio::net::UnixListener::bind(&sock_path).unwrap();

    let mut entry = SubagentEntry::new(sock_path, 0);
    entry.status = SubagentStatus::Idle;
    registry.lock().unwrap().insert("w1".to_string(), entry);

    let registry_clone = registry.clone();
    // After 200ms, set status to Running, then after 400ms set back to Idle.
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(200)).await;
        {
            let mut entries = registry_clone.lock().unwrap();
            if let Some(e) = entries.get_mut("w1") {
                e.status = SubagentStatus::Running;
            }
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        {
            let mut entries = registry_clone.lock().unwrap();
            if let Some(e) = entries.get_mut("w1") {
                e.status = SubagentStatus::Idle;
            }
        }
    });

    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"await","idle_timeout":1,"timeout":10}"#)
        .await
        .unwrap();
    assert!(!result.is_error);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["status"], "idle");
    assert_eq!(parsed["reason"], "idle");
}

#[test]
fn test_definition_includes_await() {
    let tool = empty_tool();
    let def = tool.definition();
    assert!(def.description.contains("await"));
}

#[test]
fn test_definition_schema_includes_await() {
    let tool = empty_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    let command_enum = schema["properties"]["command"]["enum"].as_array().unwrap();
    assert!(
        command_enum.iter().any(|v| v.as_str() == Some("await")),
        "await should be in command enum"
    );
}

#[test]
fn test_definition_schema_includes_timeout_and_idle_timeout() {
    let tool = empty_tool();
    let def = tool.definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert!(
        schema["properties"]["timeout"].is_object(),
        "timeout should be in schema properties"
    );
    assert!(
        schema["properties"]["idle_timeout"].is_object(),
        "idle_timeout should be in schema properties"
    );
}

#[path = "agent_cmd_await_extra_tests.rs"]
mod await_extra_tests;

#[path = "agent_cmd_kill_tests.rs"]
mod kill_tests;
