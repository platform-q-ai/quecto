use quecto::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus};
use quecto::infrastructure::tools::registry::ToolRegistryImpl;

#[test]
fn unknown_tool_mutation_reports_unknown_without_side_effects() {
    let mut reg = ToolRegistryImpl::new();
    let result = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::disable("missing", "contract")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(result.mode, ToolPolicyApplyMode::ImmediateIfIdle);
    assert_eq!(
        result.results[0].status,
        ToolPolicyMutationStatus::UnknownTool
    );
    assert!(result.results[0].before.is_none());
    assert!(result.results[0].after.is_none());
}
