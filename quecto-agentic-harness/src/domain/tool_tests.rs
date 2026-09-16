use super::super::tool_descriptor::{ProfileAvailabilityScope, ToolAvailability};
use super::*;

#[test]
fn tool_result_and_image_block_construct() {
    let r = ToolResult {
        content: "ok".into(),
        is_error: false,
        image_blocks: vec![ImageBlock {
            mime_type: "image/png",
            data: "AAAA".into(),
        }],
        delivery_metadata: None,
    };
    assert!(!r.is_error);
    assert_eq!(r.image_blocks[0].mime_type, "image/png");
}

#[test]
fn tool_policy_mutation_constructors_cover_enable_disable_and_scope() {
    let enable = ToolPolicyMutation::enable("spawn", "allow children");
    assert_eq!(enable.name, "spawn");
    assert_eq!(enable.availability, ToolAvailability::Enabled);
    assert_eq!(enable.scope, ProfileAvailabilityScope::Both);
    assert_eq!(enable.reason, "allow children");

    let disable = ToolPolicyMutation::disable("spawn", "pause children");
    assert_eq!(disable.availability, ToolAvailability::Disabled);
    assert_eq!(disable.scope, ProfileAvailabilityScope::None);

    let parent_only =
        ToolPolicyMutation::set_scope("spawn", ProfileAvailabilityScope::Parent, "parent only");
    assert_eq!(parent_only.availability, ToolAvailability::Enabled);
    assert_eq!(parent_only.scope, ProfileAvailabilityScope::Parent);
}

#[test]
fn tool_policy_request_constructors_cover_patch_and_replace() {
    let mutation = ToolPolicyMutation::disable("spawn", "off");
    let patch = ToolPolicyRequest::patch(vec![mutation.clone()]);
    assert_eq!(patch.operation, ToolPolicyOperation::Patch);
    assert_eq!(patch.mutations, vec![mutation.clone()]);
    assert_eq!(patch.unlisted_scope, None);

    let replace = ToolPolicyRequest::replace(vec![mutation], ProfileAvailabilityScope::Child);
    assert_eq!(replace.operation, ToolPolicyOperation::Replace);
    assert_eq!(
        replace.unlisted_scope,
        Some(ProfileAvailabilityScope::Child)
    );
}

#[test]
fn tool_policy_mutation_result_wire_uses_camel_case_fields_and_status() {
    // set_tool_policy ack and tool_policy_changed results serialize the domain
    // structs directly — field names and status must match protocol camelCase.
    let result = ToolPolicyMutationResult {
        name: "alpha".into(),
        requested_identifier: Some("tool-alpha".into()),
        requested_availability: ToolAvailability::Enabled,
        requested_scope: ProfileAvailabilityScope::Parent,
        status: ToolPolicyMutationStatus::Applied,
        before: None,
        after: None,
        reason: "test".into(),
    };
    let wire = serde_json::to_value(&result).expect("serialize mutation result");
    assert!(
        wire.get("requestedAvailability").is_some(),
        "expected camelCase requestedAvailability, got keys: {:?}",
        wire.as_object().map(|o| o.keys().collect::<Vec<_>>())
    );
    assert!(wire.get("requested_availability").is_none());
    assert!(
        wire.get("requestedScope").is_some(),
        "expected camelCase requestedScope"
    );
    assert!(wire.get("requested_scope").is_none());
    assert_eq!(wire["status"], "applied");
    assert_eq!(wire["requestedIdentifier"], "tool-alpha");
    assert_eq!(wire["requestedAvailability"], "enabled");
    assert_eq!(wire["requestedScope"], "parent");

    let already = ToolPolicyMutationResult {
        status: ToolPolicyMutationStatus::AlreadyInState,
        ..result.clone()
    };
    let already_wire = serde_json::to_value(&already).expect("serialize already-in-state");
    assert_eq!(already_wire["status"], "alreadyInState");

    let blocked = ToolPolicyMutationResult {
        status: ToolPolicyMutationStatus::BlockedByRestriction,
        ..result.clone()
    };
    let blocked_wire = serde_json::to_value(&blocked).expect("serialize blocked");
    assert_eq!(blocked_wire["status"], "blockedByRestriction");

    let unknown = ToolPolicyMutationResult {
        status: ToolPolicyMutationStatus::UnknownTool,
        ..result.clone()
    };
    let unknown_wire = serde_json::to_value(&unknown).expect("serialize unknown");
    assert_eq!(unknown_wire["status"], "unknownTool");

    let persistence_failed = ToolPolicyMutationResult {
        status: ToolPolicyMutationStatus::PersistenceFailed,
        ..result
    };
    let persistence_failed_wire =
        serde_json::to_value(&persistence_failed).expect("serialize persistence failed");
    assert_eq!(persistence_failed_wire["status"], "persistenceFailed");

    let reconciliation = ToolPolicyReconciliation {
        correlation_id: None,
        mode: ToolPolicyApplyMode::ImmediateIfIdle,
        results: vec![ToolPolicyMutationResult {
            name: "beta".into(),
            requested_identifier: None,
            requested_availability: ToolAvailability::Disabled,
            requested_scope: ProfileAvailabilityScope::None,
            status: ToolPolicyMutationStatus::Applied,
            before: None,
            after: None,
            reason: "off".into(),
        }],
    };
    let recon_wire = serde_json::to_value(&reconciliation).expect("serialize reconciliation");
    assert_eq!(recon_wire["mode"], "immediateIfIdle");
    assert_eq!(
        recon_wire["results"][0]["requestedAvailability"],
        "disabled"
    );
    assert_eq!(recon_wire["results"][0]["requestedScope"], "none");
    assert_eq!(recon_wire["results"][0]["status"], "applied");
}
