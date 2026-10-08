use super::*;
use crate::domain::tool_policy::value_objects::tool_descriptor::{
    ToolAvailability, ToolHealth, ToolLifecycleKind,
};

fn uds_entry(name: &str, stable_id: String, scope: ProfileAvailabilityScope) -> ToolCatalogueEntry {
    ToolCatalogueEntry {
        stable_id: stable_id.into(),
        name: name.to_string().into(),
        label: name.to_string().into(),
        description: "d".into(),
        input_schema: "{}".into(),
        source: ToolSource::Uds,
        owner: "uds:client:7".into(),
        provider_id: "uds:client:7".into(),
        version: None,
        lifecycle: ToolLifecycleKind::RuntimeLoadable,
        configurable: true,
        default_enabled: true,
        configured_enabled: None,
        profile_enabled: None,
        profile_scope: None,
        session_enabled: None,
        explicit_restriction: None,
        runtime_availability: ToolAvailability::Enabled,
        effective_enabled: scope != ProfileAvailabilityScope::None,
        effective_scope: scope,
        effective_parent_enabled: scope.allows_parent(),
        effective_child_enabled: scope.allows_child(),
        health: ToolHealth::Ok,
    }
}

/// #2446: a configured extension's tool is recorded by its stable id (the
/// same in every agent) and its extension by its key, with the widest
/// scope among its tools, so a child's own instance is covered even when
/// it names its tools differently.
#[test]
fn a_configured_extensions_tools_record_the_extension_with_their_widest_scope() {
    let snapshot = inherited_child_policy_from_catalogue([
        uds_entry(
            "browser_click",
            configured_extension_tool_id("browser-task", "browser_click"),
            ProfileAvailabilityScope::Parent,
        ),
        uds_entry(
            "browser_open",
            configured_extension_tool_id("browser-task", "browser_open"),
            ProfileAvailabilityScope::Child,
        ),
    ]);
    assert_eq!(
        snapshot.get("uds:extension:browser-task"),
        Some(&ProfileAvailabilityScope::Both)
    );
    assert_eq!(
        snapshot.get(&configured_extension_tool_id(
            "browser-task",
            "browser_click"
        )),
        Some(&ProfileAvailabilityScope::Parent),
        "a same-named tool keeps the parent's own scope"
    );
    assert_eq!(
        snapshot.get("browser_click"),
        None,
        "not by name: the id is stable"
    );
}

#[test]
fn only_a_configured_extensions_uds_id_has_an_extension_key() {
    let id = configured_extension_tool_id("stand-in", "echo_main");
    assert_eq!(
        configured_extension_key(&id),
        Some("uds:extension:stand-in")
    );
    let plain = stable_tool_id(ToolSource::Uds, "uds:client:7", "echo");
    assert_eq!(configured_extension_key(&plain), None);
    let runtime = stable_tool_id(ToolSource::Runtime, "uds:extension:x", "echo");
    assert_eq!(configured_extension_key(&runtime), None);
    assert_eq!(configured_extension_key("uds:extension:x"), None);
    let snapshot = inherited_child_policy_from_catalogue([uds_entry(
        "echo",
        plain,
        ProfileAvailabilityScope::Both,
    )]);
    assert_eq!(
        snapshot.len(),
        2,
        "a plain UDS tool records id and name only"
    );
}

#[test]
fn late_registering_keys_are_told_apart_from_typos() {
    assert!(registers_after_startup("uds:extension:stand-in"));
    assert!(registers_after_startup(&configured_extension_tool_id(
        "stand-in", "echo"
    )));
    assert!(registers_after_startup(&stable_tool_id(
        ToolSource::Runtime,
        "runtime:extension",
        "x"
    )));
    assert!(!registers_after_startup(&stable_tool_id(
        ToolSource::BundledNative,
        "quecto:official-tools",
        "bash"
    )));
    assert!(!registers_after_startup("bash"));
}
