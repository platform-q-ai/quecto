use super::*;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::message::Message;
use crate::infrastructure::config::Config;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use std::path::PathBuf;

use crate::interface::cli::{CliContext, run_with_output};

fn args(s: &str) -> Vec<String> {
    let mut v = vec!["quecto".to_string()];
    if !s.is_empty() {
        v.extend(s.split_whitespace().map(String::from));
    }
    v
}

/// Helper to load a Config from a JSON string via a temp file.
fn config_from_str(json: &str) -> Config {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(tmp.path(), json).unwrap();
    Config::load(tmp.path().to_str().unwrap()).unwrap()
}

/// Helper: write a minimal config with a fake OpenAI key.
fn write_fake_config(dir: &std::path::Path) {
    std::fs::write(
        dir.join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-fake-test-key-1234"}}}"#,
    )
    .unwrap();
}

/// Helper: create default AgentFlags with common overrides.
fn test_flags(msg: Option<&str>, session: Option<&str>, sys: Option<&str>) -> AgentFlags {
    AgentFlags {
        session_name: session.map(String::from),
        no_session: false,
        message: msg.map(String::from),
        system_prompt: sys.map(String::from),
        model_override: None,
        max_iterations: None,
        max_time: None,
        uds_mode: false,
        no_sandbox: false,
        socket_path: None,
        persist: false,
        disabled_tools: vec![],
        effort: None,
        workflow: false,
        workflow_guards: false,
        workflow_disabled: false,
        workflow_spec_path: None,
        parent_id: None,
    }
}

/// Helper: create a minimal AgentLoopImpl with a fake provider that always fails.
/// Uses 127.0.0.1:1 which gives immediate connection-refused (fast failure).
fn make_test_agent(base_dir: &std::path::Path) -> AgentLoopImpl {
    let config = config_from_str(
        r#"{"providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}}}"#,
    );
    let provider = build_agent_provider(&config, base_dir, &reqwest::Client::new()).unwrap();
    let workspace = PathBuf::from(config.workspace_path());
    let sandbox = Sandbox::new(Some(workspace.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
    AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 100,
        temperature: 0.0,
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
    })
    .with_max_tool_iterations(1)
}

// ===================================================================
// run_agent_session() integration tests (via run_with_output)
//
// These tests write a config.json with a fake API key so that the
// code path through build_agent_from_config -> run_agent_session is
// exercised.  The LLM call will fail (no real server), but the
// session setup, system-prompt injection, and error-handling paths
// are covered.
// ===================================================================

#[test]
fn test_agent_with_valid_config_provider_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent -m test"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(
        out.stderr.contains("Error:"),
        "expected provider error in stderr, got: {}",
        out.stderr
    );
    assert!(
        !out.stderr.contains("config not found"),
        "should not fail on config: {}",
        out.stderr
    );
}

#[test]
fn test_agent_ephemeral_session_no_file_created() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "-s".into(),
            "-".into(),
            "-m".into(),
            "test ephemeral".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    // Ephemeral runs must not persist a session transcript. Creation-time
    // spill entries ARE deliberately written under the sanitized empty-key
    // spill directory ("key_") so collapse/ladder recall() stubs stay
    // resolvable within a `--no-session` run (PR #1048: conversation spilling
    // matches the tool-output spill writer's ephemeral behaviour).
    let sessions_dir = tmp.path().join("sessions");
    if sessions_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(
            entries
                .iter()
                .all(|e| e.file_name() == "key_" && e.path().is_dir()),
            "ephemeral session must not create session transcript files \
             (only the empty-key spill dir is allowed), found: {:?}",
            entries.iter().map(|e| e.path()).collect::<Vec<_>>()
        );
    }
}

#[test]
fn test_agent_with_system_prompt_and_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--system".into(),
            "Be helpful and concise".into(),
            "-m".into(),
            "test with system".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
    assert!(out.stderr.contains("Error:"));
}

#[test]
fn test_agent_with_max_time_reaches_deadline_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--max-time".into(),
            "2".into(),
            "-m".into(),
            "test with deadline".into(),
        ],
        &ctx,
    );
    assert!(
        out.exit_code == 1 || out.exit_code == 2,
        "expected exit 1 or 2, got: {} (stderr: {}, stdout: {})",
        out.exit_code,
        out.stderr,
        out.stdout
    );
    assert!(!out.stderr.contains("config not found"));
}

#[test]
fn test_agent_named_session_creates_no_file_on_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "-s".into(),
            "my-session".into(),
            "-m".into(),
            "test named session".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    let session_file = tmp.path().join("sessions").join("cli_my-session.json");
    assert!(!session_file.exists());
}

#[test]
fn test_agent_with_model_override_and_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--model".into(),
            "gpt-99-turbo".into(),
            "-m".into(),
            "test model override".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
}

#[test]
fn test_agent_with_max_iterations_and_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "--max-iterations".into(),
            "3".into(),
            "-m".into(),
            "test max iter".into(),
        ],
        &ctx,
    );
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
}

#[test]
fn test_agent_with_all_flags_and_config() {
    let tmp = tempfile::TempDir::new().unwrap();
    write_fake_config(tmp.path());
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(
        vec![
            "quecto".into(),
            "agent".into(),
            "-s".into(),
            "full-test".into(),
            "--system".into(),
            "Be concise".into(),
            "--model".into(),
            "gpt-4o-mini".into(),
            "--max-iterations".into(),
            "2".into(),
            "--max-time".into(),
            "3".into(),
            "-m".into(),
            "full flag test".into(),
        ],
        &ctx,
    );
    assert!(
        out.exit_code == 1 || out.exit_code == 2,
        "expected exit 1 or 2, got: {}",
        out.exit_code
    );
    assert!(!out.stderr.contains("config not found"));
}

// ===================================================================
// run_agent_session() direct tests
// ===================================================================

#[test]
fn test_run_agent_session_ephemeral_no_save() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("hello ephemeral"), Some("-"), None);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("Error:"), "stderr: {stderr}");
    let sessions_dir = tmp.path().join("sessions");
    if sessions_dir.exists() {
        let entries: Vec<_> = std::fs::read_dir(&sessions_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .collect();
        assert!(entries.is_empty(), "no session files for ephemeral");
    }
}

#[test]
fn test_run_agent_session_ephemeral_scrubs_spill_file() {
    // Ephemeral spilling deliberately persists during the run (recall stubs
    // must resolve), but nothing may outlive it: run_agent_session must scrub
    // the empty-key spill file on every exit path, including errors
    // (PR #1048 round-2 security review).
    let tmp = tempfile::TempDir::new().unwrap();
    let spill_path = tmp.path().join("sessions").join("key_").join("spill.jsonl");
    std::fs::create_dir_all(spill_path.parent().unwrap()).unwrap();
    std::fs::write(&spill_path, "{\"id\":\"turn0:msg:user\"}\n").unwrap();

    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("secret prompt"), Some("-"), None);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1, "the fake provider must fail");
    assert!(
        !spill_path.exists(),
        "ephemeral spill content must not survive the run (even on error exits)"
    );
}

#[test]
fn test_run_agent_session_named_session_keeps_spill_file() {
    // Scrubbing is ephemeral-only: a named session's spill store must persist
    // for /resume + recall across runs.
    let tmp = tempfile::TempDir::new().unwrap();
    let key_dir = crate::infrastructure::persistence::filename::sanitize_session_key(
        &crate::domain::session::Session::build_key("cli", "keepme"),
    );
    let spill_path = tmp
        .path()
        .join("sessions")
        .join(&key_dir)
        .join("spill.jsonl");
    std::fs::create_dir_all(spill_path.parent().unwrap()).unwrap();
    std::fs::write(&spill_path, "{\"id\":\"turn0:msg:user\"}\n").unwrap();

    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("hello"), Some("keepme"), None);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let _ = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert!(
        spill_path.exists(),
        "named-session spill files must survive the run"
    );
}

#[test]
fn test_run_agent_session_default_session_key() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("hello default"), None, None);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("Error:"), "stderr: {stderr}");
}

#[test]
fn test_run_agent_session_with_system_prompt_injection() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("hello"), Some("-"), Some("You are a test bot"));
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("Error:"), "stderr: {stderr}");
}

#[test]
fn test_run_agent_session_with_deadline() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = make_test_agent(tmp.path());
    let mut flags = test_flags(Some("hello deadline"), Some("-"), None);
    flags.max_time = Some(2);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert!(code == 1 || code == 2);
}

// ===================================================================
// DeadlineResult + run_with_deadline() tests
// ===================================================================

#[test]
fn test_run_with_deadline_completes_before_timeout() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = config_from_str(
        r#"{"providers":{"openai":{"api_key":"sk-test","api_base":"http://127.0.0.1:1"}}}"#,
    );
    let provider = build_agent_provider(&config, tmp.path(), &reqwest::Client::new()).unwrap();
    let workspace = PathBuf::from(config.workspace_path());
    let sandbox = Sandbox::new(Some(workspace.clone()), true);
    let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: "test-model".to_string(),
        max_tokens: 100,
        temperature: 0.0,
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
    })
    .with_max_tool_iterations(1);

    let rt = crate::interface::cli::build_tokio_runtime().unwrap();
    let mut messages = vec![Message::user("test")];
    let result = run_with_deadline(&rt, &agent, &mut messages, 30);
    match result {
        DeadlineResult::Completed(inner) => {
            assert!(inner.is_err(), "expected provider error");
        }
        DeadlineResult::TimedOut => {
            panic!("should not time out with 30s deadline on localhost:1");
        }
    }
}

#[test]
fn test_run_with_deadline_exercises_timeout_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let agent = make_test_agent(tmp.path());
    let rt = crate::interface::cli::build_tokio_runtime().unwrap();
    let mut messages = vec![Message::user("test")];
    let result = run_with_deadline(&rt, &agent, &mut messages, 1);
    match result {
        DeadlineResult::Completed(inner) => {
            assert!(inner.is_err());
        }
        DeadlineResult::TimedOut => {}
    }
}

// ===================================================================
// Agent session with pre-existing session (session load path)
// ===================================================================

#[test]
fn test_run_agent_session_loads_existing_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_json = r#"{
        "key": "cli:existing",
        "messages": [
            {"role": "user", "content": "previous message"},
            {"role": "assistant", "content": "previous response"}
        ]
    }"#;
    std::fs::write(sessions_dir.join("cli_existing.json"), session_json).unwrap();

    let agent = make_test_agent(tmp.path());
    let flags = test_flags(Some("follow-up"), Some("existing"), None);
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("Error:"), "stderr: {stderr}");
}

#[test]
fn test_run_agent_session_loads_existing_with_system_prompt() {
    let tmp = tempfile::TempDir::new().unwrap();
    let sessions_dir = tmp.path().join("sessions");
    std::fs::create_dir_all(&sessions_dir).unwrap();
    let session_json = r#"{
        "key": "cli:sysprompt",
        "messages": [
            {"role": "user", "content": "old message"},
            {"role": "assistant", "content": "old reply"}
        ]
    }"#;
    std::fs::write(sessions_dir.join("cli_sysprompt.json"), session_json).unwrap();

    let agent = make_test_agent(tmp.path());
    let flags = test_flags(
        Some("new message"),
        Some("sysprompt"),
        Some("New system instructions"),
    );
    let mut stdout = String::new();
    let mut stderr = String::new();
    let mut out = AgentOutput {
        stdout: &mut stdout,
        stderr: &mut stderr,
    };
    let code = run_agent_session(tmp.path(), agent, &flags, &mut out);
    assert_eq!(code, 1);
    assert!(stderr.contains("Error:"), "stderr: {stderr}");
}

// build_agent_from_config with various workspace configs

#[test]
fn test_build_agent_from_config_with_workspace_path() {
    let tmp = tempfile::TempDir::new().unwrap();
    let ws = tmp.path().join("my-workspace");
    std::fs::create_dir_all(&ws).unwrap();
    let config_json = format!(
        r#"{{"providers":{{"openai":{{"api_key":"sk-test"}}}},"workspace":"{ws}"}}"#,
        ws = ws.display()
    );
    std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
    let flags = test_flags(Some("hi"), None, None);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None);
    assert!(result.is_some(), "stderr: {}", stderr);
}

#[test]
fn test_build_agent_from_config_with_max_iterations() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
    )
    .unwrap();
    let mut flags = test_flags(Some("hi"), None, None);
    flags.max_iterations = Some(7);
    let mut stderr = String::new();
    let cfg = tmp.path().join("config.json");
    let result = build_agent_from_config(tmp.path(), &cfg, false, &flags, &mut stderr, None);
    assert!(result.is_some(), "stderr: {}", stderr);
}

// ===================================================================
// Agent with anthropic provider config
// ===================================================================

#[test]
fn test_agent_with_anthropic_provider_reaches_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"anthropic":{"api_key":"sk-ant-fake-key"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent -m test-anthropic"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
    assert!(!out.stderr.contains("no LLM providers"));
}

#[test]
fn test_agent_with_both_providers_reaches_session() {
    let tmp = tempfile::TempDir::new().unwrap();
    std::fs::write(
        tmp.path().join("config.json"),
        r#"{"providers":{"openai":{"api_key":"sk-openai-fake"},"anthropic":{"api_key":"sk-ant-fake"}}}"#,
    )
    .unwrap();
    let ctx = CliContext {
        base_dir: Some(tmp.path().to_path_buf()),
        ..Default::default()
    };
    let out = run_with_output(args("agent -m test-both"), &ctx);
    assert_eq!(out.exit_code, 1);
    assert!(!out.stderr.contains("config not found"));
    assert!(!out.stderr.contains("no LLM providers"));
}
