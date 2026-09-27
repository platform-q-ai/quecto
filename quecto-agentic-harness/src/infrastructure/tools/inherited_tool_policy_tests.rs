use std::collections::BTreeMap;
use std::sync::Arc;

use super::tests::{DummyTestTool, test_registry};
use crate::domain::tool::{
    ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus, ToolProfileContext,
};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot;
use crate::infrastructure::tools::registration::ToolRegistration;

/// The child policy this registry hands spawn: the catalogue fold completed
/// with the registry's policy over entrypoint-only tools.
fn spawn_child_policy(reg: &super::ToolRegistryImpl) -> BTreeMap<String, ProfileAvailabilityScope> {
    let mut snapshot =
        crate::domain::tool_policy::inherited_child_policy_from_catalogue(reg.catalogue_entries());
    reg.record_unbuilt_entrypoint_only_policy(&mut snapshot);
    snapshot
}

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

#[test]
fn inherited_snapshot_matches_uds_registration_by_stable_id_before_name() {
    let (mut reg, _tmp) = test_registry();
    reg.set_execution_profile_context(ToolProfileContext::Child);
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_stable_id("com.example.weather.v1"),
    ));
    let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
        "com.example.weather.v1".to_string(),
        ProfileAvailabilityScope::Child,
    )]));

    assert!(
        reg.apply_inherited_tool_policy_snapshot(&snapshot)
            .is_empty()
    );
    let entry = reg.catalogue_entry("weather").unwrap();
    assert_eq!(entry.profile_scope, Some(ProfileAvailabilityScope::Child));
    assert!(entry.effective_child_enabled);
    assert!(!entry.effective_parent_enabled);

    let late_snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
        "com.example.renamed-weather.v1".to_string(),
        ProfileAvailabilityScope::Child,
    )]));
    assert_eq!(
        reg.apply_inherited_tool_policy_snapshot(&late_snapshot),
        vec!["com.example.renamed-weather.v1".to_string()]
    );
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("renamed_weather")),
            ToolRegistration::uds_owner("uds:client-a")
                .with_stable_id("com.example.renamed-weather.v1"),
        )
    );
    let late_entry = reg.catalogue_entry("renamed_weather").unwrap();
    assert_eq!(
        late_entry.profile_scope,
        Some(ProfileAvailabilityScope::Child)
    );
    assert!(late_entry.effective_child_enabled);
    assert!(!late_entry.effective_parent_enabled);
}

#[test]
fn inherited_spawn_snapshot_uses_stable_ids_for_uds_tools() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_stable_id("com.example.weather.v1"),
    ));

    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "com.example.weather.v1",
            ProfileAvailabilityScope::Child,
            "inherit stable id",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    let spawn = spawn_child_policy(&reg);
    assert_eq!(
        spawn.get("com.example.weather.v1"),
        Some(&ProfileAvailabilityScope::Child)
    );
    assert!(
        !spawn.contains_key("weather"),
        "inherited child policy should persist the stable id rather than the transient UDS name"
    );
}

#[test]
fn inherited_spawn_snapshot_includes_name_alias_for_legacy_uds_generated_stable_id() {
    let (mut reg, _tmp) = test_registry();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:parent-client"),
    ));

    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "weather",
            ProfileAvailabilityScope::Child,
            "inherit legacy UDS name fallback",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );

    let spawn = spawn_child_policy(&reg);
    assert_eq!(
        spawn.get("tool.v1:uds:17:uds:parent-client:weather"),
        Some(&ProfileAvailabilityScope::Child)
    );
    assert_eq!(
        spawn.get("weather"),
        Some(&ProfileAvailabilityScope::Child),
        "legacy/name-only UDS tools need raw-name fallback when child owner generates a different stable id"
    );
}

fn child_names(reg: &super::ToolRegistryImpl) -> Vec<String> {
    reg.definitions_for(ToolProfileContext::Child)
        .iter()
        .map(|d| d.name.to_string())
        .collect()
}

fn bundled_workflow() -> ToolRegistration {
    ToolRegistration::official_native()
        .with_provider_id(crate::infrastructure::tools::inherited_tool_policy::WORKFLOW_PROVIDER_ID)
}

#[test]
fn entrypoint_only_workflow_absent_from_snapshot_stays_visible_even_when_late() {
    for late in [false, true] {
        let (mut reg, _tmp) = test_registry();
        reg.set_execution_profile_context(ToolProfileContext::Child);
        if !late {
            assert!(reg.register_with_metadata(
                Arc::new(DummyTestTool::new("workflow")),
                bundled_workflow()
            ));
        }
        let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
            "read".to_string(),
            ProfileAvailabilityScope::Both,
        )]));
        assert!(
            reg.apply_inherited_tool_policy_snapshot(&snapshot)
                .is_empty()
        );
        if late {
            assert!(reg.register_with_metadata(
                Arc::new(DummyTestTool::new("workflow")),
                bundled_workflow()
            ));
        }
        let names = child_names(&reg);
        assert!(names.contains(&"workflow".to_string()), "late={late}");
        assert!(!names.contains(&"bash".to_string()), "late={late}");
        let entry = reg.catalogue_entry("workflow").unwrap();
        assert!(entry.effective_child_enabled, "late={late}");
        assert_ne!(entry.profile_scope, Some(ProfileAvailabilityScope::None));
    }
}

#[test]
fn entrypoint_only_workflow_recorded_in_snapshot_keeps_its_ceiling() {
    let (mut reg, _tmp) = test_registry();
    reg.set_execution_profile_context(ToolProfileContext::Child);
    assert!(
        reg.register_with_metadata(Arc::new(DummyTestTool::new("workflow")), bundled_workflow())
    );
    let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
        "workflow".to_string(),
        ProfileAvailabilityScope::Parent,
    )]));
    assert!(
        reg.apply_inherited_tool_policy_snapshot(&snapshot)
            .is_empty()
    );
    assert!(!child_names(&reg).contains(&"workflow".to_string()));
    let reconciliation = reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "workflow",
            ProfileAvailabilityScope::Both,
            "try widen",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    assert_eq!(
        reconciliation.results[0].status,
        ToolPolicyMutationStatus::BlockedByRestriction
    );
}

#[test]
fn tools_posing_as_the_workflow_tool_stay_closed() {
    let workflow_id = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity()
        .stable_id
        .into_owned();
    let posers = [
        (
            "workflow",
            ToolRegistration::uds_owner("uds:client-a"),
            "a UDS tool sharing the name",
        ),
        (
            "wf",
            ToolRegistration::official_native().with_stable_id(workflow_id.clone()),
            "a bundled tool claiming the stable id (a UDS claim is refused outright)",
        ),
        (
            "workflow",
            ToolRegistration::uds_owner(
                crate::infrastructure::tools::inherited_tool_policy::WORKFLOW_PROVIDER_ID,
            ),
            "a UDS tool whose owner is the workflow provider",
        ),
        (
            "workflow",
            ToolRegistration::official_native(),
            "a bundled tool from another provider",
        ),
        (
            "workflow",
            bundled_workflow().with_stable_id("com.example.workflow.v1"),
            "a bundled workflow with an overridden stable id",
        ),
    ];
    for (name, registration, case) in posers {
        let (mut reg, _tmp) = test_registry();
        reg.set_execution_profile_context(ToolProfileContext::Child);
        assert!(
            reg.register_with_metadata(Arc::new(DummyTestTool::new(name)), registration),
            "{case}"
        );
        let snapshot = InheritedToolPolicySnapshot::new(BTreeMap::from([(
            "read".to_string(),
            ProfileAvailabilityScope::Both,
        )]));
        reg.apply_inherited_tool_policy_snapshot(&snapshot);
        assert!(!child_names(&reg).contains(&name.to_string()), "{case}");
    }
}

#[test]
fn spawn_snapshot_records_a_denied_entrypoint_only_tool_the_parent_never_built() {
    let identity = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity();
    let (mut reg, _tmp) = test_registry();
    assert!(
        !spawn_child_policy(&reg).contains_key(identity.stable_id.as_ref()),
        "an unbuilt, undenied tool is simply absent"
    );
    for denial in [
        "workflow".to_string(),
        identity.stable_id.to_string(),
        identity.legacy_name_id.to_string(),
    ] {
        let (mut reg, _tmp) = test_registry();
        reg.apply_startup_tool_restrictions(std::slice::from_ref(&denial));
        let snapshot = spawn_child_policy(&reg);
        assert_eq!(
            snapshot.get(identity.stable_id.as_ref()),
            Some(&ProfileAvailabilityScope::None),
            "denied as {denial}"
        );
        assert_eq!(
            snapshot.get("workflow"),
            Some(&ProfileAvailabilityScope::None)
        );
    }
    let mut policy = crate::infrastructure::config::ToolPolicyConfig::default();
    policy.entries.insert(
        identity.stable_id.to_string(),
        crate::infrastructure::config::ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Parent,
        },
    );
    reg.apply_persisted_tool_policy(&policy);
    assert_eq!(
        spawn_child_policy(&reg).get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::Parent),
        "a persisted preference caps children as it would a built tool"
    );
}

#[test]
fn recorded_denial_overrides_a_bundled_tool_claiming_the_workflow_stable_id() {
    let identity = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity();
    let (mut reg, _tmp) = test_registry();
    reg.apply_startup_tool_restrictions(&["workflow".to_string()]);
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("wf_claimant")),
            ToolRegistration::official_native().with_stable_id(identity.stable_id.clone()),
        ),
        "a bundled registration may use a bundled-native id"
    );
    let snapshot = spawn_child_policy(&reg);
    assert_eq!(
        snapshot.get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::None),
        "the denial wins over the claimant's recorded scope"
    );
}

#[test]
fn only_bundled_native_registrations_use_bundled_native_stable_ids() {
    let bundled_id = "tool.v1:bundled-native:13:quecto:thirdp:tool";
    for (registration, accepted, case) in [
        (
            ToolRegistration::uds_owner("uds:client-a").with_stable_id(bundled_id),
            false,
            "uds",
        ),
        (
            ToolRegistration::runtime("runtime:x").with_stable_id(bundled_id),
            false,
            "runtime",
        ),
        (
            ToolRegistration::official_native().with_stable_id(bundled_id),
            true,
            "bundled",
        ),
        (
            ToolRegistration::uds_owner("uds:client-a").with_stable_id("com.example.tool.v1"),
            true,
            "uds outside the namespace",
        ),
    ] {
        let (mut reg, _tmp) = test_registry();
        assert_eq!(
            reg.register_with_metadata(Arc::new(DummyTestTool::new("claimant")), registration),
            accepted,
            "{case}"
        );
    }
}

#[test]
fn registers_entrypoint_only_names_only_the_bundled_workflow_registration() {
    let (mut reg, _tmp) = test_registry();
    assert!(!reg.registers_entrypoint_only("workflow"), "not registered");
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("workflow")),
        ToolRegistration::official_native()
    ));
    assert!(
        !reg.registers_entrypoint_only("workflow"),
        "another provider"
    );
    let (mut reg, _tmp) = test_registry();
    assert!(
        reg.register_with_metadata(Arc::new(DummyTestTool::new("workflow")), bundled_workflow())
    );
    assert!(reg.registers_entrypoint_only("workflow"));
}

/// #2216 review 2: a same-named tool's name-key entry must not hide the
/// missing stable-id entry, so a persisted `none` for the unbuilt workflow
/// tool still caps children under both keys.
#[test]
fn persisted_workflow_denial_survives_a_same_named_runtime_tool() {
    let identity = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity();
    for persisted in [
        ProfileAvailabilityScope::None,
        ProfileAvailabilityScope::Parent,
    ] {
        let (mut reg, _tmp) = test_registry();
        let mut policy = crate::infrastructure::config::ToolPolicyConfig::default();
        policy.entries.insert(
            identity.stable_id.to_string(),
            crate::infrastructure::config::ToolPolicyEntryConfig { scope: persisted },
        );
        reg.apply_persisted_tool_policy(&policy);
        assert!(reg.register_with_metadata(
            Arc::new(DummyTestTool::new("workflow")),
            ToolRegistration::runtime("runtime:extension"),
        ));
        let snapshot = spawn_child_policy(&reg);
        assert_eq!(
            snapshot.get(identity.stable_id.as_ref()),
            Some(&persisted),
            "stable id"
        );
        assert_eq!(snapshot.get("workflow"), Some(&persisted), "name");
    }
}

/// The name key keeps the narrower of the persisted scope and a same-named
/// tool's own entry.
#[test]
fn persisted_workflow_scope_never_widens_a_same_named_tool_entry() {
    let identity = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity();
    let (mut reg, _tmp) = test_registry();
    let mut policy = crate::infrastructure::config::ToolPolicyConfig::default();
    policy.entries.insert(
        identity.stable_id.to_string(),
        crate::infrastructure::config::ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Child,
        },
    );
    reg.apply_persisted_tool_policy(&policy);
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("workflow")),
        ToolRegistration::runtime("runtime:extension"),
    ));
    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "workflow",
            ProfileAvailabilityScope::Parent,
            "narrow the runtime tool",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    let snapshot = spawn_child_policy(&reg);
    assert_eq!(
        snapshot.get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::None),
        "child ∩ parent"
    );
    assert_eq!(
        snapshot.get("workflow"),
        Some(&ProfileAvailabilityScope::None)
    );
}

/// A built workflow tool records its own effective scope: a live narrowing
/// stands over a wider persisted preference.
#[test]
fn built_workflow_records_its_effective_scope_over_its_persisted_preference() {
    let identity = crate::infrastructure::tools::inherited_tool_policy::workflow_tool_identity();
    let (mut reg, _tmp) = test_registry();
    let mut policy = crate::infrastructure::config::ToolPolicyConfig::default();
    policy.entries.insert(
        identity.stable_id.to_string(),
        crate::infrastructure::config::ToolPolicyEntryConfig {
            scope: ProfileAvailabilityScope::Both,
        },
    );
    reg.apply_persisted_tool_policy(&policy);
    assert!(
        reg.register_with_metadata(Arc::new(DummyTestTool::new("workflow")), bundled_workflow())
    );
    reg.apply_tool_policy_mutations(
        &[ToolPolicyMutation::set_scope(
            "workflow",
            ProfileAvailabilityScope::Parent,
            "keep workflow to the parent for now",
        )],
        ToolPolicyApplyMode::ImmediateIfIdle,
    );
    let snapshot = spawn_child_policy(&reg);
    assert_eq!(
        snapshot.get(identity.stable_id.as_ref()),
        Some(&ProfileAvailabilityScope::Parent)
    );
    assert_eq!(
        snapshot.get("workflow"),
        Some(&ProfileAvailabilityScope::Parent)
    );
}
