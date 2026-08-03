use std::sync::Arc;

use super::{ToolRegistration, ToolRegistryImpl};
use crate::domain::tool_descriptor::{
    ToolAvailability, ToolHealth, ToolLifecycleKind, ToolRestrictionReason, ToolSource,
};
use crate::infrastructure::tools::registry::tests::DummyTestTool;

fn registry_with_startup_disabled_native_and_uds() -> ToolRegistryImpl {
    let mut reg = ToolRegistryImpl::new();
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("native")),
            ToolRegistration::official_native().with_provider_id("quecto:test-native"),
        ),
        "native fixture registration should succeed"
    );
    assert!(
        reg.register_with_metadata(
            Arc::new(DummyTestTool::new("uds_tool")),
            ToolRegistration::uds_owner("uds:client-1"),
        ),
        "uds fixture registration should succeed"
    );
    assert!(
        reg.apply_startup_tool_restrictions(&["native".to_string()])
            .is_empty()
    );
    reg
}

#[test]
fn catalogue_entries_distinguish_runtime_lifecycle_and_effective_state() {
    let reg = registry_with_startup_disabled_native_and_uds();
    let entries = reg.catalogue_entries();
    let native = entries.iter().find(|entry| entry.name == "native").unwrap();
    assert_eq!(
        native.stable_id,
        "tool.v1:bundled-native:quecto:test-native:native"
    );
    assert_eq!(native.source, ToolSource::BundledNative);
    assert_eq!(native.owner, "quecto:official-tools");
    assert_eq!(native.provider_id, "quecto:test-native");
    assert_eq!(native.lifecycle, ToolLifecycleKind::Bundled);
    assert!(native.configurable);
    assert!(native.default_enabled);
    assert_eq!(
        (
            native.configured_enabled,
            native.profile_enabled,
            native.session_enabled,
            native.explicit_restriction,
            native.version.as_ref(),
        ),
        (
            None,
            None,
            Some(false),
            Some(ToolRestrictionReason::ExplicitDisable),
            None,
        )
    );
    assert_eq!(native.runtime_availability, ToolAvailability::Disabled);
    assert!(!native.effective_enabled);
    assert_eq!(native.health, ToolHealth::Disabled);

    let uds = entries
        .iter()
        .find(|entry| entry.name == "uds_tool")
        .unwrap();
    assert_eq!(uds.stable_id, "tool.v1:uds:uds:client-1:uds_tool");
    assert_eq!(uds.source, ToolSource::Uds);
    assert_eq!(uds.owner, "uds:client-1");
    assert_eq!(uds.provider_id, "uds:client-1");
    assert_eq!(uds.lifecycle, ToolLifecycleKind::RuntimeLoadable);
    assert!(uds.default_enabled);
    assert_eq!(
        (
            uds.configured_enabled,
            uds.profile_enabled,
            uds.session_enabled,
            uds.explicit_restriction,
        ),
        (None, None, None, None)
    );
    assert_eq!(uds.runtime_availability, ToolAvailability::Enabled);
    assert!(uds.effective_enabled);
    assert_eq!(uds.health, ToolHealth::Ok);

    let serialized = serde_json::to_value(uds).unwrap();
    assert_eq!(serialized["stableId"], "tool.v1:uds:uds:client-1:uds_tool");
    assert_eq!(serialized["source"], "uds");
    assert_eq!(serialized["lifecycle"], "runtime-loadable");
    assert_eq!(serialized["runtimeAvailability"], "enabled");
    assert_eq!(serialized["effectiveEnabled"], true);
    assert_eq!(serialized["health"], "ok");
}

#[test]
fn spawn_restrictions_preserve_spawn_provenance() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("write")),
        ToolRegistration::official_native().with_provider_id("quecto:session-tools"),
    ));
    assert!(
        reg.apply_spawn_tool_restrictions(&["write".to_string()])
            .is_empty()
    );

    let entries = reg.catalogue_entries();
    let write = entries.iter().find(|entry| entry.name == "write").unwrap();
    assert_eq!(write.session_enabled, Some(false));
    assert_eq!(
        write.explicit_restriction,
        Some(ToolRestrictionReason::Spawn)
    );
    assert_eq!(write.runtime_availability, ToolAvailability::Disabled);
    assert!(!write.effective_enabled);
}

#[test]
fn enable_tool_restores_runtime_disabled_tool_metadata() {
    let mut reg = ToolRegistryImpl::new();
    assert!(reg.register_with_metadata(
        Arc::new(DummyTestTool::new("native")),
        ToolRegistration::official_native().with_provider_id("quecto:test-native"),
    ));
    assert!(reg.disable_tool("native"));

    assert!(reg.enable_tool("native"));

    let entries = reg.catalogue_entries();
    let native = entries.iter().find(|entry| entry.name == "native").unwrap();
    assert_eq!(native.runtime_availability, ToolAvailability::Enabled);
    assert!(native.effective_enabled);
    assert_eq!(native.session_enabled, None);
    assert_eq!(native.explicit_restriction, None);
    assert_eq!(native.health, ToolHealth::Ok);
}

#[test]
fn enable_tool_preserves_startup_restriction_metadata() {
    let mut reg = registry_with_startup_disabled_native_and_uds();

    assert!(reg.enable_tool("native"));

    let entries = reg.catalogue_entries();
    let native = entries.iter().find(|entry| entry.name == "native").unwrap();
    assert_eq!(native.runtime_availability, ToolAvailability::Disabled);
    assert!(!native.effective_enabled);
    assert_eq!(native.session_enabled, Some(false));
    assert_eq!(
        native.explicit_restriction,
        Some(ToolRestrictionReason::ExplicitDisable)
    );
    assert_eq!(native.health, ToolHealth::Disabled);
}
