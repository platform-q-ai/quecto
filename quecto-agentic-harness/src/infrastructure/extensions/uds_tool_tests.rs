use super::*;

fn test_def(name: &str) -> ToolDefinition {
    ToolDefinition {
        name: name.to_string().into(),
        description: "Test tool".into(),
        parameters_schema: r#"{"type":"object"}"#.into(),
    }
}

#[test]
fn extension_tool_preserves_stateless_delivery_and_spawn_policy_contracts() {
    use crate::domain::tool_descriptor::ProfileAvailabilityScope;
    use std::collections::BTreeMap;

    let (tool, _rx) = create_uds_tool(test_def("weather"), std::time::Duration::from_secs(5));
    let result = ToolResult {
        content: "delivered".into(),
        is_error: false,
        image_blocks: vec![],
        delivery_metadata: None,
    };
    tool.result_delivered("{}", &result);
    assert_eq!(tool.inherited_child_policy_snapshot_for_spawn(), None);

    let mut snapshot = BTreeMap::new();
    snapshot.insert("bash".to_string(), ProfileAvailabilityScope::Both);
    tool.set_inherited_child_policy_snapshot_for_spawn(snapshot);
    assert_eq!(tool.inherited_child_policy_snapshot_for_spawn(), None);
}

#[tokio::test]
async fn test_execute_returns_result_from_extension() {
    let (tool, mut rx) = create_uds_tool(test_def("weather"), std::time::Duration::from_secs(5));

    // Debug must render the definition but not the channel internals.
    let concrete = UdsExtensionTool::new(
        test_def("weather"),
        tokio::sync::mpsc::channel(1).0,
        std::time::Duration::from_secs(5),
    );
    let dbg = format!("{concrete:?}");
    assert!(dbg.contains("UdsExtensionTool") && dbg.contains("weather"));

    // Simulate extension responding.
    let handle = tokio::spawn(async move {
        let req = rx.recv().await.unwrap();
        assert_eq!(req.tool_name, "weather");
        let _ = req.reply.send(ToolResult {
            content: "22°C, sunny".into(),
            is_error: false,
            image_blocks: vec![],
            delivery_metadata: None,
        });
    });

    let result = tool.execute(r#"{"city":"London"}"#).await.unwrap();
    assert_eq!(result.content, "22°C, sunny");
    assert!(!result.is_error);
    handle.await.unwrap();
}

#[tokio::test]
async fn test_execute_timeout_returns_error() {
    let (tool, _rx) = create_uds_tool(test_def("slow"), std::time::Duration::from_millis(50));

    // Don't respond — let it timeout.
    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("timed out"));
}

#[tokio::test]
async fn test_execute_disconnected_sender_returns_error() {
    let (tool, rx) = create_uds_tool(test_def("gone"), std::time::Duration::from_secs(5));

    // Drop the receiver to simulate disconnect.
    drop(rx);

    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("disconnected"));
}

#[tokio::test]
async fn test_execute_receiver_dropped_during_wait() {
    let (tool, mut rx) = create_uds_tool(test_def("drop_mid"), std::time::Duration::from_secs(5));

    // Receive the request but drop the result_tx without sending.
    let handle = tokio::spawn(async move {
        let req = rx.recv().await.unwrap();
        drop(req.reply);
    });

    let result = tool.execute("{}").await.unwrap();
    assert!(result.is_error);
    assert!(result.content.contains("disconnected"));
    handle.await.unwrap();
}

#[test]
fn test_definition_returns_correct_name() {
    let (tool, _rx) = create_uds_tool(test_def("weather"), std::time::Duration::from_secs(5));
    assert_eq!(tool.definition().name.as_ref(), "weather");
}

#[test]
fn test_uuid_v4_uniqueness() {
    let a = uuid_v4();
    let b = uuid_v4();
    assert_ne!(a, b);
}
