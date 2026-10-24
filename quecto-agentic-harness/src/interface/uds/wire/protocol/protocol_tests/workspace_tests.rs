use super::*;

#[test]
fn unit_tree_parent_of_unknown_agent_is_none() {
    let tree = UnitTree::from_events(&[serde_json::json!({"agent_id":"root","parent_id":null})]);
    assert_eq!(tree.parent_of("nope"), None);
    assert_eq!(tree.parent_of("root"), None);
}

#[test]
fn workspace_event_serializes_1350() {
    let event = AgentEvent::Workspace {
        path: "/tmp/ws".into(),
    };
    let json = event.to_json_line();
    assert!(json.contains("\"type\":\"workspace\""), "{json}");
    assert!(json.contains("\"path\":\"/tmp/ws\""), "{json}");
}
