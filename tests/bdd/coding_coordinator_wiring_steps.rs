//! BDD step definitions for coordinator wiring via config flag.
//!
//! Tests that `build_coding_tool()` correctly routes to inline or subagent
//! mode based on `CoordinatorMode`, and that both modes register a
//! `coding_job` tool on the registry.

use cucumber::{given, then, when};

use quecto::infrastructure::config::{Config, CoordinatorMode};
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::registry::ToolRegistryImpl;
use quecto::interface::shared::build_coding_tool;

use crate::QuectoWorld;

// ============================================================================
// Given steps
// ============================================================================

#[given("a default config")]
fn given_default_config(world: &mut QuectoWorld) {
    let config: Config = serde_json::from_str("{}").expect("default config");
    world.wiring_config = Some(config);
}

#[given(regex = r#"^a config with coordinator mode "(\w+)"$"#)]
fn given_config_with_mode(world: &mut QuectoWorld, mode: String) {
    let json = format!(r#"{{ "tools": {{ "coding": {{ "coordinator_mode": "{mode}" }} }} }}"#);
    let config: Config = serde_json::from_str(&json).expect("parse config");
    world.wiring_config = Some(config);
}

#[given("a tool registry with workspace")]
fn given_tool_registry(world: &mut QuectoWorld) {
    setup_registry(world);
}

#[given("a fresh tool registry with workspace")]
fn given_fresh_tool_registry(world: &mut QuectoWorld) {
    setup_registry(world);
}

fn setup_registry(world: &mut QuectoWorld) {
    let td = tempfile::TempDir::new().expect("temp dir");
    let workspace = td.path().join("workspace");
    std::fs::create_dir_all(workspace.join("skills")).expect("create workspace/skills");

    let sandbox = Sandbox::new(Some(workspace.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);

    world.wiring_registry = Some(registry);
    world.wiring_driver = None;
    world._wiring_temp_dir = Some(td);
}

// ============================================================================
// When steps
// ============================================================================

#[when("I build the coding tool in inline mode")]
fn when_build_inline(world: &mut QuectoWorld) {
    build_with_mode(world, CoordinatorMode::Inline);
}

#[when("I build the coding tool in subagent mode")]
fn when_build_subagent(world: &mut QuectoWorld) {
    build_with_mode(world, CoordinatorMode::Subagent);
}

fn build_with_mode(world: &mut QuectoWorld, mode: CoordinatorMode) {
    let td = world._wiring_temp_dir.as_ref().expect("temp dir");
    let workspace = td.path().join("workspace");
    let registry = world.wiring_registry.as_mut().expect("registry");
    let driver = build_coding_tool(registry, &workspace, td.path(), mode);
    world.wiring_driver = driver;
}

// ============================================================================
// Then steps
// ============================================================================

#[then(regex = r#"^the coordinator mode should be "(\w+)"$"#)]
fn then_coordinator_mode(world: &mut QuectoWorld, expected: String) {
    let config = world.wiring_config.as_ref().expect("config");
    let actual = match config.tools.coding.coordinator_mode {
        CoordinatorMode::Inline => "inline",
        CoordinatorMode::Subagent => "subagent",
    };
    assert_eq!(actual, expected, "coordinator mode mismatch");
}

#[then(regex = r#"^the registry should contain a "([^"]+)" tool$"#)]
fn then_registry_contains_tool(world: &mut QuectoWorld, tool_name: String) {
    let registry = world.wiring_registry.as_ref().expect("registry");
    assert!(
        registry.definitions().iter().any(|d| d.name == tool_name),
        "registry should contain tool '{tool_name}', found: {:?}",
        registry
            .definitions()
            .iter()
            .map(|d| &d.name)
            .collect::<Vec<_>>()
    );
}

#[then("the lifecycle driver should be present")]
fn then_driver_present(world: &mut QuectoWorld) {
    assert!(
        world.wiring_driver.is_some(),
        "lifecycle driver should be present in inline mode"
    );
}

#[then("the lifecycle driver should not be present")]
fn then_driver_absent(world: &mut QuectoWorld) {
    assert!(
        world.wiring_driver.is_none(),
        "lifecycle driver should not be present in subagent mode"
    );
}
