use super::*;
use std::path::PathBuf;

#[test]
fn test_get_subagents_all_is_supported() {
    assert!(SUPPORTED_COMMANDS.contains(&"get_subagents_all"));
}

#[tokio::test]
async fn test_get_subagents_all_lists_parent_registry_with_star_agent_id() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 123),
    );
    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"*","command":"get_subagents_all"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "list should succeed: {}", result.content);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["subagents"][0]["agentId"], "w1");
    assert_eq!(parsed["subagents"][0]["status"], "running");
    assert!(
        !parsed["subagents"][0]
            .as_object()
            .unwrap()
            .contains_key("pid")
    );
}

#[tokio::test]
async fn get_subagents_all_rejects_non_star_agent_id_before_local_dispatch() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 123),
    );
    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"agent_id":"w1","command":"get_subagents_all"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("requires agent_id \"*\""));
}
