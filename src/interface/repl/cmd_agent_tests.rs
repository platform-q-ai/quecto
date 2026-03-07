use std::future::Future;
use std::io::Cursor;
use std::pin::Pin;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::error::DomainError;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::tool::{ToolDefinition, ToolRegistry, ToolResult};
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::interface::test_support::StubProvider;

use super::super::{ReplLoop, ReplSession};

// -- Stubs --

struct EmptyRegistry;

impl ToolRegistry for EmptyRegistry {
    fn definitions(&self) -> &[ToolDefinition] {
        &[]
    }

    fn execute(
        &self,
        _name: &str,
        _arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        Box::pin(async { Err(DomainError::Tool("no tools".into())) })
    }
}

// -- Helper --

fn make_repl(base_dir: &std::path::Path) -> ReplLoop<Cursor<Vec<u8>>, Vec<u8>> {
    let provider: Arc<dyn LlmProvider> = Arc::new(StubProvider);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(EmptyRegistry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
    });
    let session_store = FileSessionStore::new(base_dir);
    let session = ReplSession {
        agent,
        messages: Vec::new(),
        session_store,
        session_key: "test:agent".to_string(),
        ephemeral: true,
        system_prompt: None,
        base_dir: base_dir.to_path_buf(),
    };
    ReplLoop::new(Cursor::new(Vec::new()), Vec::new(), false, session)
}

fn output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
    String::from_utf8(repl.writer.clone()).unwrap()
}

// -- Tests --

#[test]
fn test_agent_usage() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent", &build_rt());
    let out = output(&repl);
    assert!(out.contains("Usage"), "expected 'Usage' in output: {out}");
    assert!(out.contains("list"));
    assert!(out.contains("create"));
    assert!(out.contains("show"));
    assert!(out.contains("edit"));
    assert!(out.contains("remove"));
    assert!(out.contains("run"));
}

#[test]
fn test_agent_list_no_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent list", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("No subagent profiles"),
        "expected 'No subagent profiles' in: {out}"
    );
}

#[test]
fn test_agent_list_with_profiles() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("alpha.json"), "{}").unwrap();
    std::fs::write(agents_dir.join("beta.json"), "{}").unwrap();
    // Non-json file should be ignored
    std::fs::write(agents_dir.join("readme.txt"), "ignore").unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent list", &build_rt());
    let out = output(&repl);
    assert!(out.contains("Subagent profiles:"), "missing header: {out}");
    assert!(out.contains("alpha"), "missing alpha: {out}");
    assert!(out.contains("beta"), "missing beta: {out}");
    assert!(
        !out.contains("readme"),
        "readme.txt should be filtered: {out}"
    );
}

#[test]
fn test_agent_create() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create mybot --system You are helpful", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("Agent 'mybot' created"),
        "expected created: {out}"
    );

    let path = tmp.path().join("agents").join("mybot.json");
    assert!(path.exists(), "profile file should exist");

    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["name"], "mybot");
    assert_eq!(content["system"], "You are helpful");
}

#[test]
fn test_agent_create_missing_system() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create mybot", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing required flag: --system"),
        "expected missing --system error: {out}"
    );
}

#[test]
fn test_agent_create_duplicate() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create dup --system first", &build_rt());
    // Clear writer to check second attempt output
    repl.writer.clear();
    repl.handle_agent("/agent create dup --system second", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("already exists"),
        "expected 'already exists' error: {out}"
    );
}

#[test]
fn test_agent_create_invalid_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create ../bad --system test", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("invalid agent name"),
        "expected invalid name error: {out}"
    );
}

#[test]
fn test_agent_show() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent(
        "/agent create viewer --system Be brief --model gpt-5",
        &build_rt(),
    );
    repl.writer.clear();
    repl.handle_agent("/agent show viewer", &build_rt());
    let out = output(&repl);
    assert!(out.contains("Agent: viewer"), "expected agent name: {out}");
    assert!(
        out.contains("System: Be brief"),
        "expected system prompt: {out}"
    );
    assert!(out.contains("Model: gpt-5"), "expected model: {out}");
}

#[test]
fn test_agent_show_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent show nonexistent", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("not found"),
        "expected 'not found' error: {out}"
    );
}

#[test]
fn test_agent_show_empty_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent show", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing agent name"),
        "expected 'missing agent name' error: {out}"
    );
}

#[test]
fn test_agent_edit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create editor --system Old prompt", &build_rt());
    repl.writer.clear();
    repl.handle_agent("/agent edit editor --system New prompt", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("Agent 'editor' updated"),
        "expected updated: {out}"
    );

    let path = tmp.path().join("agents").join("editor.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["system"], "New prompt");
}

#[test]
fn test_agent_edit_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent edit ghost --system test", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("not found"),
        "expected 'not found' error: {out}"
    );
}

#[test]
fn test_agent_remove() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create doomed --system temp", &build_rt());
    repl.writer.clear();
    repl.handle_agent("/agent remove doomed", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("Agent 'doomed' removed"),
        "expected removed: {out}"
    );

    let path = tmp.path().join("agents").join("doomed.json");
    assert!(!path.exists(), "file should be deleted");
}

#[test]
fn test_agent_remove_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent remove ghost", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("not found"),
        "expected 'not found' error: {out}"
    );
}

#[test]
fn test_agent_list_empty_dir() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    // Dir exists but has no .json files
    std::fs::write(agents_dir.join("notes.txt"), "not json").unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent list", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("No subagent profiles"),
        "expected 'No subagent profiles' for empty dir: {out}"
    );
}

#[test]
fn test_validated_agent_path_invalid() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Path traversal
    assert!(repl.validated_agent_path("../bad").is_none());
    let out = output(&repl);
    assert!(out.contains("invalid agent name"), "traversal: {out}");

    // Slash in name
    repl.writer.clear();
    assert!(repl.validated_agent_path("foo/bar").is_none());
    let out = output(&repl);
    assert!(out.contains("invalid agent name"), "slash: {out}");

    // Space in name
    repl.writer.clear();
    assert!(repl.validated_agent_path("bad name").is_none());
    let out = output(&repl);
    assert!(out.contains("invalid agent name"), "space: {out}");

    // Empty name
    repl.writer.clear();
    assert!(repl.validated_agent_path("").is_none());
    let out = output(&repl);
    assert!(out.contains("missing agent name"), "empty: {out}");

    // Valid name should return Some
    repl.writer.clear();
    let result = repl.validated_agent_path("good-name");
    assert!(result.is_some());
    assert!(result.unwrap().ends_with("good-name.json"));
}

fn build_rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

// -- agent_run tests --

#[test]
fn test_agent_run_success() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Create a valid profile on disk
    repl.handle_agent("/agent create runner --system You are helpful", &build_rt());
    repl.writer.clear();

    // Run the agent with that profile
    repl.handle_agent("/agent run runner Summarize the news", &build_rt());
    let out = output(&repl);
    assert!(out.contains("stub"), "expected stub response: {out}");

    // Verify the response was injected into the parent session
    assert!(
        !repl.session.messages.is_empty(),
        "agent_run should inject result into session messages"
    );
    let last = repl.session.messages.last().unwrap();
    assert_eq!(last.role, Role::Assistant);
    assert_eq!(last.content, "stub response");
}

#[test]
fn test_agent_run_not_found() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Ensure the agents dir exists but has no matching profile
    std::fs::create_dir_all(tmp.path().join("agents")).unwrap();

    repl.handle_agent("/agent run nonexistent Do something", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("not found"),
        "expected 'not found' error: {out}"
    );
}

#[test]
fn test_agent_run_missing_task() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Create a profile first
    repl.handle_agent("/agent create worker --system Be helpful", &build_rt());
    repl.writer.clear();

    // Run with profile name but no task
    repl.handle_agent("/agent run worker", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing task description"),
        "expected 'missing task description': {out}"
    );
}

#[test]
fn test_agent_run_empty_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent run", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing agent name"),
        "expected 'missing agent name': {out}"
    );
}

#[test]
fn test_agent_run_invalid_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    // Write invalid JSON
    std::fs::write(agents_dir.join("broken.json"), "not valid json {{{").unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent run broken Do something", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("invalid profile"),
        "expected 'invalid profile' error: {out}"
    );
}

#[test]
fn test_agent_create_with_model() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent(
        "/agent create smartbot --system You are smart --model gpt-5-turbo",
        &build_rt(),
    );
    let out = output(&repl);
    assert!(
        out.contains("Agent 'smartbot' created"),
        "expected created: {out}"
    );

    let path = tmp.path().join("agents").join("smartbot.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["name"], "smartbot");
    assert_eq!(content["system"], "You are smart");
    assert_eq!(content["model"], "gpt-5-turbo");
}

#[test]
fn test_agent_show_invalid_profile() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("corrupt.json"), "not json at all").unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent show corrupt", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("invalid profile"),
        "expected 'invalid profile' error for corrupt JSON: {out}"
    );
}

#[test]
fn test_agent_edit_with_model() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Create an agent without a model
    repl.handle_agent("/agent create modeler --system Initial prompt", &build_rt());
    repl.writer.clear();

    // Edit to add a model
    repl.handle_agent("/agent edit modeler --model gpt-5-turbo", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("Agent 'modeler' updated"),
        "expected updated: {out}"
    );

    let path = tmp.path().join("agents").join("modeler.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(
        content["system"], "Initial prompt",
        "system should be preserved"
    );
    assert_eq!(content["model"], "gpt-5-turbo", "model should be added");
}

#[test]
fn test_agent_edit_model_and_system() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    repl.handle_agent(
        "/agent create dual --system Old prompt --model old-model",
        &build_rt(),
    );
    repl.writer.clear();

    // Edit both system and model
    repl.handle_agent(
        "/agent edit dual --system New prompt --model new-model",
        &build_rt(),
    );
    let out = output(&repl);
    assert!(
        out.contains("Agent 'dual' updated"),
        "expected updated: {out}"
    );

    let path = tmp.path().join("agents").join("dual.json");
    let content: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(content["system"], "New prompt");
    assert_eq!(content["model"], "new-model");
}

#[test]
fn test_agent_run_no_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    // Profile with no system field — agent_run should still work
    std::fs::write(agents_dir.join("nosys.json"), r#"{"name": "nosys"}"#).unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent run nosys Do a thing", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("stub"),
        "expected stub response even without system prompt: {out}"
    );
}

#[test]
fn test_agent_run_empty_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    // Profile with empty system string — should not inject a System message
    std::fs::write(
        agents_dir.join("empty-sys.json"),
        r#"{"name": "empty-sys", "system": ""}"#,
    )
    .unwrap();

    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent run empty-sys Do stuff", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("stub"),
        "expected stub response with empty system: {out}"
    );
}

#[test]
fn test_agent_run_injects_into_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());

    // Pre-populate session with a message
    repl.session.messages.push(Message::user("earlier message"));

    repl.handle_agent("/agent create helper --system You help", &build_rt());
    repl.writer.clear();

    repl.handle_agent("/agent run helper What is 2+2", &build_rt());

    // Session should now have 2 messages: the earlier one + the injected assistant response
    assert_eq!(repl.session.messages.len(), 2);
    assert_eq!(repl.session.messages[0].role, Role::User);
    assert_eq!(repl.session.messages[1].role, Role::Assistant);
    assert_eq!(repl.session.messages[1].content, "stub response");
}

#[test]
fn test_agent_run_whitespace_only_task() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent create ws --system test", &build_rt());
    repl.writer.clear();

    // Task is whitespace only — should be treated as empty
    repl.handle_agent("/agent run ws   ", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing task description"),
        "whitespace-only task should trigger missing task: {out}"
    );
}

#[test]
fn test_agent_edit_empty_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent edit", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing agent name"),
        "expected 'missing agent name': {out}"
    );
}

#[test]
fn test_agent_remove_empty_name() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent remove", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("missing agent name"),
        "expected 'missing agent name': {out}"
    );
}

#[test]
fn test_agent_unknown_subcommand() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl(tmp.path());
    repl.handle_agent("/agent foobar", &build_rt());
    let out = output(&repl);
    assert!(
        out.contains("Usage"),
        "unknown subcommand should show usage: {out}"
    );
}
