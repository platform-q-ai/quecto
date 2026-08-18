use super::*;

#[test]
fn tool_policy_command_and_catalogue_events_round_trip() {
    let cmd = Command::SetToolPolicy {
        id: Some("p1".into()),
        mutations: vec![ToolPolicyMutation {
            tool_id: None,
            name: Some("alpha".into()),
            scope: ToolScope::Child,
            reason: Some("test".into()),
        }],
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        operation: ToolPolicyOperation::Patch,
        unlisted_scope: None,
        persist: false,
    };
    let json = serde_json::to_string(&cmd).unwrap();
    let value: serde_json::Value = serde_json::from_str(&json).unwrap();
    assert_eq!(value["type"], "set_tool_policy");
    assert_eq!(value["scope"], serde_json::Value::Null);
    assert_eq!(value["mutations"][0]["scope"], "child");
    assert_eq!(value["operation"], "patch");
    assert!(value.get("unlistedScope").is_none());

    let wire = r#"{"type":"tool_catalogue_changed","changedTools":["alpha"],"before":[],"after":[{"stableId":"s1","name":"alpha","profileScope":"child","effectiveScope":"child","effectiveParentEnabled":false,"effectiveChildEnabled":true}],"reason":"policy"}"#;
    let event: Event = serde_json::from_str(wire).unwrap();
    match event {
        Event::ToolCatalogueChanged { after, .. } => {
            assert_eq!(after[0].stable_id, "s1");
            assert_eq!(after[0].profile_scope, Some(ToolScope::Child));
            assert_eq!(after[0].effective_scope, Some(ToolScope::Child));
            assert_eq!(after[0].effective_parent_enabled, Some(false));
            assert_eq!(after[0].effective_child_enabled, Some(true));
        }
        other => panic!("expected ToolCatalogueChanged, got {other:?}"),
    }

    let policy = r#"{"type":"tool_policy_changed","changedTools":["alpha"],"results":[{"after":{"stableId":"s1","name":"alpha","effectiveScope":"child"}}],"applyMode":"immediateIfIdle","reason":"policy"}"#;
    assert!(matches!(
        serde_json::from_str::<Event>(policy).unwrap(),
        Event::ToolPolicyChanged { .. }
    ));
}
