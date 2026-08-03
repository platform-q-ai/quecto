use std::collections::BTreeMap;
use std::sync::Arc;

use super::tests::{DummyTestTool, test_registry};
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolProfileContext,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot;

#[test]
fn inherited_snapshot_hides_child_denied_tools_and_blocks_widen() {
    let (mut reg, _tmp) = test_registry();
    reg.set_execution_profile_context(ToolProfileContext::Child);
    let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([
        ("read".to_string(), ProfileAvailabilityScope::Both),
        ("bash".to_string(), ProfileAvailabilityScope::None),
    ]));

    assert!(
        reg.apply_inherited_tool_policy_snapshot(&snapshot)
            .is_empty()
    );

    let names: Vec<_> = reg
        .definitions_for(ToolProfileContext::Child)
        .iter()
        .map(|d| d.name.as_ref())
        .collect();
    assert!(names.contains(&"read"));
    assert!(!names.contains(&"bash"));

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "bash",
            ProfileAvailabilityScope::Both,
            "try widen",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    let bash = reg.catalogue_entry("bash").unwrap();
    assert!(!bash.effective_child_enabled);
}

#[test]
fn inherited_snapshot_is_closed_for_omitted_and_late_registered_tools() {
    let (mut reg, _tmp) = test_registry();
    reg.set_execution_profile_context(ToolProfileContext::Child);
    let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
        "read".to_string(),
        ProfileAvailabilityScope::Both,
    )]));

    assert!(
        reg.apply_inherited_tool_policy_snapshot(&snapshot)
            .is_empty()
    );

    let child_names: Vec<_> = reg
        .definitions_for(ToolProfileContext::Child)
        .iter()
        .map(|d| d.name.as_ref())
        .collect();
    assert!(child_names.contains(&"read"));
    assert!(!child_names.contains(&"bash"));

    assert!(reg.register_runtime_tool(Arc::new(DummyTestTool::new("late_runtime"))));
    assert!(
        !reg.definitions_for(ToolProfileContext::Child)
            .iter()
            .any(|d| d.name.as_ref() == "late_runtime"),
        "late runtime registration must not bypass inherited closed policy"
    );

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "late_runtime",
            ProfileAvailabilityScope::Both,
            "try widen late tool",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
}
