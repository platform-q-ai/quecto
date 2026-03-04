use super::*;

#[test]
fn test_slash_command_constants() {
    assert_eq!(CMD_EXIT, "/exit");
    assert_eq!(CMD_QUIT, "/quit");
    assert_eq!(CMD_HELP, "/help");
    assert_eq!(CMD_CLEAR, "/clear");
    assert_eq!(CMD_HEARTBEAT, "/heartbeat");
    assert_eq!(CMD_CRON, "/cron");
    assert_eq!(CMD_AGENT, "/agent");
    assert_eq!(CMD_SPAWN, "/spawn");
}

#[test]
fn test_repl_flags_default() {
    let flags = ReplFlags {
        session_name: None,
        system_prompt: None,
        model_override: None,
        no_sandbox: false,
    };
    assert!(flags.session_name.is_none());
    assert!(flags.system_prompt.is_none());
    assert!(flags.model_override.is_none());
}

#[test]
fn test_repl_flags_with_values() {
    let flags = ReplFlags {
        session_name: Some("mysession".to_string()),
        system_prompt: Some("You are helpful".to_string()),
        model_override: Some("gpt-5-mini".to_string()),
        no_sandbox: false,
    };
    assert_eq!(flags.session_name.as_deref(), Some("mysession"));
    assert_eq!(flags.system_prompt.as_deref(), Some("You are helpful"));
    assert_eq!(flags.model_override.as_deref(), Some("gpt-5-mini"));
}

// -- build_repl_runtime test --

#[test]
fn test_build_repl_runtime_succeeds() {
    let rt = build_repl_runtime();
    assert!(rt.is_ok());
}

// -- load_session_messages_with_rt tests --

#[test]
fn test_load_session_messages_ephemeral_returns_empty() {
    let rt = build_repl_runtime().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let messages = load_session_messages_with_rt(&rt, &store, "any_key", true);
    assert!(messages.is_empty());
}

#[test]
fn test_load_session_messages_no_session_returns_empty() {
    let rt = build_repl_runtime().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let messages = load_session_messages_with_rt(&rt, &store, "nonexistent", false);
    assert!(messages.is_empty());
}

#[test]
fn test_load_session_messages_existing_session() {
    let rt = build_repl_runtime().unwrap();
    let tmp = tempfile::TempDir::new().unwrap();
    let store = FileSessionStore::new(tmp.path());
    // Save a session first
    let session = Session {
        key: "test:key".to_string(),
        messages: vec![Message::user("hello")],
    };
    rt.block_on(store.save(&session)).unwrap();
    let messages = load_session_messages_with_rt(&rt, &store, "test:key", false);
    assert_eq!(messages.len(), 1);
    assert_eq!(messages[0].content, "hello");
}

// -- build_system_prompt tests --

#[test]
fn test_build_system_prompt_no_skills_no_user_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config: Config = serde_json::from_str("{}").unwrap();
    let flags = ReplFlags {
        session_name: None,
        system_prompt: None,
        model_override: None,
        no_sandbox: false,
    };
    let provider = make_stub_provider();
    let ctx = ReplContext {
        base_dir: tmp.path(),
        provider,
        config: &config,
        flags: &flags,
        progress_callback: None,
    };
    let result = build_system_prompt(&ctx);
    // Always Some — at minimum contains the datetime preamble.
    assert!(result.is_some());
    assert!(
        result.as_deref().unwrap().contains("Current date and time"),
        "expected datetime preamble, got: {:?}",
        result
    );
}

#[test]
fn test_build_system_prompt_with_user_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config: Config = serde_json::from_str("{}").unwrap();
    let flags = ReplFlags {
        session_name: None,
        system_prompt: Some("Be helpful".to_string()),
        model_override: None,
        no_sandbox: false,
    };
    let provider = make_stub_provider();
    let ctx = ReplContext {
        base_dir: tmp.path(),
        provider,
        config: &config,
        flags: &flags,
        progress_callback: None,
    };
    let result = build_system_prompt(&ctx);
    let prompt = result.as_deref().unwrap();
    assert!(
        prompt.contains("Current date and time"),
        "expected datetime preamble"
    );
    assert!(
        prompt.contains("Be helpful"),
        "expected user prompt in result"
    );
}

/// Stub provider for unit tests that never makes real HTTP calls.
fn make_stub_provider() -> Arc<dyn LlmProvider> {
    use crate::domain::message::LlmResponse;
    use crate::domain::provider::{ChatRequest, LlmProvider};

    #[derive(Debug)]
    struct StubProvider;

    impl LlmProvider for StubProvider {
        fn name(&self) -> &str {
            "stub"
        }

        fn chat(
            &self,
            _request: ChatRequest<'_>,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = Result<LlmResponse, crate::domain::error::DomainError>,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async {
                Ok(LlmResponse {
                    content: Some("stub response".to_string()),
                    tool_calls: vec![],
                    usage: None,
                    stop_reason: None,
                })
            })
        }
    }

    Arc::new(StubProvider)
}

// ---------------------------------------------------------------
// Helpers for ReplLoop tests
// ---------------------------------------------------------------

use crate::infrastructure::tools::registry::ToolRegistryImpl;
use std::io::Cursor;

/// Options for building a test `ReplLoop`.
struct TestReplOpts {
    is_tty: bool,
    ephemeral: bool,
    system_prompt: Option<String>,
}

impl Default for TestReplOpts {
    fn default() -> Self {
        Self {
            is_tty: false,
            ephemeral: true,
            system_prompt: None,
        }
    }
}

/// Build a `ReplLoop` with pre-loaded input and configurable options.
fn make_repl_loop(
    base_dir: &std::path::Path,
    input: &str,
    opts: TestReplOpts,
) -> ReplLoop<Cursor<Vec<u8>>, Vec<u8>> {
    let provider = make_stub_provider();
    let registry = ToolRegistryImpl::new();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: String::new(),
        context_collapse_after_turns: u32::MAX,
        max_context_tokens: 190_000,
        progress_callback: None,
    });
    let session_store = FileSessionStore::new(base_dir);
    let session = ReplSession {
        agent,
        messages: Vec::new(),
        session_store,
        session_key: "test:repl".to_string(),
        ephemeral: opts.ephemeral,
        system_prompt: opts.system_prompt,
        base_dir: base_dir.to_path_buf(),
    };
    let reader = Cursor::new(input.as_bytes().to_vec());
    let writer = Vec::new();
    ReplLoop::new(reader, writer, opts.is_tty, session)
}

/// Extract output string from a ReplLoop's writer.
fn repl_output(repl: &ReplLoop<Cursor<Vec<u8>>, Vec<u8>>) -> String {
    String::from_utf8(repl.writer.clone()).unwrap()
}

// ---------------------------------------------------------------
// run() tests — use #[test] because run() creates its own runtime
// ---------------------------------------------------------------

#[test]
fn test_run_exit_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
}

#[test]
fn test_run_quit_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "/quit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
}

#[test]
fn test_run_eof_exits() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
}

#[test]
fn test_run_help_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "/help\n/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        out.contains("Commands:"),
        "expected 'Commands:' in output: {out}"
    );
    assert!(out.contains("/help"));
    assert!(out.contains("/clear"));
    assert!(out.contains("/exit"));
    assert!(out.contains("/quit"));
    assert!(out.contains("/cron"));
    assert!(out.contains("/heartbeat"));
    assert!(out.contains("/agent"));
    assert!(out.contains("/spawn"));
}

#[test]
fn test_run_clear_command() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "/clear\n/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        out.contains("Conversation cleared"),
        "expected 'Conversation cleared' in output: {out}"
    );
}

#[test]
fn test_run_banner_shown_for_tty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "/exit\n",
        TestReplOpts {
            is_tty: true,
            ..TestReplOpts::default()
        },
    );
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        out.contains("Interactive Mode"),
        "expected 'Interactive Mode' in output: {out}"
    );
    assert!(out.contains("Type /help"));
}

#[test]
fn test_run_no_banner_for_non_tty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        !out.contains("Interactive Mode"),
        "expected no 'Interactive Mode' banner for non-tty: {out}"
    );
}

#[test]
fn test_run_empty_lines_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "\n\n\n/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
}

#[test]
fn test_run_tty_shows_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "/exit\n",
        TestReplOpts {
            is_tty: true,
            ..TestReplOpts::default()
        },
    );
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        out.contains("> "),
        "expected '> ' prompt in tty output: {out}"
    );
}

// ---------------------------------------------------------------
// inject_system_prompt / remove_system_prompt tests
// ---------------------------------------------------------------

#[test]
fn test_inject_system_prompt_none() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    let idx = repl.inject_system_prompt();
    assert!(idx.is_none());
    assert!(repl.session.messages.is_empty());
}

#[test]
fn test_inject_system_prompt_some() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            system_prompt: Some("Be helpful".to_string()),
            ..TestReplOpts::default()
        },
    );
    let idx = repl.inject_system_prompt();
    assert_eq!(idx, Some(0));
    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].role, Role::System);
    assert_eq!(repl.session.messages[0].content, "Be helpful");
}

#[test]
fn test_inject_system_prompt_appends_at_end() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            system_prompt: Some("system".to_string()),
            ..TestReplOpts::default()
        },
    );
    // Pre-populate some messages
    repl.session.messages.push(Message::user("first"));
    let idx = repl.inject_system_prompt();
    assert_eq!(idx, Some(1));
    assert_eq!(repl.session.messages.len(), 2);
    assert_eq!(repl.session.messages[1].role, Role::System);
}

#[test]
fn test_remove_system_prompt_fast_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            system_prompt: Some("system msg".to_string()),
            ..TestReplOpts::default()
        },
    );
    let idx = repl.inject_system_prompt();
    assert_eq!(repl.session.messages.len(), 1);
    repl.remove_system_prompt(idx);
    assert!(repl.session.messages.is_empty());
}

#[test]
fn test_remove_system_prompt_none_idx() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.session.messages.push(Message::user("keep me"));
    repl.remove_system_prompt(None);
    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].content, "keep me");
}

#[test]
fn test_remove_system_prompt_fallback_scan() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            system_prompt: Some("my prompt".to_string()),
            ..TestReplOpts::default()
        },
    );
    // Inject system prompt at index 0
    let idx = repl.inject_system_prompt();
    assert_eq!(idx, Some(0));
    // Insert a message before the system prompt to shift indices
    repl.session.messages.insert(0, Message::user("inserted"));
    // Now idx=0 points to the user message, not the system prompt.
    // The fallback scan should find and remove the system prompt at index 1.
    repl.remove_system_prompt(idx);
    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].content, "inserted");
}

#[test]
fn test_remove_system_prompt_no_system_prompt_configured() {
    let tmp = tempfile::TempDir::new().unwrap();
    // system_prompt is None, but we pass Some(idx) — should be a no-op
    // because the second guard (system_prompt.is_none()) triggers.
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.session.messages.push(Message::system("something"));
    repl.remove_system_prompt(Some(0));
    // Message should remain — no system_prompt configured so removal is skipped.
    assert_eq!(repl.session.messages.len(), 1);
}

// ---------------------------------------------------------------
// save_session_on_exit tests
// ---------------------------------------------------------------

#[test]
fn test_save_session_ephemeral_skipped() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.session.messages.push(Message::user("hello"));
    let rt = build_repl_runtime().unwrap();
    repl.save_session_on_exit(&rt);
    // Verify nothing was saved (ephemeral=true)
    let loaded = rt.block_on(repl.session.session_store.load("test:repl"));
    assert!(
        loaded.unwrap().is_none(),
        "expected no session saved for ephemeral"
    );
}

#[test]
fn test_save_session_persists_messages() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            ephemeral: false,
            ..TestReplOpts::default()
        },
    );
    repl.session.messages.push(Message::user("persist me"));
    let rt = build_repl_runtime().unwrap();
    repl.save_session_on_exit(&rt);
    let loaded = rt
        .block_on(repl.session.session_store.load("test:repl"))
        .unwrap()
        .expect("expected session to be saved");
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "persist me");
}

// ---------------------------------------------------------------
// handle_clear tests
// ---------------------------------------------------------------

#[test]
fn test_handle_clear_ephemeral_skips_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.session.messages.push(Message::user("hello"));
    let rt = build_repl_runtime().unwrap();
    repl.handle_clear(&rt);
    assert!(repl.session.messages.is_empty());
    let out = repl_output(&repl);
    assert!(out.contains("Conversation cleared"));
    // Verify nothing was persisted
    let loaded = rt.block_on(repl.session.session_store.load("test:repl"));
    assert!(loaded.unwrap().is_none());
}

#[test]
fn test_handle_clear_non_ephemeral_saves_empty() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            ephemeral: false,
            ..TestReplOpts::default()
        },
    );
    repl.session.messages.push(Message::user("hello"));
    let rt = build_repl_runtime().unwrap();
    repl.handle_clear(&rt);
    assert!(repl.session.messages.is_empty());
    let out = repl_output(&repl);
    assert!(out.contains("Conversation cleared"));
    // Verify an empty session was persisted
    let loaded = rt
        .block_on(repl.session.session_store.load("test:repl"))
        .unwrap()
        .expect("expected session to be saved");
    assert!(loaded.messages.is_empty());
}

// ---------------------------------------------------------------
// process_input tests
// ---------------------------------------------------------------

#[test]
fn test_process_input_adds_user_message() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    let rt = build_repl_runtime().unwrap();
    repl.process_input(&rt, "hello world");
    // After process_input, the user message should be in messages
    // (and an assistant message from the stub provider response)
    assert!(
        repl.session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "hello world"),
        "expected user message in history: {:?}",
        repl.session.messages
    );
    let out = repl_output(&repl);
    assert!(
        out.contains("stub response"),
        "expected 'stub response' in output: {out}"
    );
}

#[test]
fn test_process_input_with_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "",
        TestReplOpts {
            system_prompt: Some("Be concise".to_string()),
            ..TestReplOpts::default()
        },
    );
    let rt = build_repl_runtime().unwrap();
    repl.process_input(&rt, "test input");
    // After processing, system prompt should be removed from messages
    assert!(
        !repl
            .session
            .messages
            .iter()
            .any(|m| m.role == Role::System && m.content == "Be concise"),
        "system prompt should be removed after processing: {:?}",
        repl.session.messages
    );
    // User message should still be present
    assert!(
        repl.session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "test input"),
    );
}

// ---------------------------------------------------------------
// print_banner / print_help direct tests
// ---------------------------------------------------------------

#[test]
fn test_print_banner() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.print_banner();
    let out = repl_output(&repl);
    assert!(out.contains("Interactive Mode"), "banner: {out}");
    assert!(out.contains("Type /help"));
    let version = env!("CARGO_PKG_VERSION");
    assert!(
        out.contains(version),
        "expected version {version} in: {out}"
    );
}

#[test]
fn test_print_help() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "", TestReplOpts::default());
    repl.print_help();
    let out = repl_output(&repl);
    assert!(out.contains("Commands:"), "help output: {out}");
    assert!(out.contains("/help"));
    assert!(out.contains("/clear"));
    assert!(out.contains("/exit"));
    assert!(out.contains("/quit"));
    assert!(out.contains("/agent"));
    assert!(out.contains("/cron"));
    assert!(out.contains("/heartbeat"));
    assert!(out.contains("/spawn"));
}

// ---------------------------------------------------------------
// run() with user input (goes through agent)
// ---------------------------------------------------------------

#[test]
fn test_run_processes_user_input() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(tmp.path(), "hello\n/exit\n", TestReplOpts::default());
    let code = repl.run();
    assert_eq!(code, 0);
    let out = repl_output(&repl);
    assert!(
        out.contains("stub response"),
        "expected agent response in output: {out}"
    );
}

#[test]
fn test_run_non_ephemeral_saves_on_exit() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_repl_loop(
        tmp.path(),
        "hello\n/exit\n",
        TestReplOpts {
            ephemeral: false,
            ..TestReplOpts::default()
        },
    );
    let code = repl.run();
    assert_eq!(code, 0);
    // Verify session was saved
    let rt = build_repl_runtime().unwrap();
    let store = FileSessionStore::new(tmp.path());
    let loaded = rt
        .block_on(store.load("test:repl"))
        .unwrap()
        .expect("expected session to be saved on exit");
    assert!(!loaded.messages.is_empty(), "expected messages to be saved");
}
