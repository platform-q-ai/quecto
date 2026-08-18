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

    let mut widen_request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "python_lab",
        ProfileAvailabilityScope::Both,
        "user widens durable preference",
    )]);
    widen_request.persist = true;
    let widen = reg.apply_tool_policy_request(&widen_request, ToolPolicyApplyMode::ImmediateIfIdle);
    crate::domain::tool::ToolPolicyMutator::record_persisted_tool_policy_results(&mut reg, &widen);
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
fn same_scope_persist_after_live_only_policy_installs_retained_ceiling_for_reregistered_uds_tool() {
    use std::sync::Arc;

    let (mut reg, _tmp) = test_registry();
    let stable_id = "tool.v1:test:owner:live_then_persist_uds".to_string();
    assert!(reg.register_uds_tool_for_owner_with_stable_id(
        Arc::new(DummyTestTool::new("live_then_persist_uds")),
        "owner".into(),
        Some(stable_id.clone()),
    ));

    let live_only = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "live_then_persist_uds",
            ProfileAvailabilityScope::Parent,
            "temporary live-only parent scope",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        live_only.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert_eq!(
        reg.metadata
            .get("live_then_persist_uds")
            .unwrap()
            .configured_scope,
        None,
        "persist:false live-only mutation must not install a configured ceiling"
    );

    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "live_then_persist_uds",
        ProfileAvailabilityScope::Parent,
        "make current parent scope durable",
    )]);
    request.persist = true;
    let persisted = reg.apply_tool_policy_request(&request, ToolPolicyApplyMode::ImmediateIfIdle);
    crate::domain::tool::ToolPolicyMutator::record_persisted_tool_policy_results(
        &mut reg, &persisted,
    );
    assert_eq!(
        persisted.results[0].status,
        ToolPolicyMutationStatus::Applied,
        "persist:true at the current live scope still has retained state to install"
    );
    assert_eq!(
        reg.metadata
            .get("live_then_persist_uds")
            .unwrap()
            .configured_scope,
        Some(ProfileAvailabilityScope::Parent),
        "same-scope persist:true must install the live configured ceiling"
    );
    assert_eq!(
        reg.persisted_policy_scopes.get(&stable_id),
        Some(&ProfileAvailabilityScope::Parent),
        "same-scope persist:true must be retained by stable id for reconnects"
    );

    reg.unregister_runtime_tool("live_then_persist_uds");
    assert!(reg.register_uds_tool_for_owner_with_stable_id(
        Arc::new(DummyTestTool::new("live_then_persist_uds")),
        "owner".into(),
        Some(stable_id),
    ));
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("live_then_persist_uds").unwrap()),
        ProfileAvailabilityScope::Parent,
        "UDS/MCP reconnect before restart must reapply the just-persisted ceiling"
    );
}

#[test]
fn first_time_live_policy_persist_installs_retained_ceiling_for_reregistered_uds_tool() {
    use std::sync::Arc;

    let (mut reg, _tmp) = test_registry();
    let stable_id = "tool.v1:test:owner:first_time_uds".to_string();
    assert!(reg.register_uds_tool_for_owner_with_stable_id(
        Arc::new(DummyTestTool::new("first_time_uds")),
        "owner".into(),
        Some(stable_id.clone()),
    ));
    assert_eq!(
        reg.metadata.get("first_time_uds").unwrap().configured_scope,
        None,
        "test starts without a persisted tools.policy entry"
    );

    let mut request = ToolPolicyRequest::patch(vec![ToolPolicyMutation::set_scope(
        "first_time_uds",
        ProfileAvailabilityScope::Parent,
        "first durable preference",
    )]);
    request.persist = true;
    let persisted = reg.apply_tool_policy_request(&request, ToolPolicyApplyMode::ImmediateIfIdle);
    crate::domain::tool::ToolPolicyMutator::record_persisted_tool_policy_results(
        &mut reg, &persisted,
    );
    assert_eq!(
        persisted.results[0].status,
        ToolPolicyMutationStatus::Applied
    );
    assert_eq!(
        reg.metadata.get("first_time_uds").unwrap().configured_scope,
        Some(ProfileAvailabilityScope::Parent),
        "first persist:true mutation must immediately install the live configured ceiling"
    );
    assert_eq!(
        reg.persisted_policy_scopes.get(&stable_id),
        Some(&ProfileAvailabilityScope::Parent),
        "first persist:true mutation must be retained by stable id for reconnects"
    );

    reg.unregister_runtime_tool("first_time_uds");
    assert!(reg.register_uds_tool_for_owner_with_stable_id(
        Arc::new(DummyTestTool::new("first_time_uds")),
        "owner".into(),
        Some(stable_id),
    ));
    assert_eq!(
        ToolRegistryImpl::effective_scope(reg.metadata.get("first_time_uds").unwrap()),
        ProfileAvailabilityScope::Parent,
        "UDS/MCP reconnect before restart must reapply the just-persisted ceiling"
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
