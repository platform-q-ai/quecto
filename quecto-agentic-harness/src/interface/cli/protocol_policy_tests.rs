use super::{AgentCommand, ToolPolicyApplyModeCommand, ToolPolicyOperationCommand};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

#[test]
fn set_tool_policy_command_deserializes_scope_and_mode() {
    let json = r#"{"type":"set_tool_policy","id":"p1","mode":"atNextTurnBoundary","mutations":[{"name":"alpha","scope":"child","reason":"test"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetToolPolicy {
            id,
            mutations,
            mode,
            operation,
            unlisted_scope,
        } => {
            assert_eq!(id.as_deref(), Some("p1"));
            assert_eq!(mode, ToolPolicyApplyModeCommand::AtNextTurnBoundary);
            assert_eq!(operation, ToolPolicyOperationCommand::Patch);
            assert_eq!(unlisted_scope, None);
            assert_eq!(mutations[0].name.as_deref(), Some("alpha"));
            assert_eq!(mutations[0].scope, ProfileAvailabilityScope::Child);
        }
        other => panic!("expected SetToolPolicy, got {other:?}"),
    }
}

#[test]
fn tool_policy_apply_mode_wire_uses_camel_case() {
    let mode = crate::domain::tool::ToolPolicyApplyMode::AtNextTurnBoundary;
    let value = serde_json::to_value(mode).expect("serialize apply mode");
    assert_eq!(value, serde_json::json!("atNextTurnBoundary"));

    let mode = crate::domain::tool::ToolPolicyApplyMode::ImmediateIfIdle;
    let value = serde_json::to_value(mode).expect("serialize apply mode");
    assert_eq!(value, serde_json::json!("immediateIfIdle"));

    let command_mode: ToolPolicyApplyModeCommand =
        serde_json::from_value(serde_json::json!("atNextTurnBoundary"))
            .expect("parse command mode");
    assert_eq!(command_mode, ToolPolicyApplyModeCommand::AtNextTurnBoundary);

    let reconciliation = crate::domain::tool::ToolPolicyReconciliation {
        mode: crate::domain::tool::ToolPolicyApplyMode::AtNextTurnBoundary,
        results: vec![],
    };
    let wire = serde_json::to_value(&reconciliation).expect("serialize reconciliation");
    assert_eq!(wire["mode"], "atNextTurnBoundary");
}

#[test]
fn set_tool_policy_replace_requires_unlisted_scope_and_uses_public_wire_names() {
    let json = r#"{"type":"set_tool_policy","operation":"replace","unlistedScope":"none","mutations":[{"name":"alpha","scope":"child"}]}"#;
    let cmd: AgentCommand = serde_json::from_str(json).unwrap();
    match cmd {
        AgentCommand::SetToolPolicy {
            operation,
            unlisted_scope,
            ..
        } => {
            assert_eq!(operation, ToolPolicyOperationCommand::Replace);
            assert_eq!(unlisted_scope, Some(ProfileAvailabilityScope::None));
        }
        other => panic!("expected SetToolPolicy, got {other:?}"),
    }
}
