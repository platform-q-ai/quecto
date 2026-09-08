mod admission_broker;
mod agent;
mod auth;
mod commands;
mod models;
pub mod protocol;
pub mod provider_reload;
#[cfg(test)]
mod provider_reload_tests;
pub mod uds;
mod uds_busy_get_message;
mod uds_busy_subagents;
#[cfg(test)]
mod uds_busy_subagents_tests;
mod uds_busy_sync;
#[cfg(test)]
mod uds_busy_sync_tests;
pub mod uds_cancel;
mod uds_cancel_history;
mod uds_control_forward;
mod uds_delete_all_subagents;
mod uds_execution_state;
mod uds_progress_forward;

#[cfg(any(test, feature = "test-support"))]
pub fn live_execution_state_for_events(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}

#[cfg(any(test, feature = "test-support"))]
pub fn completed_live_execution_state(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> serde_json::Value {
    let mut state = uds_execution_state::ExecutionState::default();
    state.start_run();
    for event in events {
        state.observe(event);
    }
    state.finish_run();
    serde_json::json!({ "messageCount": state.message_count(), "execution": state.snapshot() })
}
/// Test-support: run a sequence of progress events through the mid-turn
/// publish path (`publish_turn_progress`) against a fresh conversation
/// snapshot, returning every event line emitted to the sink. BDD scenarios use
/// this to pin that mid-turn `TurnCompleted` events emit `ledger_advanced`
/// hints (the child-progress-freeze fix, 2026-07-29).
#[cfg(any(test, feature = "test-support"))]
pub async fn ledger_hint_lines_for_turn_events(
    events: &[crate::domain::agent::AgentProgressEvent],
) -> Vec<serde_json::Value> {
    let snapshot: uds_multi::ConversationSnapshot = std::sync::Arc::new(tokio::sync::RwLock::new(
        uds_snapshots::ConversationSnapshotData::default(),
    ));
    let mut buf: Vec<u8> = Vec::new();
    let mut sink = uds_cancel::EventSink::writer(&mut buf);
    for event in events {
        uds_cancel::publish_turn_progress(event, Some(&snapshot), &mut sink).await;
    }
    String::from_utf8_lossy(&buf)
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Test-support: run one raw command line through the FULL per-connection
/// reader dispatch (`uds_reader_dispatch::dispatch`) against a snapshot with
/// one committed message. Returns `(served_inline, response)`: `served_inline`
/// is true when the command was answered on the reader task and never queued
/// behind the dispatch loop. Covers the TUI's DIRECT child-feed path — a
/// plain `sync` with no `agent_id` on the child's own socket — which is
/// served by the child-local `uds_busy_sync` fast path even while the child's
/// dispatch loop is occupied (PR #1307 review).
#[cfg(any(test, feature = "test-support"))]
pub async fn busy_reader_dispatch(line: &str) -> (bool, Option<serde_json::Value>) {
    let snapshot: uds_multi::ConversationSnapshot = std::sync::Arc::new(tokio::sync::RwLock::new(
        uds_snapshots::ConversationSnapshotData::from_messages(vec![
            crate::domain::message::Message::user("committed"),
        ]),
    ));
    let clients = uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    uds_ext_protocol::register_client_writer(&clients, 1, tx);
    // The dispatch-loop channel: anything landing here would have queued
    // behind an in-flight parent/child turn.
    let (cmd_tx, mut cmd_rx) = tokio::sync::mpsc::channel(8);
    let (broadcast_tx, _broadcast_rx) = tokio::sync::broadcast::channel::<String>(8);
    uds_reader_dispatch::dispatch(uds_reader_dispatch::ReaderDispatchCtx {
        line: line.to_string(),
        snapshot: &snapshot,
        registry: &clients,
        subagent_registry: &None,
        broadcast_tx: &broadcast_tx,
        client_id: 1,
        cmd_tx: &cmd_tx,
        cancel_handle: &std::sync::Arc::new(std::sync::Mutex::new(uds_cancel::CancelSlot::Idle)),
        turn_control: &uds_cancel::TurnControl::default(),
    })
    .await;
    let served_inline = cmd_rx.try_recv().is_err();
    if !served_inline {
        return (false, None);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}

#[cfg(any(test, feature = "test-support"))]
mod uds_busy_test_support;
#[cfg(any(test, feature = "test-support"))]
pub use uds_busy_test_support::{busy_reader_intercept, busy_reader_intercept_with_registry};

#[cfg(test)]
mod uds_execution_state_tests;
mod uds_ext_protocol;
mod uds_extensions;
mod uds_lifecycle;
pub mod uds_models;
mod uds_multi;
mod uds_multi_accept;
mod uds_query;
mod uds_reader;
mod uds_reader_dispatch;
mod uds_reload;
pub mod uds_session;
mod uds_shutdown;
mod uds_snapshots;
mod uds_socket;
mod uds_state_projection;
#[cfg(test)]
mod uds_state_projection_tests;
#[cfg(test)]
mod uds_thinking_1231_tests;
mod uds_tool_intercept;
pub mod uds_wire;
mod uds_workflow_nudge;

use std::path::PathBuf;

// Re-export public types for external consumers.
pub use agent::build_agent_provider;

/// Re-export for test access to OpenAI import params struct.
pub use auth::auth_import::OpenAiImportParams;

/// Test-friendly OpenAI import with optional OAuth base URL override.
///
/// Wraps the internal `auth_import::import_openai` for BDD testing,
/// allowing injection of a mock OAuth server URL.
pub fn auth_import_openai(
    auth_json: &serde_json::Value,
    params: &OpenAiImportParams<'_>,
    stdout: &mut String,
    stderr: &mut String,
) -> Option<u32> {
    let mut out = auth::Output { stdout, stderr };
    auth::auth_import::import_openai(auth_json, params, &mut out)
}

// Re-export shared functions for backward compatibility.
pub use super::shared::merge_prompts;

/// Result of a CLI invocation, capturing stdout, stderr, and exit code.
#[derive(Debug, Clone)]
pub struct CliOutput {
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
}

/// Runtime context for CLI commands, allowing override of paths for testing.
#[derive(Debug, Clone, Default)]
pub struct CliContext {
    /// Override for the base directory (default: ~/.quecto).
    pub base_dir: Option<PathBuf>,
    /// Override config file path (default: <base_dir>/config.json).
    pub config_path: Option<PathBuf>,
    /// Pre-loaded stdin data for testing interactive commands.
    pub stdin_data: Option<String>,
    /// Whether active stdin is interactive, when known.
    pub stdin_is_tty: Option<bool>,
    /// Whether commands are being delegated by the setup/configuration REPL.
    pub is_repl: bool,
    /// Override OAuth base URL for testing (e.g. wiremock URI).
    pub oauth_base_url: Option<String>,
    /// Override process current working directory for hermetic tests.
    pub cwd: Option<PathBuf>,
}

impl CliContext {
    /// Resolve the config file path: explicit override > base_dir/config.json.
    pub(crate) fn config_path(&self) -> PathBuf {
        self.config_path
            .clone()
            .unwrap_or_else(|| self.base_dir().join("config.json"))
    }

    /// Resolve the base directory: explicit override > QUECTO_BASE_DIR env var > default.
    pub(crate) fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .or_else(|| std::env::var("QUECTO_BASE_DIR").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".quecto")))
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    }
}

/// Extract `--config <path>` from args (consumed globally).
/// Skips values of flags that take arguments (e.g. `-m`, `--system`) to avoid
/// misinterpreting message text like `-m "--config"` as the flag.
fn extract_config_flag(args: &[String]) -> Result<Option<PathBuf>, String> {
    /// Flags that consume the next arg as a value (skip their value during scan).
    const VALUE_FLAGS: &[&str] = &[
        "-m",
        "--message",
        "-s",
        "--session",
        "--system",
        "--model",
        "--max-iterations",
        "--max-time",
        "--mode",
        "--socket",
        "--disable-tool",
    ];
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" {
            let value = args
                .get(i + 1)
                .filter(|value| !value.starts_with('-'))
                .ok_or_else(|| "--config requires a path".to_string())?;
            return Ok(Some(PathBuf::from(value)));
        }
        if VALUE_FLAGS.contains(&args[i].as_str()) {
            i += 2; // skip the flag and its value
        } else {
            i += 1;
        }
    }
    Ok(None)
}

fn strip_global_config_flag(args: &[String]) -> Vec<String> {
    const VALUE_FLAGS: &[&str] = &[
        "-m",
        "--message",
        "-s",
        "--session",
        "--system",
        "--model",
        "--max-iterations",
        "--max-time",
        "--mode",
        "--socket",
        "--disable-tool",
    ];
    let mut stripped = Vec::with_capacity(args.len());
    let mut i = 0;
    while i < args.len() {
        if args[i] == "--config" && i + 1 < args.len() {
            i += 2;
        } else if VALUE_FLAGS.contains(&args[i].as_str()) && i + 1 < args.len() {
            stripped.push(args[i].clone());
            stripped.push(args[i + 1].clone());
            i += 2;
        } else {
            stripped.push(args[i].clone());
            i += 1;
        }
    }
    stripped
}

/// Run the CLI with the given args, printing to real stdout/stderr.
/// Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let config_path = match extract_config_flag(&args) {
        Ok(path) => path,
        Err(error) => {
            eprintln!("{error}");
            return 1;
        }
    };
    let stdin_is_tty = std::io::IsTerminal::is_terminal(&std::io::stdin());
    let ctx = CliContext {
        config_path,
        stdin_is_tty: Some(stdin_is_tty),
        ..Default::default()
    };

    // Enter the live REPL when no command remains after global options.
    // `run_with_output` intentionally uses captured input, so this decision must
    // happen here for config-only production invocations.
    if strip_global_config_flag(&args).len() < 2 {
        if let Some(error) = ctx
            .config_path
            .as_deref()
            .and_then(|path| explicit_config_missing(path, true))
        {
            eprintln!("{error}");
            return 1;
        }
        return super::repl::run_repl(
            std::io::stdin().lock(),
            std::io::stdout(),
            stdin_is_tty,
            |args, reader| run_repl_command(&ctx, args, reader),
        );
    }

    let output = run_with_output(args, &ctx);
    if !output.stdout.is_empty() {
        print!("{}", output.stdout);
    }
    if !output.stderr.is_empty() {
        eprint!("{}", output.stderr);
    }
    output.exit_code
}

/// Run the CLI with the given args and context, capturing all output for testing.
pub fn run_with_output(args: Vec<String>, ctx: &CliContext) -> CliOutput {
    // Merge --config from args into context if not already set.
    let merged_ctx;
    let ctx = if ctx.config_path.is_none() {
        match extract_config_flag(&args) {
            Ok(Some(path)) => {
                merged_ctx = CliContext {
                    config_path: Some(path),
                    ..ctx.clone()
                };
                &merged_ctx
            }
            Ok(None) => ctx,
            Err(error) => {
                return CliOutput {
                    stdout: String::new(),
                    stderr: format!("{error}\n"),
                    exit_code: 1,
                };
            }
        }
    } else {
        ctx
    };
    let args = strip_global_config_flag(&args);
    let mut stdout = String::new();
    let mut stderr = String::new();

    if args.len() < 2 {
        // No args → REPL mode. Delegate to run_repl_with_output with empty input
        // so the REPL exits immediately on EOF (consistent with piped empty input).
        return run_repl_with_output(ctx, &[], &[], false);
    }

    let exit_code = {
        match args[1].as_str() {
            "agent" => agent::cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "status" => commands::cmd_status(ctx, &mut stdout, &mut stderr),
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "models" => models::cmd_models(ctx, &args[2..], &mut stdout, &mut stderr),
            "admission-broker" => {
                admission_broker::cmd_admission_broker(ctx, &args[2..], &mut stdout, &mut stderr)
            }
            "help" | "--help" | "-h" => {
                help_text(&mut stdout);
                0
            }
            "version" | "--version" | "-v" => {
                version_text(&mut stdout);
                0
            }
            other => {
                stderr.push_str(&format!("Unknown command: {other}\n"));
                help_text(&mut stdout);
                1
            }
        }
    };

    CliOutput {
        stdout,
        stderr,
        exit_code,
    }
}

/// Run the setup/configuration REPL with captured output.
pub fn run_repl_with_output(
    ctx: &CliContext,
    args: &[String],
    input: &[u8],
    is_tty: bool,
) -> CliOutput {
    let mut output = Vec::new();
    if !args.is_empty() {
        return CliOutput {
            stdout: String::new(),
            stderr:
                "REPL agent flags are no longer supported; use `quecto agent` for agent operation\n"
                    .into(),
            exit_code: 1,
        };
    }
    let ctx = CliContext {
        stdin_is_tty: Some(is_tty),
        ..ctx.clone()
    };
    let exit_code = super::repl::run_repl(
        std::io::BufReader::new(input),
        &mut output,
        is_tty,
        |args, reader| run_repl_command(&ctx, args, reader),
    );
    CliOutput {
        stdout: String::from_utf8_lossy(&output).to_string(),
        stderr: String::new(),
        exit_code,
    }
}

fn run_repl_command(
    ctx: &CliContext,
    args: Vec<String>,
    reader: &mut dyn std::io::BufRead,
) -> (String, String, i32) {
    let ctx = CliContext {
        is_repl: true,
        ..ctx.clone()
    };
    let ctx = &ctx;
    let mut stdout = String::new();
    let mut stderr = String::new();
    let code = match args.first().map(String::as_str) {
        Some("auth") => {
            auth::cmd_auth_with_reader(ctx, &args[1..], &mut stdout, &mut stderr, reader)
        }
        Some("status") => commands::cmd_status(ctx, &mut stdout, &mut stderr),
        Some("models") => models::cmd_models(ctx, &args[1..], &mut stdout, &mut stderr),
        _ => 1,
    };
    (stdout, stderr, code)
}

pub(crate) fn explicit_config_missing(
    config_path: &std::path::Path,
    explicit: bool,
) -> Option<String> {
    (explicit && !config_path.exists())
        .then(|| format!("config not found: {}", config_path.display()))
}

pub(crate) fn is_valid_session_name(name: &str) -> bool {
    name == "-"
        || (!name.is_empty()
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-'))
}

pub(crate) fn build_tokio_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

fn version_text(out: &mut String) {
    out.push_str(&format!("quecto {}\n", env!("CARGO_PKG_VERSION")));
}

fn help_text(out: &mut String) {
    out.push_str(&format!(
        "quecto - Personal AI Assistant v{}\n",
        env!("CARGO_PKG_VERSION")
    ));
    out.push_str("\nUsage: quecto [command]\n");
    out.push_str("\nWhen run with no arguments, quecto enters the setup and configuration REPL.\n");
    out.push_str("  Use `quecto agent` or `quecto-tui` for agent operation.\n");
    out.push_str("\nGlobal options:\n");
    out.push_str(
        "  --config <path>  Override config file path (default: <base_dir>/config.json)\n",
    );
    out.push_str("\nCommands:\n");
    out.push_str("  admission-broker run|status|reset\n");
    out.push_str("              Shared inference admission authority (requires an `admission` config section)\n");
    out.push_str("  agent       Run a one-shot agent session (-m required)\n");
    out.push_str("              Options: -s <name>  Named session (default: \"default\")\n");
    out.push_str("                       --no-session  Ephemeral mode — nothing saved or loaded\n");
    out.push_str("                       --model <m>   Override model\n");
    out.push_str("                       --system <p>  System prompt\n");
    out.push_str("                       --max-iterations <n>  Max tool iterations\n");
    out.push_str("                       --max-time <s>  Wall-clock timeout in seconds\n");
    out.push_str(
        "                       --mode uds    framed JSON agent mode via Unix domain socket\n",
    );
    out.push_str(
        "                       --socket <path>  Socket path for --mode uds (default: auto in tmpdir)\n",
    );
    out.push_str(
        "                       --effort <level>  Effort level for 4.6 models (low/medium/high/max)\n",
    );
    out.push_str(
        "                       --disable-tool <name>  Disable/hide a tool and deny re-registration (repeatable)\n",
    );
    out.push_str("  auth        Manage authentication (login, logout, status)\n");
    out.push_str("  models      Manage runtime model registry (discover)\n");
    out.push_str("  status      Show status\n");
    out.push_str("  help        Show this help\n");
    out.push_str("  version     Show version information\n");
}

#[cfg(test)]
#[path = "leading_config_dispatch_tests.rs"]
mod leading_config_dispatch_tests;

#[cfg(test)]
mod mod_tests;
