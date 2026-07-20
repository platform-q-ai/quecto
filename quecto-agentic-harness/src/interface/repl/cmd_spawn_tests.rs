use std::io::Cursor;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::provider::LlmProvider;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::interface::test_support::StubProvider;

use super::super::{ReplLoop, ReplSession};

/// Build a minimal `ReplLoop` backed by in-memory buffers and a stub provider.
fn make_repl() -> (ReplLoop<Cursor<Vec<u8>>, Vec<u8>>, tempfile::TempDir) {
    let tmp = tempfile::TempDir::new().unwrap();
    let provider: Arc<dyn LlmProvider> = Arc::new(StubProvider);
    let registry = ToolRegistryImpl::new();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.7,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_tool_calls: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
        streaming: false,
        effort: None,
        audit_log: None,
        pin_recent_turns: 2,
        context_collapse_after_messages: u32::MAX,
        model_context_window: None,
    });
    let session_store = FileSessionStore::new(tmp.path());
    let session = ReplSession {
        agent,
        messages: Vec::new(),
        session_store,
        session_key: "test:spawn".to_string(),
        ephemeral: true,
        system_prompt: None,
        base_dir: tmp.path().to_path_buf(),
    };
    let reader = Cursor::new(Vec::new());
    let writer = Vec::new();
    let repl = ReplLoop::new(reader, writer, false, session);
    (repl, tmp)
}

fn output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
    String::from_utf8(repl.writer.clone()).unwrap()
}

#[test]
fn test_spawn_usage() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn --help", &rt);
    let out = output(&repl);
    assert!(out.contains("Usage"), "expected Usage in output: {out}");
    assert!(out.contains("--agent"));
    assert!(out.contains("--system"));
    assert!(out.contains("--max-time"));
    assert!(out.contains("--help"));
}

#[test]
fn test_spawn_empty_task() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn", &rt);
    let out = output(&repl);
    assert!(
        out.contains("missing task description"),
        "expected 'missing task description' in output: {out}"
    );
}

#[test]
fn test_spawn_model_rejected() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn --model gpt-5 some task", &rt);
    let out = output(&repl);
    assert!(
        out.contains("not supported in REPL mode"),
        "expected REPL mode rejection in output: {out}"
    );
}

#[test]
fn test_spawn_simple_task() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn Do something", &rt);
    let out = output(&repl);
    assert!(
        out.contains("stub response"),
        "expected agent response in output: {out}"
    );
    // The result should also be injected into parent session messages.
    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].content, "stub response");
}

#[test]
fn test_spawn_with_system() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn --system 'Be concise' summarize this", &rt);
    let out = output(&repl);
    assert!(
        out.contains("stub response"),
        "expected agent response in output: {out}"
    );
}

#[test]
fn test_spawn_with_agent_profile() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, tmp) = make_repl();

    // Create agents directory and a profile file.
    let agents_dir = tmp.path().join("agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    let profile = serde_json::json!({
        "name": "helper",
        "system": "You are a helpful assistant"
    });
    std::fs::write(
        agents_dir.join("helper.json"),
        serde_json::to_string_pretty(&profile).unwrap(),
    )
    .unwrap();

    repl.handle_spawn("/spawn --agent helper do the thing", &rt);
    let out = output(&repl);
    assert!(
        out.contains("stub response"),
        "expected agent response in output: {out}"
    );
    // Result injected into parent session.
    assert_eq!(repl.session.messages.len(), 1);
}

#[test]
fn test_spawn_with_agent_not_found() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn --agent nonexistent do something", &rt);
    let out = output(&repl);
    assert!(
        out.contains("not found"),
        "expected 'not found' in output: {out}"
    );
}

#[test]
fn test_spawn_parse_error() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let (mut repl, _tmp) = make_repl();
    repl.handle_spawn("/spawn --agent", &rt);
    let out = output(&repl);
    assert!(
        out.contains("Error"),
        "expected error message in output: {out}"
    );
    assert!(
        out.contains("--agent requires a value"),
        "expected '--agent requires a value' in output: {out}"
    );
}
