use super::*;

use crate::domain::error::DomainError;
use crate::domain::message::LlmResponse;
use crate::domain::provider::{ChatRequest, LlmProvider};
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::interface::test_support::make_stub_provider;
use std::io::BufReader;

pub(super) struct BufReplOpts {
    pub(super) is_tty: bool,
    pub(super) ephemeral: bool,
    pub(super) system_prompt: Option<String>,
}

impl Default for BufReplOpts {
    fn default() -> Self {
        Self {
            is_tty: false,
            ephemeral: true,
            system_prompt: None,
        }
    }
}

pub(super) fn make_buf_repl_for_agent_cov(
    base_dir: &std::path::Path,
) -> ReplLoop<BufReader<&'static [u8]>, Vec<u8>> {
    make_buf_repl(base_dir, b"".as_slice(), BufReplOpts::default())
}

pub(super) fn out_for_agent_cov(repl: &ReplLoop<BufReader<&[u8]>, Vec<u8>>) -> String {
    out(repl)
}

pub(super) fn rt_for_agent_cov() -> tokio::runtime::Runtime {
    rt()
}

pub(super) fn make_buf_repl<'a>(
    base_dir: &std::path::Path,
    input: &'a [u8],
    opts: BufReplOpts,
) -> ReplLoop<BufReader<&'a [u8]>, Vec<u8>> {
    make_buf_repl_with_provider(base_dir, input, opts, make_stub_provider())
}

fn make_buf_repl_with_provider<'a>(
    base_dir: &std::path::Path,
    input: &'a [u8],
    opts: BufReplOpts,
    provider: std::sync::Arc<dyn LlmProvider>,
) -> ReplLoop<BufReader<&'a [u8]>, Vec<u8>> {
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(ToolRegistryImpl::new()),
        model: "test-model".to_string(),
        max_tokens: 1024,
        temperature: 0.0,
        spill_store: None,
        session_key: "cov:repl".to_string(),
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
    let session = ReplSession {
        agent,
        messages: Vec::new(),
        session_store: FileSessionStore::new(base_dir),
        session_key: "cov:repl".to_string(),
        ephemeral: opts.ephemeral,
        system_prompt: opts.system_prompt,
        base_dir: base_dir.to_path_buf(),
    };
    ReplLoop::new(BufReader::new(input), Vec::new(), opts.is_tty, session)
}

pub(super) fn out(repl: &ReplLoop<BufReader<&[u8]>, Vec<u8>>) -> String {
    String::from_utf8(repl.writer.clone()).unwrap()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}

#[derive(Debug)]
struct FailingProvider;

impl LlmProvider for FailingProvider {
    fn name(&self) -> &str {
        "failing"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn chat<'a>(
        &'a self,
        _request: ChatRequest<'a>,
    ) -> std::pin::Pin<
        Box<dyn std::future::Future<Output = Result<LlmResponse, DomainError>> + Send + 'a>,
    > {
        Box::pin(async { Err(DomainError::Provider("cov fail".into())) })
    }
}

#[test]
fn failing_provider_trait_methods_are_invoked() {
    rt().block_on(async {
        let provider = FailingProvider;
        assert_eq!(provider.name(), "failing");
        assert!(provider.as_any().downcast_ref::<FailingProvider>().is_some());
        let messages = [];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "stub",
            max_tokens: 100,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let err = provider.chat(request).await.unwrap_err();
        assert!(err.to_string().contains("cov fail"));
        let messages = [];
        let request = ChatRequest {
            messages: &messages,
            tools: &[],
            model: "stub",
            max_tokens: 100,
            temperature: 0.0,
            session_id: None,
            tool_choice: None,
            metadata: None,
            thinking_level: None,
            cancel_flag: None,
            effort: None,
        };
        let mut rx = provider.chat_stream_incremental(request).await;
        assert!(matches!(rx.recv().await, Some(crate::domain::provider::StreamEvent::Error(e)) if e.contains("cov fail")));
    });
}

#[test]
fn bufreader_repl_prints_banner_help_and_runs_slash_commands() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = b"/help\n/clear\n/agent\n/spawn --help\n/exit\n";
    let mut repl = make_buf_repl(
        tmp.path(),
        input.as_slice(),
        BufReplOpts {
            is_tty: true,
            ephemeral: true,
            system_prompt: None,
        },
    );

    assert_eq!(repl.run(), 0);
    let output = out(&repl);
    assert!(output.contains("Interactive Mode"), "{output}");
    assert!(output.contains("Commands:"), "{output}");
    assert!(output.contains("Conversation cleared."), "{output}");
    assert!(output.contains("Usage: /agent <subcommand>"), "{output}");
    assert!(output.contains("Usage: /spawn [flags] <task>"), "{output}");
    assert!(
        output.contains("> "),
        "tty prompt should be printed: {output}"
    );
}

#[test]
fn bufreader_repl_process_input_injects_and_removes_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_buf_repl(
        tmp.path(),
        b"".as_slice(),
        BufReplOpts {
            system_prompt: Some("cov system".to_string()),
            ..Default::default()
        },
    );
    repl.process_input(&rt(), "hello provider");

    let output = out(&repl);
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 2);
    assert_eq!(repl.session.messages[0].role, Role::User);
    assert_eq!(repl.session.messages[0].content, "hello provider");
    assert_eq!(repl.session.messages[1].role, Role::Assistant);
    assert!(
        repl.session
            .messages
            .iter()
            .all(|message| message.role != Role::System),
        "transient system prompt should have been removed: {:?}",
        repl.session.messages
    );
}

#[test]
fn bufreader_repl_remove_system_prompt_fallback_finds_moved_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_buf_repl(
        tmp.path(),
        b"".as_slice(),
        BufReplOpts {
            system_prompt: Some("move me".to_string()),
            ..Default::default()
        },
    );
    let idx = repl.inject_system_prompt();
    repl.session
        .messages
        .insert(0, Message::user("inserted before"));

    repl.remove_system_prompt(idx);

    assert_eq!(repl.session.messages.len(), 1);
    assert_eq!(repl.session.messages[0].content, "inserted before");
}

#[test]
fn bufreader_repl_save_session_on_exit_persists_non_ephemeral_messages() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_buf_repl(
        tmp.path(),
        b"".as_slice(),
        BufReplOpts {
            ephemeral: false,
            ..Default::default()
        },
    );
    repl.session.messages.push(Message::user("persist me"));

    let runtime = rt();
    repl.save_session_on_exit(&runtime);
    let loaded = runtime
        .block_on(repl.session.session_store.load("cov:repl"))
        .unwrap()
        .unwrap();
    assert_eq!(loaded.messages.len(), 1);
    assert_eq!(loaded.messages[0].content, "persist me");
}

#[test]
fn resolve_effort_from_config_accepts_valid_and_ignores_absent() {
    let mut config: Config = serde_json::from_str("{}").unwrap();
    assert!(resolve_effort_from_config(&config).is_none());

    config.agents.defaults.effort = Some("high".to_string());
    assert_eq!(
        resolve_effort_from_config(&config),
        Some(crate::domain::provider::EffortLevel::High)
    );
}

#[test]
fn resolve_effort_from_config_invalid_value_is_ignored_defensively() {
    let mut config: Config = serde_json::from_str("{}").unwrap();
    config.agents.defaults.effort = Some("bogus".to_string());
    assert!(resolve_effort_from_config(&config).is_none());
}

#[test]
fn repl_process_input_provider_error_prints_error_and_removes_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_buf_repl_with_provider(
        tmp.path(),
        b"".as_slice(),
        BufReplOpts {
            system_prompt: Some("transient system".to_string()),
            ..Default::default()
        },
        std::sync::Arc::new(FailingProvider),
    );

    repl.process_input(&rt(), "hello");

    let output = out(&repl);
    assert!(output.contains("Error:"), "{output}");
    assert!(output.contains("cov fail"), "{output}");
    assert!(repl.session.messages.iter().all(|m| m.role != Role::System));
    assert!(
        repl.session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "hello")
    );
}

#[test]
fn remove_system_prompt_no_matching_prompt_leaves_messages_unchanged() {
    let tmp = tempfile::TempDir::new().unwrap();
    let mut repl = make_buf_repl(
        tmp.path(),
        b"".as_slice(),
        BufReplOpts {
            system_prompt: Some("expected".to_string()),
            ..Default::default()
        },
    );
    repl.session.messages.push(Message::user("not system"));
    repl.session.messages.push(Message::system("different"));

    repl.remove_system_prompt(Some(0));

    assert_eq!(repl.session.messages.len(), 2);
    assert_eq!(repl.session.messages[0].content, "not system");
    assert_eq!(repl.session.messages[1].content, "different");
}

#[test]
fn run_dispatches_agent_and_spawn_prefix_commands() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = b"/agent create helper --system Be helpful\n/agent list\n/spawn --system Be terse do it\n/exit\n";
    let mut repl = make_buf_repl(tmp.path(), input.as_slice(), BufReplOpts::default());

    assert_eq!(repl.run(), 0);
    let output = out(&repl);
    assert!(output.contains("Agent 'helper' created"), "{output}");
    assert!(output.contains("Subagent profiles:"), "{output}");
    assert!(output.contains("helper"), "{output}");
    assert!(output.contains("stub response"), "{output}");
    assert_eq!(repl.session.messages.len(), 1);
}

#[test]
fn bufreader_tty_run_prints_banner_help_clear_and_uses_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let input = b"/help\n/clear\nhello\n/exit\n";
    let mut repl = make_buf_repl(
        tmp.path(),
        input.as_slice(),
        BufReplOpts {
            is_tty: true,
            ephemeral: false,
            system_prompt: Some("temporary sys".to_string()),
        },
    );

    assert_eq!(repl.run(), 0);

    let output = out(&repl);
    assert!(output.contains("quecto v"), "{output}");
    assert!(output.contains("Interactive Mode"), "{output}");
    assert!(output.contains("Commands:"), "{output}");
    assert!(output.contains("Conversation cleared."), "{output}");
    assert!(output.contains("stub response"), "{output}");
    assert!(repl.session.messages.iter().all(|m| m.role != Role::System));
    assert!(
        repl.session
            .messages
            .iter()
            .any(|m| m.role == Role::User && m.content == "hello")
    );
}
