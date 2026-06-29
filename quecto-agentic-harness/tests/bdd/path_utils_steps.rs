use super::*;
use quecto::infrastructure::tools::path_utils::{resolve_read_path, resolve_to_cwd};
use std::path::PathBuf;

// ===========================================================================
// Given steps
// ===========================================================================

#[given("a workspace directory at a temp path")]
fn given_workspace_temp(world: &mut QuectoWorld) {
    let td = TempDir::new().expect("create temp dir");
    world.tool_workspace = Some(td.path().to_path_buf());
    world._temp_dir = Some(td);
}

// ===========================================================================
// When steps
// ===========================================================================

#[when(expr = "I resolve path {string} relative to the workspace")]
fn when_resolve_path(world: &mut QuectoWorld, path: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("workspace not set")
        .clone();
    let resolved = resolve_to_cwd(&path, &ws);
    world.path_utils_resolved = Some(resolved);
}

#[when(expr = "I resolve path containing a non-breaking space in name {string}")]
fn when_resolve_path_nbsp(world: &mut QuectoWorld, raw_name: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("workspace not set")
        .clone();
    // The feature file encodes \u00A0 as a literal Unicode escape; we handle
    // both the raw char and the escape sequence representation.
    let name = raw_name.replace("\\u00A0", "\u{00A0}");
    let resolved = resolve_to_cwd(&name, &ws);
    world.path_utils_resolved = Some(resolved);
}

#[when(expr = "I resolve read path {string} relative to the workspace")]
fn when_resolve_read_path(world: &mut QuectoWorld, path: String) {
    let ws = world
        .tool_workspace
        .as_ref()
        .expect("workspace not set")
        .clone();
    let resolved = resolve_read_path(&path, &ws);
    world.path_utils_resolved = Some(resolved);
}

// ===========================================================================
// Then steps
// ===========================================================================

#[then(expr = "the resolved path should equal workspace joined with {string}")]
fn then_resolved_equals_workspace_joined(world: &mut QuectoWorld, suffix: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let expected = ws.join(&suffix);
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &expected,
        "expected '{}', got '{}'",
        expected.display(),
        actual.display()
    );
}

#[then(expr = "the resolved path should be {string}")]
fn then_resolved_equals_literal(world: &mut QuectoWorld, expected: String) {
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &PathBuf::from(&expected),
        "expected '{}', got '{}'",
        expected,
        actual.display()
    );
}

#[then("the resolved path should equal the home directory")]
fn then_resolved_is_home(world: &mut QuectoWorld) {
    let home = dirs::home_dir().expect("no home dir");
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &home,
        "expected home '{}', got '{}'",
        home.display(),
        actual.display()
    );
}

#[then(expr = "the resolved path should equal the home directory joined with {string}")]
fn then_resolved_is_home_joined(world: &mut QuectoWorld, suffix: String) {
    let home = dirs::home_dir().expect("no home dir");
    let expected = home.join(&suffix);
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &expected,
        "expected '{}', got '{}'",
        expected.display(),
        actual.display()
    );
}

#[then("the resolved path should equal the workspace root")]
fn then_resolved_is_workspace_root(world: &mut QuectoWorld) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let expected = ws.join(".");
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &expected,
        "expected '{}', got '{}'",
        expected.display(),
        actual.display()
    );
}

#[then("the resolved read path should exist on disk")]
fn then_read_path_exists(world: &mut QuectoWorld) {
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert!(
        actual.exists(),
        "expected '{}' to exist on disk",
        actual.display()
    );
}

#[then(expr = "the resolved read path should equal workspace joined with {string}")]
fn then_read_path_equals_workspace_joined(world: &mut QuectoWorld, suffix: String) {
    let ws = world.tool_workspace.as_ref().expect("workspace not set");
    let expected = ws.join(&suffix);
    let actual = world
        .path_utils_resolved
        .as_ref()
        .expect("no resolved path");
    assert_eq!(
        actual,
        &expected,
        "expected '{}', got '{}'",
        expected.display(),
        actual.display()
    );
}
