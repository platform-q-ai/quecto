//! Mixed config/session transitions must repair session state without authority bypass.
use super::*;
use crate::infrastructure::config::{ToolPolicyConfig, ToolPolicyEntryConfig};

fn replace(scope: ProfileAvailabilityScope) -> ToolPolicyRequest {
    ToolPolicyRequest::replace(Vec::new(), scope)
}

fn reload_read(reg: &mut ToolRegistryImpl) {
    let stable = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "read")
        .unwrap()
        .stable_id
        .into_owned();
    let mut policy = ToolPolicyConfig::default();
    policy.entries.insert(
        stable,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Both,
        },
    );
    assert!(reg.apply_persisted_tool_policy(&policy).is_empty());
}

#[test]
fn run24_equal_profile_replace_repairs_runtime_after_persisted_reload() {
    let (mut reg, _home) = test_registry();
    reg.apply_tool_policy_request(
        &replace(ProfileAvailabilityScope::None),
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    reload_read(&mut reg);
    reg.apply_tool_policy_request(
        &replace(ProfileAvailabilityScope::Both),
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == "read")
        .unwrap();
    assert!(
        entry.effective_parent_enabled && entry.effective_child_enabled,
        "equal profile is not equal runtime: {entry:?}"
    );
    let again = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::enable(
            "read",
            "legitimate no-op after repair",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        again.results[0].status,
        ToolPolicyMutationStatus::AlreadyInState
    );
}

#[test]
fn run24_reload_replace_preserves_shadowing_ceilings_and_unknown_failure() {
    for shadow in ["default", "session", "inherited", "restriction"] {
        let (mut reg, _home) = test_registry();
        reg.apply_tool_policy_request(
            &replace(ProfileAvailabilityScope::None),
            ToolPolicyApplyMode::ImmediateIfIdle,
        );
        reload_read(&mut reg);
        match shadow {
            "default" => {
                reg.disable_tool_by_entrypoint_default("read");
            }
            "session" => {
                reg.metadata.get_mut("read").unwrap().session_enabled = Some(false);
            }
            "inherited" => {
                reg.metadata.get_mut("read").unwrap().inherited_scope =
                    Some(ProfileAvailabilityScope::Child);
            }
            "restriction" => {
                reg.disable_tool_by_spawn_restriction("read");
            }
            _ => unreachable!("affirmative fixture variants"),
        }
        reg.apply_tool_policy_request(
            &replace(ProfileAvailabilityScope::Both),
            ToolPolicyApplyMode::ImmediateIfIdle,
        );
        let entry = reg
            .catalogue_entries()
            .into_iter()
            .find(|entry| entry.name == "read")
            .unwrap();
        assert!(
            !entry.effective_parent_enabled,
            "Replace must not override {shadow}"
        );
        let before = format!("{entry:?}");
        let unknown = reg.apply_tool_policy_mutations(
            &[ToolPolicyMutation::enable(
                "missing-run24",
                "unknown must be inert",
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        );
        assert_eq!(
            unknown.results[0].status,
            ToolPolicyMutationStatus::UnknownTool
        );
        let after = reg
            .catalogue_entries()
            .into_iter()
            .find(|entry| entry.name == "read")
            .unwrap();
        assert_eq!(
            format!("{after:?}"),
            before,
            "failed unknown mutation is inert"
        );
    }
}
