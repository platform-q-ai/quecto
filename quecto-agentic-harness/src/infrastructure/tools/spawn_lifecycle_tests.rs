use super::super::spawn::SpawnTool;
use crate::application::tools::ports::Tool;

/// A tool nobody composed refuses a real launch before any process exists
/// and registers nothing; the stub path (empty base dir) launches nothing
/// and is not refused.
#[tokio::test]
async fn uncomposed_tool_refuses_a_real_launch_and_registers_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let registry = super::super::subagent_registry::new_registry();
    let tool = SpawnTool::with_base_dir(vec![], dir.path().to_path_buf())
        .with_socket_dir(dir.path().to_path_buf())
        .with_registry(registry.clone());
    assert!(!tool.lifecycle_composed());
    let result = tool
        .execute(r#"{"agent_id":"worker","task":"t"}"#)
        .await
        .expect("the tool answers");
    assert!(result.is_error, "{}", result.content);
    assert_eq!(
        result.content,
        format!("Failed to spawn subagent: {}", super::NO_LIFECYCLE_COMPOSED)
    );
    assert!(registry.lock().unwrap().is_empty());

    let stub = SpawnTool::new(vec![]);
    let result = stub
        .execute(r#"{"agent_id":"worker","task":"t"}"#)
        .await
        .expect("the stub answers");
    assert!(!result.is_error, "{}", result.content);
}
