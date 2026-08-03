use std::sync::Arc;

use super::*;
use quecto::domain::tool_descriptor::ToolAvailability;
use quecto::infrastructure::tools::registration::ToolRegistration;

#[given(expr = "a bundled native tool named {string} from provider {string}")]
fn bundled_native_tool_from_provider(world: &mut QuectoWorld, name: String, provider: String) {
    let mut registry = ToolRegistryImpl::new();
    let tool = Arc::new(MockBddTool::new(&name, "ok"));
    assert!(registry.register_with_metadata(
        tool,
        ToolRegistration::official_native().with_provider_id(provider),
    ));
    world.tool_registry = Some(registry);
}

#[given(expr = "a bundled native tool named {string} from provider {string} with alias {string}")]
fn bundled_native_tool_from_provider_with_alias(
    world: &mut QuectoWorld,
    name: String,
    provider: String,
    alias: String,
) {
    let mut registry = ToolRegistryImpl::new();
    let tool = Arc::new(MockBddTool::new(&name, "ok"));
    assert!(
        registry.register_with_metadata(
            tool,
            ToolRegistration::official_native()
                .with_provider_id(provider)
                .with_alias(alias),
        )
    );
    world.tool_registry = Some(registry);
}

#[given("two providers register tools with the same stable id")]
fn two_providers_register_same_stable_id(world: &mut QuectoWorld) {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_with_metadata(
        Arc::new(MockBddTool::new("weather", "ok")),
        ToolRegistration::uds_owner("uds:client-a"),
    ));
    world.tool_policy_change_result = Some(
        registry.register_with_metadata(
            Arc::new(MockBddTool::new("weather_v2", "ok")),
            ToolRegistration::uds_owner("uds:client-b")
                .with_stable_id("tool.v1:uds:uds:client-a:weather"),
        ),
    );
    world.tool_registry = Some(registry);
}

#[given(expr = "UDS tools named {string} from providers {string} and {string}")]
fn uds_tools_named_from_providers(world: &mut QuectoWorld, name: String, a: String, b: String) {
    let mut registry = ToolRegistryImpl::new();
    assert!(registry.register_with_metadata(
        Arc::new(MockBddTool::new(&name, "ok")),
        ToolRegistration::uds_owner(a.clone()).with_provider_id(a),
    ));
    assert!(registry.register_with_metadata(
        Arc::new(MockBddTool::new(&format!("{name}_other"), "ok")),
        ToolRegistration::uds_owner(b.clone()).with_provider_id(b),
    ));
    world.tool_registry = Some(registry);
}

#[when("the tool catalogue is requested")]
fn tool_catalogue_is_requested(_world: &mut QuectoWorld) {}

#[when(expr = "policy disables legacy tool id {string}")]
fn policy_disables_legacy_tool_id(world: &mut QuectoWorld, id: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let warnings = registry.apply_startup_tool_restrictions(&[id]);
    world.tool_policy_change_result = Some(warnings.is_empty());
}

#[when(expr = "policy disables stable tool id {string}")]
fn policy_disables_stable_tool_id(world: &mut QuectoWorld, id: String) {
    let registry = world.tool_registry.as_mut().expect("tool registry not set");
    let warnings = registry.apply_startup_tool_restrictions(&[id]);
    world.tool_policy_change_result = Some(warnings.is_empty());
}

#[when("the second tool is registered")]
fn the_second_tool_is_registered(_world: &mut QuectoWorld) {}

#[then(expr = "the catalogue entry for {string} should have stable id {string}")]
fn catalogue_entry_should_have_stable_id(world: &mut QuectoWorld, name: String, stable_id: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let entry = registry
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == name)
        .expect("catalogue entry not found");
    assert_eq!(entry.stable_id, stable_id);
}

#[then(expr = "the catalogue entry for {string} should be disabled")]
fn catalogue_entry_should_be_disabled(world: &mut QuectoWorld, name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let entry = registry
        .catalogue_entries()
        .into_iter()
        .find(|entry| entry.name == name)
        .expect("catalogue entry not found");
    assert_eq!(entry.runtime_availability, ToolAvailability::Disabled);
}

#[then("registration should be rejected")]
fn registration_should_be_rejected(world: &mut QuectoWorld) {
    assert_eq!(world.tool_policy_change_result, Some(false));
}

#[then(expr = "only provider {string} tool {string} should be disabled")]
fn only_provider_tool_should_be_disabled(world: &mut QuectoWorld, provider: String, name: String) {
    let registry = world.tool_registry.as_ref().expect("tool registry not set");
    let entries = registry.catalogue_entries();
    let target = entries
        .iter()
        .find(|entry| entry.name == name && entry.provider_id == provider)
        .expect("target entry not found");
    assert_eq!(target.runtime_availability, ToolAvailability::Disabled);
    for entry in entries
        .iter()
        .filter(|entry| entry.name != target.name || entry.provider_id != target.provider_id)
    {
        assert_eq!(entry.runtime_availability, ToolAvailability::Enabled);
    }
}

#[then("the unknown policy id should be reported")]
fn unknown_policy_id_should_be_reported(world: &mut QuectoWorld) {
    assert_eq!(world.tool_policy_change_result, Some(false));
}
