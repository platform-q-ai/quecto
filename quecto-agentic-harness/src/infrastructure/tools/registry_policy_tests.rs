use super::tests::{DummyTestTool, test_registry};
use crate::domain::tool::{
    Tool, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus,
};
use std::sync::Arc;

#[test]
fn policy_mutation_reports_deterministic_outcomes_and_snapshots() {
    let (mut reg, _tmp) = test_registry();
    let restricted = vec!["bash".to_string()];
    reg.apply_spawn_tool_restrictions(&restricted);

    let reconciliation = reg.apply_tool_policy_mutations(
        &[
            ToolPolicyMutation::disable("read", "test disable"),
            ToolPolicyMutation::disable("read", "test duplicate"),
            ToolPolicyMutation::enable("bash", "test blocked"),
            ToolPolicyMutation::enable("missing", "test unknown"),
        ],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(reconciliation.mode, ToolPolicyApplyMode::ImmediateIfIdle);
    let statuses: Vec<_> = reconciliation
        .results
        .iter()
        .map(|result| result.status)
        .collect();
    assert_eq!(
        statuses,
        vec![
            ToolPolicyMutationStatus::Applied,
            ToolPolicyMutationStatus::AlreadyInState,
            ToolPolicyMutationStatus::BlockedByRestriction,
            ToolPolicyMutationStatus::UnknownTool,
        ]
    );
    let read = &reconciliation.results[0];
    assert_eq!(read.reason, "test disable");
    assert!(read.before.as_ref().unwrap().effective_enabled);
    assert!(!read.after.as_ref().unwrap().effective_enabled);

    let duplicate_read = &reconciliation.results[1];
    assert_eq!(duplicate_read.reason, "test duplicate");
    assert_eq!(duplicate_read.before, duplicate_read.after);
    assert!(!duplicate_read.after.as_ref().unwrap().effective_enabled);

    let blocked_bash = &reconciliation.results[2];
    assert_eq!(blocked_bash.reason, "test blocked");
    assert_eq!(blocked_bash.before, blocked_bash.after);
    assert!(!blocked_bash.after.as_ref().unwrap().effective_enabled);

    let missing = &reconciliation.results[3];
    assert_eq!(missing.reason, "test unknown");
    assert!(missing.before.is_none());
    assert!(missing.after.is_none());

    assert!(!reg.descriptor("read").unwrap().availability.is_enabled());
    assert!(!reg.descriptor("bash").unwrap().availability.is_enabled());
}

#[test]
fn startup_restriction_blocks_live_policy_enable_and_later_registration() {
    let (mut reg, _tmp) = test_registry();
    let restricted = vec!["read".to_string(), "future_tool".to_string()];
    let warnings = reg.apply_startup_tool_restrictions(&restricted);
    assert_eq!(warnings, vec!["future_tool".to_string()]);

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::enable("read", "startup ceiling")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert!(
        !reg.definitions()
            .iter()
            .any(|definition| definition.name == "read")
    );

    let future_tool: Arc<dyn Tool> = Arc::new(DummyTestTool::new("future_tool"));
    assert!(!reg.register_uds_extension(future_tool));
    assert!(reg.get("future_tool").is_none());
}

#[test]
fn entrypoint_default_restriction_blocks_live_policy_enable() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.disable_tool_by_entrypoint_default("read"));

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::enable("read", "entrypoint ceiling")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert!(
        !reconciliation.results[0]
            .after
            .as_ref()
            .unwrap()
            .effective_enabled
    );
}
