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
    let refused = tool
        .execute(r#"{"agent_id":"worker","task":"t"}"#)
        .await
        .expect_err("the tool refuses");
    assert_eq!(
        refused.to_string(),
        format!("tool error: {}", super::NO_LIFECYCLE_COMPOSED)
    );
    assert!(registry.lock().unwrap().is_empty());

    let stub = SpawnTool::new(vec![]);
    let result = stub
        .execute(r#"{"agent_id":"worker","task":"t"}"#)
        .await
        .expect("the stub answers");
    assert!(!result.is_error, "{}", result.content);
}

/// A frozen harness refuses a spawn as a tool error, stub and launch path
/// alike: every spawn refusal carries the one prefix (#2221).
#[tokio::test]
async fn a_frozen_harness_refuses_a_spawn_as_a_tool_error() {
    let lifecycle = super::super::harness_lifecycle::new_shared_harness_lifecycle();
    *lifecycle.lock().unwrap() =
        crate::domain::agents::services::subagent_teardown::HarnessLifecycleState::Frozen;
    let dir = tempfile::tempdir().expect("tempdir");
    let tools = [
        SpawnTool::new(vec![]).with_harness_lifecycle(lifecycle.clone()),
        SpawnTool::with_base_dir(vec![], dir.path().to_path_buf())
            .with_harness_lifecycle(lifecycle.clone()),
    ];
    for tool in tools {
        let refused = tool
            .execute(r#"{"agent_id":"worker","task":"t"}"#)
            .await
            .expect_err("a frozen harness admits no child");
        assert_eq!(
            refused.to_string(),
            "tool error: spawn refused: the harness is Frozen and admits no new subagent"
        );
        assert!(tool.registry().lock().unwrap().is_empty());
    }
}
