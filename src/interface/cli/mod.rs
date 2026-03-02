mod agent;
mod auth;
mod commands;

use std::path::PathBuf;

use super::repl::{ReplContext, ReplFlags};
use crate::infrastructure::config::Config;

// Re-export public types for external consumers.
pub use agent::build_agent_provider;

// Re-export shared functions for backward compatibility.
pub use super::shared::{load_skill_prompt, merge_prompts};

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
    /// Pre-loaded stdin data for testing interactive commands.
    pub stdin_data: Option<String>,
    /// Override OAuth base URL for testing (e.g. wiremock URI).
    pub oauth_base_url: Option<String>,
    /// Override GitHub raw content base URL for skills install testing.
    pub github_raw_base_url: Option<String>,
}

impl CliContext {
    /// Resolve the base directory: explicit override > QUECTO_BASE_DIR env var > default.
    pub(crate) fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .or_else(|| std::env::var("QUECTO_BASE_DIR").ok().map(PathBuf::from))
            .or_else(|| dirs::home_dir().map(|h| h.join(".quecto")))
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    }
}

/// Run the CLI with the given args, printing to real stdout/stderr.
/// Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let ctx = CliContext::default();

    // Handle gateway specially — it's a long-running async process
    if args.len() >= 2 && args[1] == "gateway" {
        return commands::cmd_gateway_run(&ctx);
    }

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
    let mut stdout = String::new();
    let mut stderr = String::new();

    if args.len() < 2 {
        // No args → REPL mode. Delegate to run_repl_with_output with empty input
        // so the REPL exits immediately on EOF (consistent with piped empty input).
        return run_repl_with_output(ctx, &[], &[], false);
    }

    let exit_code = {
        match args[1].as_str() {
            "onboard" => commands::cmd_onboard(ctx, &mut stdout, &mut stderr),
            "agent" => agent::cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "gateway" => {
                stdout.push_str("Use 'quecto gateway' to start the gateway service\n");
                0
            }
            "status" => commands::cmd_status(ctx, &mut stdout, &mut stderr),
            "auth" => auth::cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "cron" => {
                stderr.push_str("cron: not yet implemented\n");
                1
            }
            "skills" => commands::cmd_skills(ctx, &args[2..], &mut stdout, &mut stderr),
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
    let renderer = Arc::new(Mutex::new(
        super::repl::progress::ProgressRenderer::new_tty_capture(stderr_buf),
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
    let config_path = base_dir.join("config.json");
    if !config_path.exists() {
        let _ = writeln!(io.writer, "Config not found at {}", config_path.display());
        let _ = writeln!(io.writer, "Run 'quecto onboard' first");
        return 1;
    }

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

    let provider = match build_agent_provider(&config, &base_dir) {
        Ok(p) => p,
        Err(msg) => {
            let _ = writeln!(io.writer, "Error: {msg}");
            return 1;
        }
    };

    let repl_ctx = ReplContext {
        base_dir: &base_dir,
        provider,
        config: &config,
        flags: &flags,
        progress_callback,
    };
    super::repl::run_repl(io.reader, io.writer, io.is_tty, &repl_ctx)
}

/// Parse REPL-specific flags from args (session, system, model).
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
    out.push_str("\nCommands:\n");
    out.push_str("  onboard     Initialize configuration and workspace\n");
    out.push_str("  agent       Run a one-shot agent session (-m required)\n");
    out.push_str("  auth        Manage authentication (login, logout, status)\n");
    out.push_str("  gateway     Start the Telegram gateway\n");
    out.push_str("  status      Show status\n");
    out.push_str("  cron        Manage scheduled tasks\n");
    out.push_str("  skills      Manage skills (install, list, remove)\n");
    out.push_str("  help        Show this help\n");
    out.push_str("  version     Show version information\n");
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
