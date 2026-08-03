use super::subagent_policy_gateway::*;
use crate::domain::error::DomainError;
use crate::domain::tool::{
    ChildToolPolicyPropagationStatus, ToolPolicyApplyMode, ToolPolicyChildPropagator,
    ToolPolicyMutation,
};
use crate::infrastructure::tools::subagent_registry::{SubagentEntry, SubagentStatus};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

#[tokio::test]
async fn synchronous_gateway_is_safe_inside_active_tokio_runtime() {
    let gateway = SubagentPolicyGateway::new(None);
    let result = gateway.propagate_tool_policy_to_children(
        &[ToolPolicyMutation::disable("bash", "inside async runtime")],
        ToolPolicyApplyMode::AtNextTurnBoundary,
    );

    assert!(result.is_empty());
}

#[test]
fn gateway_has_children_reflects_non_exited_registry_entries() {
    let empty_gateway = SubagentPolicyGateway::new(None);
    assert!(!empty_gateway.has_children());

    let registry = Arc::new(Mutex::new(HashMap::from([("child-1".to_string(), {
        let mut entry = SubagentEntry::new(PathBuf::from("/tmp/idle.sock"), 0);
        entry.status = SubagentStatus::Idle;
        entry
    })])));
    let gateway = SubagentPolicyGateway::new(Some(registry));
    assert!(gateway.has_children());
}

#[test]
fn gateway_has_children_ignores_exited_registry_entries() {
    let registry = Arc::new(Mutex::new(HashMap::from([("child-1".to_string(), {
        let mut entry = SubagentEntry::new(PathBuf::from("/tmp/exited.sock"), 0);
        entry.status = SubagentStatus::Exited;
        entry
    })])));
    let gateway = SubagentPolicyGateway::new(Some(registry));

    assert!(!gateway.has_children());
}

#[test]
fn no_registry_has_no_child_propagation_results() {
    let results = snapshot_targets(&None);
    assert!(results.is_empty());
}

#[test]
fn exited_child_maps_to_disconnected_result_without_uds() {
    let registry = Arc::new(Mutex::new(HashMap::from([("child-1".to_string(), {
        let mut entry = SubagentEntry::new(PathBuf::from("/tmp/missing.sock"), 0);
        entry.status = SubagentStatus::Exited;
        entry
    })])));

    let targets = snapshot_targets(&Some(registry));
    let result = exited_target_result(targets.into_iter().next().unwrap());

    assert_eq!(result.agent_id, "child-1");
    assert_eq!(
        result.status,
        ChildToolPolicyPropagationStatus::Disconnected
    );
    assert_eq!(result.error.as_deref(), Some("child is exited"));
}

#[test]
fn running_child_receives_boundary_mode_even_for_immediate_parent_request() {
    assert_eq!(
        child_apply_mode(
            ToolPolicyApplyMode::ImmediateIfIdle,
            &SubagentStatus::Running
        ),
        ToolPolicyApplyMode::AtNextTurnBoundary
    );
}

#[test]
fn idle_child_receives_immediate_mode_for_immediate_parent_request() {
    assert_eq!(
        child_apply_mode(ToolPolicyApplyMode::ImmediateIfIdle, &SubagentStatus::Idle),
        ToolPolicyApplyMode::ImmediateIfIdle
    );
}

#[test]
fn boundary_parent_request_always_sends_boundary_mode_to_child() {
    assert_eq!(
        child_apply_mode(
            ToolPolicyApplyMode::AtNextTurnBoundary,
            &SubagentStatus::Idle
        ),
        ToolPolicyApplyMode::AtNextTurnBoundary
    );
}

#[test]
fn command_json_contains_set_tool_policy_mode_and_mutations() {
    let command = child_policy_command(
        &[ToolPolicyMutation::disable("bash", "review policy")],
        ToolPolicyApplyMode::AtNextTurnBoundary,
    );
    let value: serde_json::Value = serde_json::from_str(&command).unwrap();

    assert_eq!(value["type"], "set_tool_policy");
    assert_eq!(value["mode"], "atNextTurnBoundary");
    assert_eq!(value["mutations"][0]["name"], "bash");
    assert_eq!(value["mutations"][0]["scope"], "none");
    assert_eq!(value["mutations"][0]["reason"], "review policy");
}

#[test]
fn queued_child_response_maps_to_queued_status() {
    let result = map_child_response(
        "child-1".to_string(),
        Ok(r#"{"ok":true,"data":{"queued":true}}"#.to_string()),
    );

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::Queued);
    assert!(result.reconciliation.is_none());
}

#[test]
fn applied_child_response_parses_reconciliation() {
    let response = r#"{
        "ok": true,
        "data": {
            "mode": "immediateIfIdle",
            "results": [{
                "name": "bash",
                "requestedAvailability": "disabled",
                "requestedScope": "none",
                "status": "applied",
                "reason": "review policy"
            }]
        }
    }"#;

    let result = map_child_response("child-1".to_string(), Ok(response.to_string()));

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::Applied);
    let reconciliation = result.reconciliation.expect("child reconciliation");
    assert_eq!(reconciliation.mode, ToolPolicyApplyMode::ImmediateIfIdle);
    assert_eq!(reconciliation.results[0].name, "bash");
}

#[test]
fn blocked_child_response_maps_to_blocked_by_ceiling() {
    let response = r#"{
        "mode": "immediateIfIdle",
        "results": [{
            "name": "bash",
            "requestedAvailability": "enabled",
            "requestedScope": "both",
            "status": "blockedByRestriction",
            "reason": "cannot widen"
        }]
    }"#;

    let result = map_child_response("child-1".to_string(), Ok(response.to_string()));

    assert_eq!(
        result.status,
        ChildToolPolicyPropagationStatus::BlockedByCeiling
    );
}

#[test]
fn unknown_tool_child_response_maps_to_unknown_tool() {
    let response = r#"{
        "mode": "immediateIfIdle",
        "results": [{
            "name": "missing",
            "requestedAvailability": "disabled",
            "requestedScope": "none",
            "status": "unknownTool",
            "reason": "missing"
        }]
    }"#;

    let result = map_child_response("child-1".to_string(), Ok(response.to_string()));

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::UnknownTool);
}

#[test]
fn timeout_error_maps_to_timeout_status() {
    let result = map_child_response(
        "child-1".to_string(),
        Err(DomainError::Other("timed out waiting for child".into())),
    );

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::Timeout);
}

#[test]
fn invalid_json_child_response_maps_to_error() {
    let result = map_child_response("child-1".to_string(), Ok("not json".to_string()));

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::Error);
    assert!(result.error.unwrap().contains("invalid child response"));
}

#[test]
fn disconnected_error_maps_to_disconnected_status() {
    let result = map_child_response(
        "child-1".to_string(),
        Err(DomainError::Other("connection refused".into())),
    );

    assert_eq!(
        result.status,
        ChildToolPolicyPropagationStatus::Disconnected
    );
    assert_eq!(result.error.as_deref(), Some("connection refused"));
}

#[test]
fn malformed_success_response_without_reconciliation_maps_to_error() {
    let result = map_child_response(
        "child-1".to_string(),
        Ok(r#"{"ok":true,"data":{"ignored":true}}"#.to_string()),
    );

    assert_eq!(result.status, ChildToolPolicyPropagationStatus::Error);
    assert!(result.reconciliation.is_none());
    assert!(result.error.is_none());
}

#[test]
fn parse_reconciliation_rejects_unknown_mode() {
    let value = serde_json::json!({"mode":"later","results":[]});

    assert!(parse_reconciliation(&value).is_none());
}

#[test]
fn parse_mutation_result_rejects_unknown_availability() {
    let value = serde_json::json!({
        "name": "bash",
        "requestedAvailability": "hidden",
        "requestedScope": "none",
        "status": "applied",
        "reason": "bad availability"
    });

    assert!(parse_mutation_result(&value).is_none());
}
