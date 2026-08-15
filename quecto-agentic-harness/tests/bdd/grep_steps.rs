use crate::QuectoWorld;
use cucumber::{given, then, when};
use quecto::domain::tool::Tool;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::grep::GrepTool;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::TempDir;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn ensure_grep_workspace(world: &mut QuectoWorld) -> PathBuf {
    if world.grep_workspace.is_none() {
        let tmp = TempDir::new().expect("failed to create grep temp dir");
        let path = tmp.path().to_path_buf();
        world._grep_temp_dir = Some(tmp);
        world.grep_workspace = Some(path);
    }
    world.grep_workspace.clone().unwrap()
}

fn make_grep_tool(world: &mut QuectoWorld) -> GrepTool {
    let ws = ensure_grep_workspace(world);
    let ws_arc = Arc::new(ws.clone());
    // restrict_to_workspace: true — sandbox enforces workspace containment
    let sandbox = Arc::new(Sandbox::new(Some(ws.clone()), true));
    GrepTool::new(ws_arc, sandbox)
}

fn run_tool(tool: GrepTool, args: serde_json::Value) -> quecto::domain::tool::ToolResult {
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

#[given("a grep tool workspace")]
fn given_grep_workspace(world: &mut QuectoWorld) {
    ensure_grep_workspace(world);
}

#[given(regex = r#"^a grep workspace file "([^"]+)" with content:$"#)]
fn given_grep_file_with_docstring(
    world: &mut QuectoWorld,
    step: &cucumber::gherkin::Step,
    filename: String,
) {
    let ws = ensure_grep_workspace(world);
    let content = step
        .docstring
        .as_deref()
        .unwrap_or("")
        .trim_start_matches('\n');
    std::fs::write(ws.join(&filename), content).expect("failed to write grep file");
}

#[given(regex = r#"^a grep workspace file "([^"]+)" with 200 lines containing "([^"]+)"$"#)]
fn given_grep_file_many_lines(world: &mut QuectoWorld, filename: String, word: String) {
    let ws = ensure_grep_workspace(world);
    let content: String = (1..=200)
        .map(|i| format!("line {}: {}\n", i, word))
        .collect();
    std::fs::write(ws.join(&filename), content).expect("failed to write many-line grep file");
}

#[given(
    regex = r#"^a grep workspace file "([^"]+)" with (\d+) lines of (\d+) chars containing "([^"]+)"$"#
)]
fn given_grep_file_long_lines(
    world: &mut QuectoWorld,
    filename: String,
    line_count: usize,
    chars: usize,
    word: String,
) {
    let ws = ensure_grep_workspace(world);
    let padding = "x".repeat(chars.saturating_sub(word.len()));
    let content: String = (1..=line_count)
        .map(|_| format!("{}{}\n", word, padding))
        .collect();
    std::fs::write(ws.join(&filename), content).expect("failed to write long-line grep file");
}

// ---------------------------------------------------------------------------
// When
// ---------------------------------------------------------------------------

#[when(regex = r#"^I grep for pattern "([^"]+)"$"#)]
fn when_grep_pattern(world: &mut QuectoWorld, pattern: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with ignoreCase (true|false)$"#)]
fn when_grep_ignore_case(world: &mut QuectoWorld, pattern: String, flag: String) {
    let tool = make_grep_tool(world);
    let ignore_case = flag == "true";
    let args = serde_json::json!({ "pattern": pattern, "ignoreCase": ignore_case });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with literal (true|false)$"#)]
fn when_grep_literal(world: &mut QuectoWorld, pattern: String, flag: String) {
    let tool = make_grep_tool(world);
    let literal = flag == "true";
    let args = serde_json::json!({ "pattern": pattern, "literal": literal });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with glob "([^"]+)"$"#)]
fn when_grep_glob(world: &mut QuectoWorld, pattern: String, glob: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "glob": glob });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with limit (\d+)$"#)]
fn when_grep_limit(world: &mut QuectoWorld, pattern: String, limit: usize) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "limit": limit });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" with context (\d+)$"#)]
fn when_grep_context(world: &mut QuectoWorld, pattern: String, context: u64) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "context": context });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep with missing rg binary for pattern "([^"]+)"$"#)]
fn when_grep_missing_binary(world: &mut QuectoWorld, pattern: String) {
    let ws = ensure_grep_workspace(world);
    let ws_arc = Arc::new(ws.clone());
    let sandbox = Arc::new(Sandbox::new(Some(ws.clone()), true));
    let tool = GrepTool::with_rg_binary(
        ws_arc,
        sandbox,
        "/nonexistent/path/to/rg_binary_xyz".to_string(),
    );
    let args = serde_json::json!({ "pattern": pattern });
    world.grep_result = Some(run_tool(tool, args));
}

#[when(regex = r#"^I grep for pattern "([^"]+)" in path "([^"]+)"$"#)]
fn when_grep_outside_workspace(world: &mut QuectoWorld, pattern: String, path: String) {
    let tool = make_grep_tool(world);
    let args = serde_json::json!({ "pattern": pattern, "path": path });
    world.grep_result = Some(run_tool(tool, args));
}

// ---------------------------------------------------------------------------
// Then
// ---------------------------------------------------------------------------

#[then(regex = r#"^the grep result should contain "([^"]+)"$"#)]
fn then_grep_result_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        result.content.contains(&expected),
        "grep result should contain {:?}, got:\n{}",
        expected,
        result.content
    );
}

#[then(regex = r#"^the grep result should not contain "([^"]+)"$"#)]
fn then_grep_result_not_contains(world: &mut QuectoWorld, expected: String) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        !result.content.contains(&expected),
        "grep result should NOT contain {:?}, got:\n{}",
        expected,
        result.content
    );
}

#[then("the grep result should not be an error")]
fn then_grep_not_error(world: &mut QuectoWorld) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    if result.content.contains("rg not found on PATH")
        || result.content.starts_with("rg not available")
    {
        return;
    }
    assert!(
        !result.is_error,
        "grep result should not be an error, got:\n{}",
        result.content
    );
}

#[then("the grep result should be an error")]
fn then_grep_is_error(world: &mut QuectoWorld) {
    let result = world
        .grep_result
        .as_ref()
        .expect("no grep result — did you run a When step?");
    assert!(
        result.is_error,
        "grep result should be an error, got:\n{}",
        result.content
    );
}
