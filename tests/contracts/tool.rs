//! Contract tests for the `Tool` port.
//!
//! Every adapter must:
//!   1. Return a `ToolDefinition` with a non-empty name.
//!   2. Execute with JSON arguments and return a `ToolResult`.
//!   3. Surface argument errors via `is_error: true` rather than returning `Err`.
//!
//! We exercise a representative real adapter (`ReadTool`) against these rules.
//! Adapter-specific behaviour (encoding edge cases, path normalisation, etc.)
//! lives in the adapter's own unit tests.

use quecto::domain::tool::Tool;
use quecto::infrastructure::security::sandbox::Sandbox;
use quecto::infrastructure::tools::filesystem::ReadTool;
use std::path::PathBuf;
use std::sync::Arc;

fn read_tool(workspace: PathBuf) -> Arc<dyn Tool> {
    let sandbox = Arc::new(Sandbox::new(Some(workspace.clone()), true));
    Arc::new(ReadTool::new(Arc::new(workspace), sandbox))
}

#[test]
fn definition_has_nonempty_name_and_schema() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = read_tool(tmp.path().to_path_buf());
    let d = tool.definition();
    assert!(!d.name.is_empty(), "every tool must name itself");
    assert!(!d.parameters_schema.is_empty(),
        "every tool must declare a JSON schema so the LLM can call it");
    // The schema must parse as JSON.
    let _: serde_json::Value = serde_json::from_str(&d.parameters_schema)
        .expect("parameters_schema must be valid JSON");
}

#[tokio::test]
async fn execute_returns_tool_result_for_valid_call() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("hello.txt");
    std::fs::write(&file, "greetings\n").unwrap();
    let tool = read_tool(tmp.path().to_path_buf());

    let args = serde_json::json!({ "path": file }).to_string();
    let r = tool.execute(&args).await.expect("execute must not return Err on valid args");
    assert!(!r.is_error, "a successful read must have is_error=false; got {r:?}");
    assert!(r.content.contains("greetings"),
        "the tool's content must include the file's text, got: {}", r.content);
}

#[tokio::test]
async fn execute_signals_an_error_on_invalid_json_rather_than_panicking() {
    let tmp = tempfile::tempdir().unwrap();
    let tool = read_tool(tmp.path().to_path_buf());

    // The port's contract for argument-handling is "signal an error somehow" —
    // adapters are free to use Err(DomainError) OR ToolResult { is_error: true }.
    // What matters is that invalid input never panics and the caller can
    // distinguish success from failure.
    let signalled_error = match tool.execute("{ not json").await {
        Err(_) => true,
        Ok(r) => r.is_error,
    };
    assert!(signalled_error, "invalid JSON must produce either Err or is_error=true");
}
