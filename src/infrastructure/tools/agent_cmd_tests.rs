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
    let (agent_id, cmd) = tool
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
    let (_, cmd) = tool
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
fn test_parse_get_messages_tail_default_count() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages_tail"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_messages_tail");
    assert_eq!(parsed["count"], 1);
}

#[test]
fn test_parse_get_messages_tail_custom_count() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages_tail","count":5}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["count"], 5);
}

#[test]
fn test_parse_abort() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"abort"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "abort");
}

#[test]
fn test_parse_get_session_stats() {
    let tool = empty_tool();
    let (_, cmd) = tool
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
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"follow_up","message":"After done"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "follow_up");
    assert_eq!(parsed["message"], "After done");
}

#[test]
fn test_parse_get_messages() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_messages"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_messages");
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
    let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"set_model","model":"anthropic/claude-sonnet-4-20250514"}"#)
            .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "set_model");
    assert_eq!(parsed["model"], "anthropic/claude-sonnet-4-20250514");
}

#[test]
fn test_parse_set_model_with_provider_and_model_id() {
    let tool = empty_tool();
    let (_, cmd) = tool
            .parse_and_build(r#"{"agent_id":"w1","command":"set_model","provider":"anthropic","model_id":"claude-sonnet-4-20250514"}"#)
            .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "set_model");
    assert_eq!(parsed["provider"], "anthropic");
    assert_eq!(parsed["modelId"], "claude-sonnet-4-20250514");
}

#[test]
fn test_parse_clear_history() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"clear_history"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "clear_history");
}

#[test]
fn test_parse_get_subagents() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_subagents"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_subagents");
}

#[test]
fn test_parse_get_extensions() {
    let tool = empty_tool();
    let (_, cmd) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_extensions"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_extensions");
}

#[test]
fn test_parse_reload_extensions() {
    let tool = empty_tool();
    let (_, cmd) = tool
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
        r#"{"agent_id":"w1","command":"set_model","model_id":"claude-sonnet-4-20250514"}"#,
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
