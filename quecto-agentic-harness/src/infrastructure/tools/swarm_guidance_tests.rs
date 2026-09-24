use super::*;

#[test]
fn a_run_refused_during_setup_names_op_create_with_a_relative_deadline() {
    let message = run_refused(Some("setup"));
    assert!(message.contains(r#""op":"create""#), "{message}");
    assert!(
        message.contains("deadline_in_seconds"),
        "no stale absolute deadline: {message}"
    );
    assert!(message.contains("Allowed now: create"), "{message}");
    assert!(
        !message.contains("\"setup\""),
        "the status is not quoted: {message}"
    );
}

#[test]
fn a_run_refused_when_paused_names_every_allowed_op_and_the_supervisor() {
    let message = run_refused(Some("paused"));
    for op in ["summary", "events", "usage", "reconcile"] {
        assert!(message.contains(op), "{op}: {message}");
    }
    // A worker told to cancel_run would only be refused: it is coordinator-only.
    assert!(
        message.contains("the coordinator may also usage_budget and cancel_run"),
        "{message}"
    );
    assert!(message.contains("supervisor"), "{message}");
}

#[test]
fn an_ended_or_unreadable_run_says_what_to_do() {
    let ended = run_refused(Some("succeeded"));
    assert!(
        ended.contains("succeeded") && ended.contains("Allowed: summary"),
        "{ended}"
    );
    assert!(run_refused(None).contains("op=summary"));
    assert!(deadline_passed().contains("extend"));
}

#[test]
fn an_unknown_op_lists_the_valid_ones_and_how_to_end_a_run() {
    let message = unknown_op("stop");
    for op in VALID_OPS {
        assert!(message.contains(op), "{op}: {message}");
    }
    assert!(message.contains("board.stop"), "{message}");
}

#[test]
fn the_valid_ops_are_exactly_the_schema_enum() {
    // A drift guard: an op added to the schema (or the list) without the
    // other would make the unknown-op guidance lie.
    let schema: serde_json::Value =
        serde_json::from_str(include_str!("swarm_helpers/tool_schema.json")).unwrap();
    let mut schema_ops: Vec<&str> = schema["properties"]["op"]["enum"]
        .as_array()
        .unwrap()
        .iter()
        .map(|op| op.as_str().unwrap())
        .collect();
    let mut ops = VALID_OPS.to_vec();
    schema_ops.sort_unstable();
    ops.sort_unstable();
    assert_eq!(ops, schema_ops);
}

#[test]
fn the_tool_description_names_op_create_and_a_relative_deadline() {
    let description = include_str!("swarm_helpers/tool_description.txt");
    assert!(description.contains("op=create"), "no op=create");
    assert!(description.contains("deadline_in_seconds"));
}

#[tokio::test]
async fn op_run_before_create_points_the_founder_at_op_create() {
    // End to end through the tool: a bootstrapped, not yet created run.
    use crate::application::tools::ports::Tool;
    let directory = tempfile::tempdir().unwrap();
    let workspace = std::sync::Arc::new(directory.path().to_path_buf());
    std::fs::create_dir_all(workspace.join(".quecto")).unwrap();
    let context = crate::infrastructure::tools::swarm_bridge::SwarmContext {
        lifecycle: std::sync::Arc::new(crate::application::ports::SwarmTestLifecycle),
        checkout: workspace.as_ref().clone(),
        member: "coordinator".into(),
    };
    context
        .call(
            "_bootstrap",
            serde_json::json!([1, "start", "/tmp/unused.sock"]),
        )
        .unwrap();
    let tool = super::super::SwarmTool::new(
        workspace.clone(),
        std::sync::Arc::new(crate::infrastructure::security::sandbox::Sandbox::new(
            Some(workspace.as_ref().clone()),
        )),
        super::super::SwarmConfig::default(),
    )
    .with_context(Some(context));
    let result = tool
        .execute(r#"{"op":"run","code":"print('hello')"}"#)
        .await
        .unwrap();
    assert!(result.is_error);
    assert!(
        result.content.contains(r#""op":"create""#),
        "{}",
        result.content
    );
}
