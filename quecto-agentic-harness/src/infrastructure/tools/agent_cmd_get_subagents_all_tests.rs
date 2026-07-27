use super::*;
use std::path::PathBuf;

#[test]
fn test_get_subagents_all_is_supported() {
    assert!(SUPPORTED_COMMANDS.contains(&"get_subagents_all"));
}

#[tokio::test]
async fn test_get_subagents_all_lists_parent_registry_without_agent_id() {
    let registry = new_registry();
    registry.lock().unwrap().insert(
        "w1".to_string(),
        SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 123),
    );
    let tool = AgentCmdTool::new(registry);
    let result = tool
        .execute(r#"{"command":"get_subagents_all"}"#)
        .await
        .unwrap();
    assert!(!result.is_error, "list should succeed: {}", result.content);
    let parsed: serde_json::Value = serde_json::from_str(&result.content).unwrap();
    assert_eq!(parsed["subagents"][0]["agentId"], "w1");
    assert_eq!(parsed["subagents"][0]["pid"], 123);
}
