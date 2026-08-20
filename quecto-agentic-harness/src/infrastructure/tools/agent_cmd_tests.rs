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
fn test_agent_cmd_schema_declares_before_and_since_cursors() {
    let def = empty_tool().definition();
    let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
    assert_eq!(schema["properties"]["before"]["type"], "string");
    let before_description = schema["properties"]["before"]["description"].as_str();
    assert!(
        before_description.is_some_and(|d| d.contains("older page")),
        "schema must explain the cursor pages older history: {schema}"
    );
    assert_eq!(schema["properties"]["since"]["type"], "integer");
    let since_description = schema["properties"]["since"]["description"].as_str();
    assert!(
        since_description.is_some_and(|d| d.contains("get_state") && d.contains("unchanged")),
        "schema must explain get_state since cursor: {schema}"
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
fn test_parse_get_tool_catalogue() {
    let tool = empty_tool();
    let (_, cmd, _) = tool
        .parse_and_build(r#"{"agent_id":"w1","command":"get_tool_catalogue"}"#)
        .unwrap();
    let parsed: serde_json::Value = serde_json::from_str(&cmd).unwrap();
    assert_eq!(parsed["type"], "get_tool_catalogue");
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
    assert!(def.description.contains("get_subagents_all"));
    assert!(def.description.contains("kill"));
}

// ── Kill command tests (#559) ────────────────────────────────────

#[test]
fn agent_cmd_preserves_stateless_spawn_policy_defaults() {
    use crate::domain::tool_descriptor::ProfileAvailabilityScope;
    use std::collections::BTreeMap;

    let tool = empty_tool();
    let result = ToolResult {
        content: "delivered".into(),
        is_error: false,
        image_blocks: vec![],
    };
    <AgentCmdTool as Tool>::result_delivered(&tool, r#"{"command":"get_state"}"#, &result);
    assert_eq!(
        <AgentCmdTool as Tool>::inherited_child_policy_snapshot_for_spawn(&tool),
        None
    );

    let mut snapshot = BTreeMap::new();
    snapshot.insert("bash".to_string(), ProfileAvailabilityScope::Both);
    <AgentCmdTool as Tool>::set_inherited_child_policy_snapshot_for_spawn(&tool, snapshot);
    assert_eq!(
        <AgentCmdTool as Tool>::inherited_child_policy_snapshot_for_spawn(&tool),
        None
    );
}

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

#[path = "agent_cmd_kill_tests.rs"]
mod kill_tests;
