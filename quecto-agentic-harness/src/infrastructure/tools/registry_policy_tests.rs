use super::tests::{DummyTestTool, test_registry};
use crate::domain::tool::{
    Tool, ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolProfileContext,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
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
    assert_eq!(
        read.before.as_ref().unwrap().effective_scope,
        ProfileAvailabilityScope::Both
    );
    assert_eq!(read.before.as_ref().unwrap().profile_scope, None);
    assert!(read.before.as_ref().unwrap().effective_parent_enabled);
    assert!(read.before.as_ref().unwrap().effective_child_enabled);
    assert!(read.before.as_ref().unwrap().effective_enabled);
    assert_eq!(
        read.after.as_ref().unwrap().effective_scope,
        ProfileAvailabilityScope::None
    );
    assert_eq!(
        read.after.as_ref().unwrap().profile_scope,
        Some(ProfileAvailabilityScope::None)
    );
    assert!(!read.after.as_ref().unwrap().effective_parent_enabled);
    assert!(!read.after.as_ref().unwrap().effective_child_enabled);
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
    assert!(!reg.register_uds_tool(future_tool));
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

#[test]
fn scope_policy_controls_parent_and_child_model_visible_definitions() {
    let (mut reg, _tmp) = test_registry();

    let reconciliation = reg.apply_tool_policy_mutations(
        &[
            ToolPolicyMutation::set_scope("read", ProfileAvailabilityScope::Parent, "parent only"),
            ToolPolicyMutation::set_scope("bash", ProfileAvailabilityScope::Child, "child only"),
        ],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert_eq!(
        reconciliation.results[1].status,
        ToolPolicyMutationStatus::Applied
    );

    let parent_names: Vec<_> = reg
        .definitions_for(ToolProfileContext::Parent)
        .iter()
        .map(|definition| definition.name.as_ref())
        .collect();
    let child_names: Vec<_> = reg
        .definitions_for(ToolProfileContext::Child)
        .iter()
        .map(|definition| definition.name.as_ref())
        .collect();

    assert!(parent_names.contains(&"read"));
    assert!(!parent_names.contains(&"bash"));
    assert!(!child_names.contains(&"read"));
    assert!(child_names.contains(&"bash"));
    assert_eq!(
        reg.definitions(),
        reg.definitions_for(ToolProfileContext::Parent)
    );
}

#[test]
fn catalogue_before_and_after_report_scope_fields() {
    let (mut reg, _tmp) = test_registry();

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Child,
            "catalogue scope",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    let result = &reconciliation.results[0];
    assert_eq!(result.requested_scope, ProfileAvailabilityScope::Child);
    let before = result.before.as_ref().unwrap();
    assert_eq!(before.effective_scope, ProfileAvailabilityScope::Both);
    assert!(before.effective_parent_enabled);
    assert!(before.effective_child_enabled);
    assert!(before.effective_enabled);

    let after = result.after.as_ref().unwrap();
    assert_eq!(after.profile_scope, Some(ProfileAvailabilityScope::Child));
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::Child);
    assert!(!after.effective_parent_enabled);
    assert!(after.effective_child_enabled);
    assert!(after.effective_enabled);
    assert_eq!(after.profile_enabled, Some(true));
}

#[test]
fn restriction_ceiling_blocks_scope_widening_but_allows_narrowing_to_none() {
    let (mut reg, _tmp) = test_registry();
    reg.apply_spawn_tool_restrictions(&["read".to_string()]);

    let widen = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Both,
            "blocked widening",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        widen.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert_eq!(widen.results[0].before, widen.results[0].after);
    assert_eq!(
        widen.results[0].after.as_ref().unwrap().effective_scope,
        ProfileAvailabilityScope::None
    );

    let narrow = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::disable("read", "already narrowed")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        narrow.results[0].status,
        ToolPolicyMutationStatus::AlreadyInState
    );
    assert_eq!(
        narrow.results[0].after.as_ref().unwrap().effective_scope,
        ProfileAvailabilityScope::None
    );
}

#[test]
fn already_in_state_compares_profile_scope_not_legacy_boolean() {
    let (mut reg, _tmp) = test_registry();

    assert_eq!(
        reg.apply_tool_policy_mutations(
            &[ToolPolicyMutation::set_scope(
                "read",
                ProfileAvailabilityScope::Parent,
                "first",
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        )
        .results[0]
            .status,
        ToolPolicyMutationStatus::Applied
    );

    let second = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Parent,
            "same scope",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(
        second.results[0].status,
        ToolPolicyMutationStatus::AlreadyInState
    );
}

#[test]
fn scope_setting_mutations_cover_every_scope_and_derived_catalogue_fields_serialize() {
    let cases = [
        (ProfileAvailabilityScope::None, false, false, false),
        (ProfileAvailabilityScope::Parent, true, false, true),
        (ProfileAvailabilityScope::Child, false, true, true),
        (ProfileAvailabilityScope::Both, true, true, true),
    ];

    for (scope, parent_enabled, child_enabled, effective_enabled) in cases {
        let (mut reg, _tmp) = test_registry();
        if scope == ProfileAvailabilityScope::Both {
            reg.apply_tool_policy_mutations(
                &[ToolPolicyMutation::set_scope(
                    "read",
                    ProfileAvailabilityScope::None,
                    "prime",
                )],
                ToolPolicyApplyMode::ImmediateIfIdle,
            );
        }
        let reconciliation = reg.apply_tool_policy_mutations(
            &[ToolPolicyMutation::set_scope(
                "read",
                scope,
                format!("scope {scope:?}"),
            )],
            ToolPolicyApplyMode::ImmediateIfIdle,
        );
        assert_eq!(
            reconciliation.results[0].status,
            ToolPolicyMutationStatus::Applied
        );
        let entry = reg
            .catalogue_entries()
            .into_iter()
            .find(|entry| entry.name.as_ref() == "read")
            .unwrap();
        assert_eq!(entry.profile_scope, Some(scope));
        assert_eq!(entry.effective_scope, scope);
        assert_eq!(entry.effective_parent_enabled, parent_enabled);
        assert_eq!(entry.effective_child_enabled, child_enabled);
        assert_eq!(entry.effective_enabled, effective_enabled);
        assert_eq!(entry.profile_enabled, Some(effective_enabled));

        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["profileScope"], serde_json::json!(scope));
        assert_eq!(json["effectiveScope"], serde_json::json!(scope));
        assert_eq!(json["effectiveParentEnabled"], parent_enabled);
        assert_eq!(json["effectiveChildEnabled"], child_enabled);
        assert_eq!(json["effectiveEnabled"], effective_enabled);
        assert_eq!(json["profileEnabled"], effective_enabled);
    }
}

#[test]
fn effective_scope_intersects_runtime_default_session_and_profile_ceilings() {
    let (mut reg, _tmp) = test_registry();

    assert!(reg.disable_tool_by_entrypoint_default("read"));
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "read")
        .unwrap();
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::None);
    assert!(!entry.effective_parent_enabled);
    assert!(!entry.effective_child_enabled);
    assert!(!entry.effective_enabled);

    let (mut reg, _tmp) = test_registry();
    assert!(reg.disable_tool("read"));
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "read")
        .unwrap();
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::None);
    assert!(!entry.effective_parent_enabled);
    assert!(!entry.effective_child_enabled);
    assert!(!entry.effective_enabled);

    let (mut reg, _tmp) = test_registry();
    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Parent,
            "parent",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "read")
        .unwrap();
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::Parent);
    assert!(entry.effective_parent_enabled);
    assert!(!entry.effective_child_enabled);
    assert!(entry.effective_enabled);

    reg.apply_spawn_tool_restrictions(&["read".to_string()]);
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "read")
        .unwrap();
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::None);
    assert!(!entry.effective_parent_enabled);
    assert!(!entry.effective_child_enabled);
    assert!(!entry.effective_enabled);
}

#[test]
fn enable_mutation_after_runtime_disable_restores_profile_scope() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.disable_tool("read"));

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::enable(
            "read",
            "enable after runtime disable",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    let after = reconciliation.results[0].after.as_ref().unwrap();
    assert_eq!(after.profile_scope, Some(ProfileAvailabilityScope::Both));
    assert_eq!(after.effective_scope, ProfileAvailabilityScope::Both);
    assert!(after.effective_enabled);
}

#[test]
fn explicit_restriction_is_a_ceiling_for_scope_mutations() {
    let (mut reg, _tmp) = test_registry();
    reg.apply_spawn_tool_restrictions(&["read".to_string()]);

    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "read",
            ProfileAvailabilityScope::Parent,
            "blocked by explicit spawn restriction",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
    assert_eq!(
        reconciliation.results[0].before,
        reconciliation.results[0].after
    );
}

#[test]
fn runtime_enable_clears_prior_profile_disable() {
    let (mut reg, _tmp) = test_registry();
    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::disable("read", "profile off")],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    assert!(reg.enable_tool("read"));
    let entry = reg
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name.as_ref() == "read")
        .unwrap();
    assert_eq!(entry.profile_scope, Some(ProfileAvailabilityScope::Both));
    assert_eq!(entry.effective_scope, ProfileAvailabilityScope::Both);
    assert!(entry.effective_parent_enabled);
    assert!(entry.effective_child_enabled);
}
