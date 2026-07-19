use super::*;

use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::interface::test_support::make_stub_provider;
use std::io::BufReader;

struct BufReplOpts {
    is_tty: bool,
    ephemeral: bool,
    system_prompt: Option<String>,
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

fn make_buf_repl<'a>(
    base_dir: &std::path::Path,
    input: &'a [u8],
    opts: BufReplOpts,
) -> ReplLoop<BufReader<&'a [u8]>, Vec<u8>> {
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider: make_stub_provider(),
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

fn out(repl: &ReplLoop<BufReader<&[u8]>, Vec<u8>>) -> String {
    String::from_utf8(repl.writer.clone()).unwrap()
}

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
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
