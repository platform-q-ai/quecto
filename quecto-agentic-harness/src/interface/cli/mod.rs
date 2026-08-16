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
    uds_reader_dispatch::dispatch(uds_reader_dispatch::ReaderDispatchCtx {
        line: line.to_string(),
        snapshot: &snapshot,
        registry: &clients,
        subagent_registry: &None,
        client_id: 1,
        cmd_tx: &cmd_tx,
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

/// Test-support: run one raw command line through the reader-task busy
/// interceptor for sub-agent liveness commands, with no sub-agent registry.
/// Returns whether it was handled off the dispatch loop and, when handled,
/// the correlated response written to the client's channel.
#[cfg(any(test, feature = "test-support"))]
pub async fn busy_reader_intercept(line: &str) -> (bool, Option<serde_json::Value>) {
    let clients = uds_ext_protocol::new_client_tool_registry();
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(8);
    uds_ext_protocol::register_client_writer(&clients, 1, tx);
    let handled = uds_busy_subagents::intercept(line, &None, &clients, 1).await;
    if !handled {
        return (false, None);
    }
    let response = tokio::time::timeout(std::time::Duration::from_secs(5), rx.recv())
        .await
        .ok()
        .flatten()
        .and_then(|l| serde_json::from_str(&l).ok());
    (true, response)
}

#[cfg(test)]
mod uds_execution_state_tests;
mod uds_ext_protocol;
mod uds_extensions;
mod uds_lifecycle;
mod uds_models;
mod uds_multi;
mod uds_multi_accept;
mod uds_query;
mod uds_reader;
mod uds_reader_dispatch;
mod uds_reload;
pub mod uds_session;
mod uds_snapshots;
mod uds_socket;
mod uds_tool_intercept;
pub mod uds_wire;
mod uds_workflow_nudge;

use std::path::PathBuf;

use super::repl::{ReplContext, ReplFlags};
use crate::infrastructure::config::Config;

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
fn extract_config_flag(args: &[String]) -> Option<PathBuf> {
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
        if args[i] == "--config" && i + 1 < args.len() {
            return Some(PathBuf::from(&args[i + 1]));
        }
        if VALUE_FLAGS.contains(&args[i].as_str()) {
            i += 2; // skip the flag and its value
        } else {
            i += 1;
        }
    }
    None
}

/// Run the CLI with the given args, printing to real stdout/stderr.
/// Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let ctx = CliContext {
        config_path: extract_config_flag(&args),
        ..Default::default()
    };

    // No arguments → enter REPL mode with real stdin/stdout
    if args.len() < 2 {
        let io = ReplIo {
            reader: std::io::stdin().lock(),
            writer: std::io::stdout(),
            is_tty: std::io::IsTerminal::is_terminal(&std::io::stdin()),
        };
        return cmd_repl(&ctx, &[], io);
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
        if let Some(path) = extract_config_flag(&args) {
            merged_ctx = CliContext {
                config_path: Some(path),
                ..ctx.clone()
            };
            &merged_ctx
        } else {
            ctx
        }
    } else {
        ctx
    };
    let mut stdout = String::new();
    let mut stderr = String::new();

    if args.len() < 2 {
        // No args → REPL mode. Delegate to run_repl_with_output with empty input
        // so the REPL exits immediately on EOF (consistent with piped empty input).
        return run_repl_with_output(ctx, &[], &[], false);
    }

    if is_repl_leading_flag(args[1].as_str()) {
        return run_repl_with_output(ctx, &args[1..], &[], false);
    }

    let exit_code = {
        match args[1].as_str() {
            "agent" => agent::cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "status" => commands::cmd_status(ctx, &mut stdout, &mut stderr),
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "models" => models::cmd_models(ctx, &args[2..], &mut stdout, &mut stderr),
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

/// Run the REPL with the given input/output, capturing output for BDD testing.
/// The `is_tty` parameter controls whether the REPL shows the banner and prompt.
///
/// When `is_tty = true`, the REPL will start the spinner thread. In the test
/// harness, the spinner writes to real stderr (process-global). Use
/// [`run_repl_with_tty_captured`] if you need to inspect spinner output.
pub fn run_repl_with_output(
    ctx: &CliContext,
    args: &[String],
    input: &[u8],
    is_tty: bool,
) -> CliOutput {
    let mut output = Vec::new();
    let io = ReplIo {
        reader: std::io::BufReader::new(input),
        writer: &mut output,
        is_tty,
    };
    let exit_code = cmd_repl(ctx, args, io);
    let stdout = String::from_utf8_lossy(&output).to_string();
    CliOutput {
        stdout,
        stderr: String::new(),
        exit_code,
    }
}

/// Run the REPL in TTY mode, capturing both stdout AND spinner stderr output.
///
/// Unlike [`run_repl_with_output`], this variant intercepts the spinner thread's
/// output into a captured stderr buffer so BDD tests can assert on rendered
/// tool names, spinner frames, etc. without touching the real process stderr.
///
/// This function exists solely for the BDD test harness and is gated on the
/// `test-support` feature to prevent it from shipping in release binaries.
#[cfg(any(test, feature = "test-support"))]
pub fn run_repl_with_tty_captured(ctx: &CliContext, args: &[String], input: &[u8]) -> CliOutput {
    use std::sync::{Arc, Mutex};

    let stderr_buf: Arc<Mutex<Vec<u8>>> = Arc::new(Mutex::new(Vec::new()));
    let stderr_clone = stderr_buf.clone();

    // Build a progress callback that renders into the capture buffer (not real stderr).
    //
    // Lock ordering: renderer_mutex → inner buffer_mutex (inside MutexVecWriter).
    // Both locks are held briefly and never acquired in the reverse order anywhere
    // in this path, so there is no deadlock risk. This ordering is only valid for
    // the single-threaded BDD test harness — do not use new_tty_capture in
    // multi-threaded production contexts.
    let status_header = super::repl::progress::build_status_header_line();
    let renderer = Arc::new(Mutex::new(
        super::repl::progress::ProgressRenderer::new_tty_capture_with_status(
            stderr_buf,
            status_header,
        ),
    ));
    let callback: crate::domain::agent::ProgressCallback = Arc::new(move |event| {
        // Recover from mutex poison rather than panicking (double-panic = abort).
        let mut guard = renderer.lock().unwrap_or_else(|e| e.into_inner());
        guard.handle_event(event);
    });

    let mut output = Vec::new();
    let io = ReplIo {
        reader: std::io::BufReader::new(input),
        writer: &mut output,
        is_tty: true,
    };
    let exit_code = cmd_repl_with_progress(ctx, args, io, Some(callback));
    let stdout = String::from_utf8_lossy(&output).to_string();
    let stderr = String::from_utf8_lossy(&stderr_clone.lock().unwrap()).to_string();
    CliOutput {
        stdout,
        stderr,
        exit_code,
    }
}

/// Options for [`run_repl_with_progress_recorder`].
///
/// See [`run_repl_with_progress_recorder`] for usage.
#[cfg(any(test, feature = "test-support"))]
pub struct ReplRecorderOptions<'a> {
    pub ctx: &'a CliContext,
    pub args: &'a [String],
    pub input: &'a [u8],
    pub is_tty: bool,
    pub progress_callback: crate::domain::agent::ProgressCallback,
}

/// Run the REPL with a progress callback recorder for BDD testing.
///
/// The `progress_callback` is wired into the agent loop and receives every
/// [`AgentProgressEvent`] during processing. This allows BDD tests to assert
/// that the right events were fired without needing to inspect TTY output.
///
/// This function exists solely for the BDD test harness and is gated on the
/// `test-support` feature to prevent it from shipping in release binaries.
///
/// [`AgentProgressEvent`]: crate::domain::agent::AgentProgressEvent
#[cfg(any(test, feature = "test-support"))]
pub fn run_repl_with_progress_recorder(opts: ReplRecorderOptions<'_>) -> CliOutput {
    let mut output = Vec::new();
    let io = ReplIo {
        reader: std::io::BufReader::new(opts.input),
        writer: &mut output,
        is_tty: opts.is_tty,
    };
    let exit_code = cmd_repl_with_progress(opts.ctx, opts.args, io, Some(opts.progress_callback));
    let stdout = String::from_utf8_lossy(&output).to_string();
    CliOutput {
        stdout,
        stderr: String::new(),
        exit_code,
    }
}

/// Bundles REPL I/O streams and TTY detection.
struct ReplIo<R: std::io::BufRead, W: std::io::Write> {
    reader: R,
    writer: W,
    is_tty: bool,
}

/// Canonical "config not found" check shared by the agent builder and the REPL
/// launcher. An *explicitly*-provided `--config` path must exist; the default
/// path may be missing (zero-config). Returns the error message to surface, or
/// `None` when there's nothing to report. Callers own their output sink.
pub(crate) fn explicit_config_missing(
    config_path: &std::path::Path,
    explicit: bool,
) -> Option<String> {
    (explicit && !config_path.exists())
        .then(|| format!("config not found: {}", config_path.display()))
}

/// REPL command: parse flags and launch the interactive loop.
fn cmd_repl<R: std::io::BufRead, W: std::io::Write>(
    ctx: &CliContext,
    args: &[String],
    io: ReplIo<R, W>,
) -> i32 {
    cmd_repl_with_progress(ctx, args, io, None)
}

/// REPL command with an optional progress callback for live event reporting.
///
/// The `progress_callback` is forwarded into the agent loop configuration so
/// it receives [`AgentProgressEvent`]s as they fire. Pass `None` for normal
/// interactive operation (the REPL builds its own TTY spinner if needed).
///
/// [`AgentProgressEvent`]: crate::domain::agent::AgentProgressEvent
fn cmd_repl_with_progress<R: std::io::BufRead, W: std::io::Write>(
    ctx: &CliContext,
    args: &[String],
    mut io: ReplIo<R, W>,
    progress_callback: Option<crate::domain::agent::ProgressCallback>,
) -> i32 {
    let flags = match parse_repl_flags(args) {
        Ok(f) => f,
        Err(msg) => {
            let _ = writeln!(io.writer, "Error: {msg}");
            return 1;
        }
    };

    let base_dir = ctx.base_dir();
    let config_path = ctx.config_path();
    if let Some(msg) = explicit_config_missing(&config_path, ctx.config_path.is_some()) {
        let _ = writeln!(io.writer, "Error: {msg}");
        return 1;
    }
    // Zero-config: a missing default config file loads defaults (no onboarding step).

    let env_overrides: std::collections::HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            let _ = writeln!(io.writer, "Error: failed to load config: {e}");
            return 1;
        }
    };

    let http_client = crate::interface::shared::build_http_client();

    let provider = match build_agent_provider(&config, &base_dir, &http_client) {
        Ok(p) => p,
        Err(msg) => {
            let _ = writeln!(io.writer, "Error: {msg}");
            return 1;
        }
    };

    let repl_ctx = ReplContext {
        base_dir: &base_dir,
        config_path: &config_path,
        provider,
        config: &config,
        flags: &flags,
        cwd_override: ctx.cwd.as_deref(),
        progress_callback,
    };
    super::repl::run_repl(io.reader, io.writer, io.is_tty, &repl_ctx)
}

/// Parse REPL-specific flags from args (session, system, model).
fn is_repl_leading_flag(arg: &str) -> bool {
    matches!(
        arg,
        "--no-sandbox" | "-s" | "--session" | "--system" | "--model" | "--config"
    )
}

fn parse_repl_flags(args: &[String]) -> Result<ReplFlags, String> {
    let mut session_name: Option<String> = None;
    let mut system_prompt: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                if i + 1 < args.len() {
                    let name = &args[i + 1];
                    if !is_valid_session_name(name) {
                        return Err(
                            "session name must contain only alphanumeric, '-', or '_'".to_string()
                        );
                    }
                    session_name = Some(name.clone());
                    i += 2;
                } else {
                    return Err("-s requires a session name".to_string());
                }
            }
            "--system" => {
                if i + 1 < args.len() {
                    system_prompt = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--system requires a value".to_string());
                }
            }
            "--model" => {
                if i + 1 < args.len() {
                    model_override = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    return Err("--model requires a value".to_string());
                }
            }
            "--config" => {
                // Consumed globally by CliContext, but skip the value here.
                if i + 1 >= args.len() {
                    return Err("--config requires a path".to_string());
                }
                i += 2;
            }
            "--no-sandbox" => {
                // Deprecated compatibility no-op; sandbox mode has been removed.
                i += 1;
            }
            other if other.starts_with("--") || other.starts_with('-') => {
                return Err(format!("unknown flag '{other}'"));
            }
            _ => {
                i += 1;
            }
        }
    }

    Ok(ReplFlags {
        session_name,
        system_prompt,
        model_override,
    })
}

/// Validate a session name: must be `-` (ephemeral) or only contain `[a-zA-Z0-9_-]`.
/// Rejects path traversal characters like `/`, `..`, and other path-unsafe chars.
pub(crate) fn is_valid_session_name(name: &str) -> bool {
    if name == "-" {
        return true;
    }
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// Build a tokio runtime for CLI agent execution.
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
    out.push_str("\nWhen run with no arguments, quecto enters interactive REPL mode.\n");
    out.push_str("  REPL options: -s <name>, --system <p>, --model <m>\n");
    out.push_str("\nGlobal options:\n");
    out.push_str(
        "  --config <path>  Override config file path (default: <base_dir>/config.json)\n",
    );
    out.push_str("\nCommands:\n");
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
#[path = "mod_tests.rs"]
mod tests;
