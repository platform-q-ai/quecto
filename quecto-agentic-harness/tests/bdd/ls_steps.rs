// BDD step definitions for LsTool Quecto-compatible scenarios (issue #149).

use crate::QuectoWorld;
use cucumber::{given, then, when};
use quecto::domain::tool::Tool;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::filesystem::LsTool;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_ls_workspace(world: &mut QuectoWorld) -> PathBuf {
    if world.ls_workspace.is_none() {
        let tmp = TempDir::new().expect("failed to create ls temp dir");
        let path = tmp.path().to_path_buf();
        world._ls_temp_dir = Some(tmp);
        world.ls_workspace = Some(path);
    }
    world.ls_workspace.clone().unwrap()
}

fn make_ls_tool(world: &mut QuectoWorld) -> LsTool {
    let ws = ensure_ls_workspace(world);
    let ws_arc = Arc::new(ws.clone());
    let sandbox = Arc::new(Sandbox::new(Some(ws.clone())));
    LsTool::new(ws_arc, sandbox)
}

fn run_ls(tool: LsTool, args: serde_json::Value) -> quecto::domain::tool::ToolResult {
    tokio::runtime::Runtime::new()
        .unwrap()
        .block_on(async { tool.execute(&args.to_string()).await })
        .unwrap_or_else(|e| quecto::domain::tool::ToolResult {
            content: e.to_string(),
            is_error: true,
            image_blocks: vec![],
        })
}

// ---------------------------------------------------------------------------
// Given
// ---------------------------------------------------------------------------

#[given("an ls tool workspace")]
fn given_ls_workspace(world: &mut QuectoWorld) {
    ensure_ls_workspace(world);
}

#[given(regex = r#"^ls workspace file "([^"]+)"$"#)]
fn given_ls_file(world: &mut QuectoWorld, name: String) {
    let ws = ensure_ls_workspace(world);
    std::fs::write(ws.join(&name), "").expect("write ls file");
}

#[given(regex = r#"^ls workspace directory "([^"]+)"$"#)]
fn given_ls_directory(world: &mut QuectoWorld, name: String) {
    let ws = ensure_ls_workspace(world);
    std::fs::create_dir_all(ws.join(&name)).expect("create ls directory");
}

#[given(regex = r#"^ls workspace with (\d+) files named "file_NNN\.txt"$"#)]
fn given_ls_many_files(world: &mut QuectoWorld, count: usize) {
    let ws = ensure_ls_workspace(world);
    for i in 0..count {
        std::fs::write(ws.join(format!("file_{:04}.txt", i)), "").expect("write ls file");
    }
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when("I list the workspace")]
fn when_ls_default(world: &mut QuectoWorld) {
    let tool = make_ls_tool(world);
    world.ls_result = Some(run_ls(tool, serde_json::json!({})));
}

#[when(regex = r#"^I list the workspace with limit (\d+)$"#)]
fn when_ls_with_limit(world: &mut QuectoWorld, limit: usize) {
    let tool = make_ls_tool(world);
    world.ls_result = Some(run_ls(tool, serde_json::json!({"limit": limit})));
}

#[when(regex = r#"^I list the workspace with float limit (\d+\.\d+)$"#)]
fn when_ls_with_float_limit(world: &mut QuectoWorld, limit: f64) {
    let tool = make_ls_tool(world);
    world.ls_result = Some(run_ls(tool, serde_json::json!({"limit": limit})));
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then(regex = r#"^the ls result should contain "([^"]+)"$"#)]
fn then_ls_contains(world: &mut QuectoWorld, expected: String) {
    let r = world.ls_result.as_ref().expect("no ls result");
    assert!(
        r.content.contains(&expected),
        "ls result should contain {:?}, got:\n{}",
        expected,
        r.content
    );
}

#[then(regex = r#"^the ls result should not contain "([^"]+)"$"#)]
fn then_ls_not_contains(world: &mut QuectoWorld, expected: String) {
    let r = world.ls_result.as_ref().expect("no ls result");
    assert!(
        !r.content.contains(&expected),
        "ls result should NOT contain {:?}, got:\n{}",
        expected,
        r.content
    );
}

#[then("the ls result should not be an error")]
fn then_ls_not_error(world: &mut QuectoWorld) {
    let r = world.ls_result.as_ref().expect("no ls result");
    assert!(!r.is_error, "ls should not be error, got: {}", r.content);
}

#[then(regex = r#"^the ls result should have "([^"]+)" before "([^"]+)"$"#)]
fn then_ls_order(world: &mut QuectoWorld, first: String, second: String) {
    let r = world.ls_result.as_ref().expect("no ls result");
    let pos_first = r.content.find(&first);
    let pos_second = r.content.find(&second);
    match (pos_first, pos_second) {
        (Some(a), Some(b)) => assert!(
            a < b,
            "expected {:?} before {:?} in:\n{}",
            first,
            second,
            r.content
        ),
        _ => panic!(
            "could not find {:?} and/or {:?} in:\n{}",
            first, second, r.content
        ),
    }
}
