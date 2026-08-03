use std::sync::Arc;

use super::{ToolRegistration, ToolRegistryImpl};
use crate::domain::tool::{ToolPolicyApplyMode, ToolPolicyMutation, ToolPolicyMutationStatus};
use crate::domain::tool_descriptor::ToolAvailability;
use crate::infrastructure::tools::registry::tests::DummyTestTool;

#[test]
fn legacy_name_policy_id_disables_canonical_tool() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("bash")),
        ToolRegistration::official_native().with_provider_id("quecto:official-tools"),
    ));

    assert!(
        reg.apply_startup_tool_restrictions(&["tool.name.v0:bash".into()])
            .is_empty()
    );

    let entry = reg.catalogue_entries().pop().unwrap();
    assert_eq!(
        entry.stable_id,
        "tool.v1:bundled-native:21:quecto:official-tools:bash"
    );
    assert_eq!(entry.runtime_availability, ToolAvailability::Disabled);
}

#[test]
fn renamed_alias_policy_id_disables_canonical_tool() {
    let mut reg = ToolRegistryImpl::new();
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );

    assert!(
        reg.apply_startup_tool_restrictions(&["tool.name.v0:view".into()])
            .is_empty()
    );

    let entry = reg.catalogue_entries().pop().unwrap();
    assert_eq!(entry.name, "read");
    assert_eq!(entry.runtime_availability, ToolAvailability::Disabled);
}

#[test]
fn duplicate_stable_ids_are_rejected() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a"),
    ));
    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_v2")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-a:weather"),
        )
    );
}

#[test]
fn same_tool_names_from_different_providers_get_distinct_stable_ids() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_provider_id("uds:client-a"),
    ));
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_other")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_provider_id("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-b:weather"),
        )
    );

    let entries = reg.catalogue_entries();
    let ids: Vec<_> = entries
        .iter()
        .map(|entry| entry.stable_id.as_ref())
        .collect();
    assert!(ids.contains(&"tool.v1:uds:12:uds:client-a:weather"));
    assert!(ids.contains(&"tool.v1:uds:12:uds:client-b:weather"));
}

#[test]
fn provider_namespace_collision_keeps_stable_ids_distinct() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_provider_id("uds:client-a"),
    ));
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_other")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_provider_id("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-b:weather"),
        )
    );

    assert!(
        reg.apply_startup_tool_restrictions(&["tool.v1:uds:12:uds:client-a:weather".into()])
            .is_empty()
    );

    let entries = reg.catalogue_entries();
    let a = entries
        .iter()
        .find(|entry| entry.name == "weather")
        .unwrap();
    let b = entries
        .iter()
        .find(|entry| entry.name == "weather_other")
        .unwrap();
    assert_eq!(a.runtime_availability, ToolAvailability::Disabled);
    assert_eq!(b.runtime_availability, ToolAvailability::Enabled);
}

#[test]
fn unknown_policy_ids_are_reported() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("bash")),
        ToolRegistration::official_native().with_provider_id("quecto:official-tools"),
    ));

    let warnings = reg.apply_startup_tool_restrictions(&[
        "tool.v1:bundled-native:21:quecto:official-tools:missing".into(),
    ]);

    assert_eq!(
        warnings,
        vec!["tool.v1:bundled-native:21:quecto:official-tools:missing"]
    );
    assert_eq!(
        reg.catalogue_entries()[0].runtime_availability,
        ToolAvailability::Enabled
    );
    assert!(!reg.register_with_metadata(
        Arc::new(DummyTestTool::new(
            "tool.v1:bundled-native:21:quecto:official-tools:missing"
        )),
        ToolRegistration::official_native().with_provider_id("quecto:official-tools"),
    ));
    assert!(!reg.register_with_metadata(
        Arc::new(DummyTestTool::new("missing")),
        ToolRegistration::official_native().with_provider_id("quecto:official-tools"),
    ));
}

#[test]
fn pre_registration_legacy_alias_policy_id_blocks_later_renamed_tool() {
    let mut reg = ToolRegistryImpl::new();

    let warnings = reg.apply_startup_tool_restrictions(&["tool.name.v0:view".into()]);

    assert_eq!(warnings, vec!["tool.name.v0:view"]);
    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );
    assert!(reg.catalogue_entries().is_empty());
}

#[test]
fn unknown_stable_policy_id_blocks_later_matching_provider_registration() {
    let mut reg = ToolRegistryImpl::new();

    let warnings =
        reg.apply_startup_tool_restrictions(&["tool.v1:uds:12:uds:client-a:weather".into()]);

    assert_eq!(warnings, vec!["tool.v1:uds:12:uds:client-a:weather"]);
    assert!(!reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_provider_id("uds:client-a"),
    ));
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_other")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_provider_id("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-b:weather"),
        )
    );
}

#[test]
fn registry_rejects_alias_collisions() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a").with_alias("forecast"),
    ));
    assert!(!reg.register_with_metadata(
        Arc::new(DummyTestTool::new("forecast")),
        ToolRegistration::uds_owner("uds:client-b"),
    ));
}

#[test]
fn live_policy_mutations_resolve_stable_legacy_alias_and_unknown_ids() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("bash")),
        ToolRegistration::official_native().with_provider_id("quecto:official-tools"),
    ));
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );

    let reconciliation = reg.apply_tool_policy_mutations(
        &[
            ToolPolicyMutation::disable(
                "tool.v1:bundled-native:21:quecto:official-tools:bash",
                "stable",
            ),
            ToolPolicyMutation::disable("tool.name.v0:read", "legacy"),
            ToolPolicyMutation::disable("view", "alias"),
            ToolPolicyMutation::disable(
                "tool.v1:bundled-native:21:quecto:official-tools:missing",
                "missing",
            ),
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
    assert_eq!(
        reconciliation.results[2].status,
        ToolPolicyMutationStatus::AlreadyInState
    );
    assert_eq!(
        reconciliation.results[3].status,
        ToolPolicyMutationStatus::UnknownTool
    );
}

#[test]
fn unknown_raw_name_policy_id_blocks_later_legacy_alias_registration() {
    let mut reg = ToolRegistryImpl::new();

    let warnings = reg.apply_startup_tool_restrictions(&["view".into()]);

    assert_eq!(warnings, vec!["view"]);
    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );
}

#[test]
fn startup_disable_by_known_stable_id_blocks_unload_then_reintroduce_with_disabled_alias() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a"),
    ));

    assert!(
        reg.apply_startup_tool_restrictions(&["tool.v1:uds:12:uds:client-a:weather".into()])
            .is_empty()
    );
    reg.unregister_runtime_tool("weather");

    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_v2")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-b:weather_v2")
                .with_alias("weather"),
        )
    );
}

#[test]
fn remove_registered_stable_id_tool_blocks_reintroducing_same_stable_id_under_new_name() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("weather")),
        ToolRegistration::uds_owner("uds:client-a"),
    ));

    assert!(reg.remove("weather"));

    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("weather_v2")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_stable_id("tool.v1:uds:12:uds:client-a:weather"),
        )
    );
}

#[test]
fn remove_all_registered_alias_tool_blocks_reintroducing_removed_alias() {
    let mut reg = ToolRegistryImpl::new();
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );

    assert!(reg.remove_all(&["read".into()]).is_empty());

    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("open")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );
}

#[test]
fn remove_unknown_raw_name_blocks_later_legacy_alias_registration() {
    let mut reg = ToolRegistryImpl::new();

    assert!(!reg.remove("view"));

    assert!(
        !reg.register_with_metadata(
            Arc::new(DummyTestTool::new("read")),
            ToolRegistration::official_native()
                .with_provider_id("quecto:official-tools")
                .with_alias("view"),
        )
    );
}
