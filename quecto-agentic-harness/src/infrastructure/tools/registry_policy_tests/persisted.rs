use super::*;

#[test]
fn persisted_policy_intersects_with_defaults_profile_restrictions_and_runtime() {
    use crate::infrastructure::config::{ToolPolicyConfig, ToolPolicyEntryConfig};
    use std::collections::HashMap;

    let (mut reg, _tmp) = test_registry();
    reg.disable_tool_by_entrypoint_default("write");
    reg.apply_spawn_tool_restrictions(&["bash".to_string()]);

    let write_id = reg
        .metadata
        .get("write")
        .unwrap()
        .identity_for_name("write")
        .stable_id
        .to_string();
    let python_id = reg
        .metadata
        .get("python_lab")
        .unwrap()
        .identity_for_name("python_lab")
        .stable_id
        .to_string();
    let bash_id = reg
        .metadata
        .get("bash")
        .unwrap()
        .identity_for_name("bash")
        .stable_id
        .to_string();
    let mut entries = HashMap::new();
    entries.insert(
        write_id,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Both,
        },
    );
    entries.insert(
        python_id,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Parent,
        },
    );
    entries.insert(
        bash_id,
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Both,
        },
    );
    entries.insert(
        "tool.v1:removed:future_tool".to_string(),
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Both,
        },
    );
    entries.insert(
        "bash".to_string(),
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::None,
        },
    );

    let unknown = reg.apply_persisted_tool_policy(&ToolPolicyConfig { entries });
    assert_eq!(
        unknown,
        vec![
            "bash".to_string(),
            "tool.v1:removed:future_tool".to_string()
        ]
    );

    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("write").unwrap()),
        ProfileAvailabilityScope::None,
        "persisted both must not widen entrypoint-disabled defaults"
    );
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("python_lab").unwrap()),
        ProfileAvailabilityScope::Parent
    );

    let python = reg.metadata.get_mut("python_lab").unwrap();
    python.inherited_scope = Some(ProfileAvailabilityScope::Both);
    python.profile_scope = Some(ProfileAvailabilityScope::Both);
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("python_lab").unwrap()),
        ProfileAvailabilityScope::Parent,
        "later inherited/session policy must not widen persisted configured scope"
    );
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("bash").unwrap()),
        ProfileAvailabilityScope::None,
        "persisted both must not widen spawn/session restrictions"
    );

    let widen = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "python_lab",
            ProfileAvailabilityScope::Both,
            "user widens durable preference",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert!(
        matches!(
            widen.results[0].status,
            crate::domain::tool::ToolPolicyMutationStatus::Applied
                | crate::domain::tool::ToolPolicyMutationStatus::AlreadyInState
        ),
        "persisted preferences must not lock users out of widening them later"
    );
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("python_lab").unwrap()),
        ProfileAvailabilityScope::Both,
        "persisted live widen must update the effective configured preference immediately"
    );

    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "python_lab",
            ProfileAvailabilityScope::None,
            "live profile narrows persisted parent",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("python_lab").unwrap()),
        ProfileAvailabilityScope::None
    );
}

#[test]
fn late_registered_uds_tool_gets_retained_persisted_policy() {
    use crate::infrastructure::config::{ToolPolicyConfig, ToolPolicyEntryConfig};
    use std::collections::HashMap;
    use std::sync::Arc;

    let (mut reg, _tmp) = test_registry();
    let stable_id = "tool.v1:test:late:tool".to_string();
    let mut entries = HashMap::new();
    entries.insert(
        stable_id.clone(),
        ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::None,
        },
    );

    let unknown = reg.apply_persisted_tool_policy(&ToolPolicyConfig { entries });
    assert_eq!(unknown, vec![stable_id.clone()]);

    assert!(reg.register_uds_tool_for_owner_with_stable_id(
        Arc::new(DummyTestTool::new("late_uds")),
        "owner".into(),
        Some(stable_id),
    ));

    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("late_uds").unwrap()),
        ProfileAvailabilityScope::None,
        "late UDS/MCP registration must reapply retained persisted policy"
    );
}

#[test]
fn registry_trait_forwarders_cover_tool_policy_and_catalogue_ports() {
    use crate::domain::tool::{
        RuntimeToolLifecycleRegistry, SessionAwareTools, ToolCatalog, ToolPolicyMutator,
    };

    let (mut reg, _tmp) = test_registry();
    let catalog: &dyn ToolCatalog = &reg;
    assert!(!catalog.definitions().is_empty());
    assert!(
        !catalog
            .definitions_for(crate::domain::tool::ToolProfileContext::Parent)
            .is_empty()
    );
    assert!(!catalog.descriptors().is_empty());
    assert!(!catalog.catalogue_entries().is_empty());

    let mutator: &mut dyn ToolPolicyMutator = &mut reg;
    let patch = ToolPolicyMutation::set_scope(
        "python_lab",
        ProfileAvailabilityScope::Child,
        "trait coverage patch",
    );
    let applied =
        mutator.apply_tool_policy_mutations(&[patch], ToolPolicyApplyMode::ImmediateIfIdle);
    assert!(!applied.results.is_empty());
    let replace = crate::domain::tool::ToolPolicyRequest::replace(
        vec![ToolPolicyMutation::set_scope(
            "python_lab",
            ProfileAvailabilityScope::Both,
            "trait coverage replace",
        )],
        ProfileAvailabilityScope::None,
    );
    let replaced =
        mutator.apply_tool_policy_request(&replace, ToolPolicyApplyMode::ImmediateIfIdle);
    assert!(!replaced.results.is_empty());

    let lifecycle: &mut dyn RuntimeToolLifecycleRegistry = &mut reg;
    assert!(lifecycle.register_runtime_tool(std::sync::Arc::new(DummyTestTool::new("cov_rt"))));
    assert!(lifecycle.register_uds_tool(std::sync::Arc::new(DummyTestTool::new("cov_uds"))));
    assert!(lifecycle.can_register_uds_tool_for_owner("cov_owner", "owner"));
    assert!(lifecycle.can_register_uds_tool_for_owner_with_stable_id(
        "cov_owner_stable",
        "owner",
        Some("tool.v1:test:owner:cov_owner_stable")
    ));
    assert!(lifecycle.register_uds_tool_for_owner(
        std::sync::Arc::new(DummyTestTool::new("cov_owner")),
        std::borrow::Cow::Borrowed("owner")
    ));
    assert!(lifecycle.register_uds_tool_for_owner_with_stable_id(
        std::sync::Arc::new(DummyTestTool::new("cov_owner_stable")),
        std::borrow::Cow::Borrowed("owner"),
        Some("tool.v1:test:owner:cov_owner_stable".to_string())
    ));
    assert!(
        lifecycle
            .runtime_tool_names()
            .contains(&"cov_rt".to_string())
    );
    assert!(lifecycle.enable_tool("python_lab"));
    assert!(lifecycle.disable_tool("python_lab"));
    lifecycle.unregister_runtime_tool("missing-tool-for-coverage");
    assert!(
        lifecycle
            .unregister_runtime_tools_for_owner("missing-owner")
            .is_empty()
    );
    lifecycle.set_inherited_child_policy_snapshot_for_spawn(std::collections::BTreeMap::new());

    let session: &dyn SessionAwareTools = &reg;
    session.set_session_key("coverage-session");
}
