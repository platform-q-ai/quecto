use super::*;
use serde_json::json;

#[test]
fn tool_catalogue_changed_event_serializes_additive_shape() {
    let event = AgentEvent::ToolCatalogueChanged {
        changed_tools: vec!["alpha".to_string()],
        before: vec![json!({"name":"alpha","effectiveEnabled":true})],
        after: vec![json!({"name":"alpha","effectiveEnabled":false})],
        reason: "reload".to_string(),
    };
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["type"], "tool_catalogue_changed");
    assert_eq!(value["changedTools"], json!(["alpha"]));
    assert_eq!(value["before"][0]["effectiveEnabled"], true);
    assert_eq!(value["after"][0]["effectiveEnabled"], false);
    assert_eq!(value["reason"], "reload");
}

#[test]
fn tool_policy_changed_event_serializes_apply_mode_and_results() {
    let event = AgentEvent::ToolPolicyChanged {
        changed_tools: vec!["alpha".to_string()],
        results: vec![
            json!({"name":"alpha","status":"applied","after":{"name":"alpha","profileScope":"child","effectiveScope":"child","effectiveParentEnabled":false,"effectiveChildEnabled":true}}),
        ],
        child_propagation: vec![json!({"agentId":"child-1","status":"queued"})],
        apply_mode: "atNextTurnBoundary".to_string(),
        reason: "queued".to_string(),
    };
    let value = serde_json::to_value(event).unwrap();
    assert_eq!(value["type"], "tool_policy_changed");
    assert_eq!(value["changedTools"], json!(["alpha"]));
    assert_eq!(value["applyMode"], "atNextTurnBoundary");
    assert_eq!(value["results"][0]["status"], "applied");
    assert_eq!(value["results"][0]["after"]["profileScope"], "child");
    assert_eq!(value["results"][0]["after"]["effectiveScope"], "child");
    assert_eq!(value["childPropagation"][0]["status"], "queued");
    assert_eq!(value["reason"], "queued");
}
