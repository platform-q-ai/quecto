use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use super::gateway::{Gateway, resolve_api_key};
use super::repl::{ReplContext, ReplFlags};
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::onboard;
use crate::domain::agent::AgentLoop;
use crate::domain::message::{Message, Role};
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::auth::credential_store::{AuthMethod, Credential, CredentialStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

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
}

impl CliContext {
    /// Resolve the base directory: explicit override > QUECTO_BASE_DIR env var > default.
    fn base_dir(&self) -> PathBuf {
        self.base_dir
            .clone()
            .or_else(|| std::env::var("QUECTO_BASE_DIR").ok().map(PathBuf::from))
            .or_else(onboard::default_base_dir)
            .unwrap_or_else(|| PathBuf::from(".quecto"))
    }
}

/// Run the CLI with the given args, printing to real stdout/stderr.
/// Returns the exit code.
pub fn run(args: Vec<String>) -> i32 {
    let ctx = CliContext::default();

    // Handle gateway specially — it's a long-running async process
    if args.len() >= 2 && args[1] == "gateway" {
        return cmd_gateway_run(&ctx);
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
            "onboard" => cmd_onboard(ctx, &mut stdout, &mut stderr),
            "agent" => cmd_agent(ctx, &args[2..], &mut stdout, &mut stderr),
            "gateway" => {
                stdout.push_str("Use 'quecto gateway' to start the gateway service\n");
                0
            }
            "status" => cmd_status(ctx, &mut stdout, &mut stderr),
            "auth" => cmd_auth(ctx, &args[2..], &mut stdout, &mut stderr),
            "cron" => {
                stderr.push_str("cron: not yet implemented\n");
                1
            }
            "skills" => cmd_skills(ctx, &args[2..], &mut stdout, &mut stderr),
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
    mut io: ReplIo<R, W>,
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

/// Parsed flags for the `agent` subcommand.
struct AgentFlags {
    /// Session name for persistence. `None` = "default", `Some("-")` = ephemeral.
    session_name: Option<String>,
    message: Option<String>,
    system_prompt: Option<String>,
    model_override: Option<String>,
    /// Override max tool iterations (takes precedence over config).
    max_iterations: Option<u32>,
    /// Wall-clock timeout in seconds for the entire agent run.
    max_time: Option<u64>,
}

/// Validate a session name: must be `-` (ephemeral) or only contain `[a-zA-Z0-9_-]`.
/// Rejects path traversal characters like `/`, `..`, and other path-unsafe chars.
fn is_valid_session_name(name: &str) -> bool {
    if name == "-" {
        return true;
    }
    if name.is_empty() {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_agent_flags(args: &[String], stderr: &mut String) -> Option<AgentFlags> {
    let mut session_name: Option<String> = None;
    let mut message: Option<String> = None;
    let mut system_prompt: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut max_iterations: Option<u32> = None;
    let mut max_time: Option<u64> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "-s" | "--session" => {
                if i + 1 < args.len() {
                    let name = &args[i + 1];
                    if !is_valid_session_name(name) {
                        stderr.push_str(
                            "agent: session name must contain only alphanumeric, '-', or '_'\n",
                        );
                        return None;
                    }
                    session_name = Some(name.clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -s requires a session name\n");
                    return None;
                }
            }
            "-m" | "--message" => {
                if i + 1 < args.len() {
                    message = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: -m requires a message\n");
                    return None;
                }
            }
            "--system" => {
                if i + 1 < args.len() {
                    system_prompt = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: --system requires a value\n");
                    return None;
                }
            }
            "--model" => {
                if i + 1 < args.len() {
                    model_override = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("agent: --model requires a value\n");
                    return None;
                }
            }
            "--max-iterations" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u32>() {
                        Ok(0) | Err(_) => {
                            stderr
                                .push_str("agent: --max-iterations requires a positive integer\n");
                            return None;
                        }
                        Ok(n) => max_iterations = Some(n),
                    }
                    i += 2;
                } else {
                    stderr.push_str("agent: --max-iterations requires a value\n");
                    return None;
                }
            }
            "--max-time" => {
                if i + 1 < args.len() {
                    match args[i + 1].parse::<u64>() {
                        Ok(0) | Err(_) => {
                            stderr.push_str("agent: --max-time requires a positive integer\n");
                            return None;
                        }
                        Ok(n) => max_time = Some(n),
                    }
                    i += 2;
                } else {
                    stderr.push_str("agent: --max-time requires a value\n");
                    return None;
                }
            }
            _ => {
                i += 1;
            }
        }
    }

    Some(AgentFlags {
        session_name,
        message,
        system_prompt,
        model_override,
        max_iterations,
        max_time,
    })
}

fn cmd_agent(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let mut flags = match parse_agent_flags(args, stderr) {
        Some(f) => f,
        None => return 1,
    };

    if flags.message.is_none() {
        stderr.push_str("agent: -m is required for non-interactive mode\n");
        return 1;
    }

    let base_dir = ctx.base_dir();
    let agent = match build_agent_from_config(&base_dir, &flags, stderr) {
        Some(a) => a,
        None => return 1,
    };

    // Load skills and prepend their content to the system prompt
    let skill_prompt = super::shared::load_skill_prompt(&base_dir);
    if !skill_prompt.is_empty() {
        flags.system_prompt = Some(super::shared::merge_prompts(
            &skill_prompt,
            &flags.system_prompt,
        ));
    }

    let mut out = AgentOutput { stdout, stderr };
    run_agent_session(&base_dir, agent, &flags, &mut out)
}

// Re-export shared functions for backward compatibility.
pub use super::shared::{load_skill_prompt, merge_prompts};

/// Load config, build provider, and construct the agent loop. Returns None on error.
fn build_agent_from_config(
    base_dir: &std::path::Path,
    flags: &AgentFlags,
    stderr: &mut String,
) -> Option<AgentLoopImpl> {
    let config_path = base_dir.join("config.json");
    if !config_path.exists() {
        stderr.push_str(&format!(
            "config not found at {}\nrun 'quecto onboard' first\n",
            config_path.display()
        ));
        return None;
    }

    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            stderr.push_str(&format!("failed to load config: {}\n", e));
            return None;
        }
    };

    let provider = match build_agent_provider(&config, base_dir) {
        Ok(p) => p,
        Err(msg) => {
            stderr.push_str(&format!("{}\n", msg));
            return None;
        }
    };

    let workspace = PathBuf::from(config.workspace_path());
    let model = flags
        .model_override
        .clone()
        .unwrap_or(config.agents.defaults.model.clone());
    let sandbox = Sandbox::new(
        Some(workspace.clone()),
        config.agents.defaults.restrict_to_workspace,
    );
    let registry = ToolRegistryImpl::with_core_tools(workspace, sandbox);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model,
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
    })
    .with_max_tool_iterations(
        flags
            .max_iterations
            .unwrap_or(config.agents.defaults.max_tool_iterations),
    );

    Some(agent)
}

/// Bundles the stdout/stderr pair passed through the agent pipeline.
struct AgentOutput<'a> {
    stdout: &'a mut String,
    stderr: &'a mut String,
}

/// Run the agent loop with session load/save. Returns the exit code.
fn run_agent_session(
    base_dir: &std::path::Path,
    agent: AgentLoopImpl,
    flags: &AgentFlags,
    out: &mut AgentOutput<'_>,
) -> i32 {
    let ephemeral = flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };

    let session_store = FileSessionStore::new(base_dir);
    let rt = match build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            out.stderr
                .push_str(&format!("failed to create runtime: {}\n", e));
            return 1;
        }
    };

    let mut messages: Vec<Message> = if !ephemeral {
        match rt.block_on(session_store.load(&session_key)) {
            Ok(Some(session)) => session.messages,
            Ok(None) => Vec::new(),
            Err(e) => {
                out.stderr
                    .push_str(&format!("failed to load session: {}\n", e));
                return 1;
            }
        }
    } else {
        Vec::new()
    };

    // System prompt is injected at call time but not persisted in session history.
    // Track its index so we can remove exactly this message before saving.
    let system_prompt_idx = if flags.system_prompt.is_some() {
        let idx = messages.len();
        messages.push(Message {
            role: Role::System,
            content: flags.system_prompt.as_deref().unwrap_or("").to_string(),
            tool_calls: vec![],
            tool_call_id: None,
        });
        Some(idx)
    } else {
        None
    };

    let message = flags.message.as_deref().unwrap_or("");
    messages.push(Message {
        role: Role::User,
        content: message.to_string(),
        tool_calls: vec![],
        tool_call_id: None,
    });

    let agent_result = if let Some(secs) = flags.max_time {
        match run_with_deadline(&rt, &agent, &mut messages, secs) {
            DeadlineResult::Completed(inner) => inner,
            DeadlineResult::TimedOut => {
                out.stderr.push_str("max-time exceeded\n");
                return 2;
            }
        }
    } else {
        rt.block_on(agent.process(&mut messages))
    };

    match agent_result {
        Ok(result) => {
            if !ephemeral {
                if let Some(idx) = system_prompt_idx {
                    if idx < messages.len() {
                        messages.remove(idx);
                    }
                }
                let session = Session {
                    key: session_key,
                    messages: std::mem::take(&mut messages),
                };
                if let Err(e) = rt.block_on(session_store.save(&session)) {
                    out.stderr
                        .push_str(&format!("warning: failed to save session: {}\n", e));
                }
            }
            out.stdout.push_str(&result.response);
            out.stdout.push('\n');
            0
        }
        Err(e) => {
            out.stderr.push_str(&format!("Error: {}\n", e));
            1
        }
    }
}

/// Build a tokio runtime for CLI agent execution.
fn build_tokio_runtime() -> Result<tokio::runtime::Runtime, std::io::Error> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
}

/// Outcome of a deadline-bounded agent run.
enum DeadlineResult {
    /// Agent completed (successfully or with error) within the deadline.
    Completed(Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>),
    /// The deadline expired before the agent finished.
    TimedOut,
}

/// Run the agent with a wall-clock deadline.
///
/// Uses `thread::scope` + `recv_timeout` so the deadline is enforced without
/// requiring `tokio::time::timeout` (which needs an active reactor context
/// that may conflict with test harness runtimes). After timeout, the scoped
/// thread still runs until the in-flight LLM/tool call completes (bounded by
/// per-tool and HTTP client timeouts), then the scope exits.
fn run_with_deadline(
    rt: &tokio::runtime::Runtime,
    agent: &AgentLoopImpl,
    messages: &mut Vec<Message>,
    timeout_secs: u64,
) -> DeadlineResult {
    let dur = std::time::Duration::from_secs(timeout_secs);
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let deadline = std::time::Instant::now() + dur;

    std::thread::scope(|s| {
        s.spawn(|| {
            let result = rt.block_on(agent.process(messages));
            let _ = tx.send(result);
        });

        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        match rx.recv_timeout(remaining) {
            Ok(result) => DeadlineResult::Completed(result),
            Err(_) => DeadlineResult::TimedOut,
        }
    })
}

/// Build a FallbackProvider from config + credential store, suitable for the agent CLI.
fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
) -> Result<Arc<dyn LlmProvider>, String> {
    let store = CredentialStore::new(base_dir);
    let creds = store.load_snapshot().unwrap_or_default();

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();

    // Try OpenAI
    let openai_key = resolve_api_key(&config.providers.openai.api_key, &creds, "openai");
    if !openai_key.is_empty() {
        let base = if config.providers.openai.api_base.is_empty() {
            None
        } else {
            Some(config.providers.openai.api_base.clone())
        };
        if let Some(p) = providers::create_provider("openai", openai_key, base) {
            provider_list.push(p);
        }
    }

    // Try Anthropic
    let anthropic_key = resolve_api_key(&config.providers.anthropic.api_key, &creds, "anthropic");
    if !anthropic_key.is_empty() {
        let base = if config.providers.anthropic.api_base.is_empty() {
            None
        } else {
            Some(config.providers.anthropic.api_base.clone())
        };
        if let Some(p) = providers::create_provider("anthropic", anthropic_key, base) {
            provider_list.push(p);
        }
    }

    if provider_list.is_empty() {
        return Err(
            "no LLM providers configured (set an API key or run 'quecto auth login')".to_string(),
        );
    }

    Ok(Arc::new(FallbackProvider::new(provider_list)))
}

fn cmd_auth(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();

    if args.is_empty() {
        stderr.push_str("auth: missing subcommand (login, logout, status)\n");
        return 1;
    }

    match args[0].as_str() {
        "login" => cmd_auth_login(ctx, &args[1..], stdout, stderr),
        "logout" => cmd_auth_logout(&base, &args[1..], stdout, stderr),
        "status" => cmd_auth_status(&base, stdout),
        other => {
            stderr.push_str(&format!("auth: unknown subcommand '{}'\n", other));
            1
        }
    }
}

/// Known provider names accepted by the auth commands.
const KNOWN_PROVIDERS: &[&str] = &["openai", "anthropic"];

fn cmd_auth_login(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let base = ctx.base_dir();
    let mut provider: Option<String> = None;
    let mut token: Option<String> = None;
    let mut use_oauth = false;
    let mut use_device_code = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --provider requires a value\n");
                    return 1;
                }
            }
            "--token" => {
                if i + 1 < args.len() {
                    token = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth login: --token requires a value\n");
                    return 1;
                }
            }
            "--oauth" => {
                use_oauth = true;
                i += 1;
            }
            "--device-code" => {
                use_device_code = true;
                i += 1;
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth login: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth login: --provider is required\n");
        return 1;
    };

    if !KNOWN_PROVIDERS.contains(&provider.as_str()) {
        stderr.push_str(&format!(
            "auth login: unknown provider '{}'. Known: {}\n",
            provider,
            KNOWN_PROVIDERS.join(", ")
        ));
        return 1;
    }

    if use_oauth {
        return cmd_auth_login_oauth(ctx, &provider, stdout, stderr);
    }

    if use_device_code {
        return cmd_auth_login_device_code(ctx, &provider, stdout, stderr);
    }

    // If --token was provided, use it directly.
    // Otherwise, prompt for interactive token paste.
    let token = match token {
        Some(t) => t,
        None => {
            stdout.push_str(&format!("Paste your API token for {}:\n", provider));
            match read_stdin_line(ctx) {
                Ok(line) => line,
                Err(e) => {
                    stderr.push_str(&format!("auth login: {}\n", e));
                    return 1;
                }
            }
        }
    };

    let token = token.trim().to_string();
    if token.is_empty() {
        stderr.push_str("auth login: --token value must not be empty\n");
        return 1;
    }

    let store = CredentialStore::new(&base);
    match store.store(Credential {
        provider: provider.clone(),
        token,
        method: AuthMethod::Token,
        expires_at: None,
    }) {
        Ok(()) => {
            stdout.push_str(&format!("Credential stored for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to store credential: {}\n", e));
            1
        }
    }
}

/// Read a single line from stdin (or from `ctx.stdin_data` in test mode).
/// Returns `Err` with an error message if stdin cannot be read.
fn read_stdin_line(ctx: &CliContext) -> Result<String, String> {
    if let Some(ref data) = ctx.stdin_data {
        // Return the first line of pre-loaded stdin data.
        Ok(data.lines().next().unwrap_or("").to_string())
    } else {
        let mut line = String::new();
        std::io::stdin()
            .read_line(&mut line)
            .map_err(|e| format!("failed to read from stdin: {}", e))?;
        Ok(line)
    }
}

/// Resolve OAuth config: use test override if set, otherwise look up the provider.
fn resolve_oauth_config(
    ctx: &CliContext,
    provider: &str,
    flow_name: &str,
    stderr: &mut String,
) -> Option<crate::infrastructure::auth::oauth::OAuthConfig> {
    use crate::infrastructure::auth::oauth::OAuthConfig;

    if let Some(ref base_url) = ctx.oauth_base_url {
        Some(OAuthConfig::with_base_url(base_url))
    } else {
        match OAuthConfig::for_provider(provider) {
            Some(c) => Some(c),
            None => {
                stderr.push_str(&format!(
                    "auth login: {} is not supported for '{}'\n",
                    flow_name, provider
                ));
                None
            }
        }
    }
}

/// OAuth browser-based login flow.
fn cmd_auth_login_oauth(
    ctx: &CliContext,
    provider: &str,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "OAuth", stderr) {
        Some(c) => c,
        None => return 1,
    };

    stdout.push_str(&format!(
        "Open this URL in your browser:\n{}\n\nWaiting for authorization...\n",
        config.authorization_url
    ));
    0
}

/// Device code login flow for headless environments.
fn cmd_auth_login_device_code(
    ctx: &CliContext,
    provider: &str,
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let config = match resolve_oauth_config(ctx, provider, "device code flow", stderr) {
        Some(c) => c,
        None => return 1,
    };

    let rt = match build_tokio_runtime() {
        Ok(rt) => rt,
        Err(e) => {
            stderr.push_str(&format!("auth login: failed to create runtime: {}\n", e));
            return 1;
        }
    };
    match rt.block_on(crate::infrastructure::auth::oauth::request_device_code(
        &config,
    )) {
        Ok(resp) => {
            stdout.push_str(&format!(
                "Go to: {}\nEnter code: {}\n\nWaiting for authorization...\n",
                resp.verification_uri, resp.user_code
            ));
            0
        }
        Err(e) => {
            stderr.push_str(&format!("auth login: device code request failed: {}\n", e));
            1
        }
    }
}

fn cmd_auth_logout(
    base: &std::path::Path,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut provider: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--provider" => {
                if i + 1 < args.len() {
                    provider = Some(args[i + 1].clone());
                    i += 2;
                } else {
                    stderr.push_str("auth logout: --provider requires a value\n");
                    return 1;
                }
            }
            other if other.starts_with("--") => {
                stderr.push_str(&format!("auth logout: unknown flag '{}'\n", other));
                return 1;
            }
            _ => {
                i += 1;
            }
        }
    }

    let Some(provider) = provider else {
        stderr.push_str("auth logout: --provider is required\n");
        return 1;
    };

    let store = CredentialStore::new(base);
    match store.remove(&provider) {
        Ok(true) => {
            stdout.push_str(&format!("Credential removed for {}\n", provider));
            0
        }
        Ok(false) => {
            stdout.push_str(&format!("no credential found for {}\n", provider));
            0
        }
        Err(e) => {
            stderr.push_str(&format!(
                "auth logout: failed to remove credential: {}\n",
                e
            ));
            1
        }
    }
}

fn cmd_auth_status(base: &std::path::Path, stdout: &mut String) -> i32 {
    let store = CredentialStore::new(base);
    match store.status_summary() {
        Ok(statuses) => {
            if statuses.is_empty() {
                stdout.push_str("no credentials stored\n");
            } else {
                stdout.push_str("Credentials:\n");
                for s in &statuses {
                    stdout.push_str(&format!("  {} ({}) — {}\n", s.provider, s.method, s.status));
                }
            }
            0
        }
        Err(e) => {
            stdout.push_str(&format!("failed to read credentials: {}\n", e));
            1
        }
    }
}

/// List installed skills with their descriptions from SKILL.md frontmatter.
fn cmd_skills_list(base: &std::path::Path, stdout: &mut String) -> i32 {
    use crate::domain::skill::SkillLoader;
    use crate::infrastructure::persistence::skill_loader::FileSkillLoader;

    let workspace = base.join("workspace");
    let loader = FileSkillLoader::new(&workspace);
    let skills = match loader.list() {
        Ok(s) => s,
        Err(_) => {
            stdout.push_str("No skills installed\n");
            return 0;
        }
    };
    if skills.is_empty() {
        stdout.push_str("No skills installed\n");
    } else {
        for skill in &skills {
            stdout.push_str(&format!("  {} — {}\n", skill.name, skill.description));
        }
    }
    0
}

fn cmd_skills(ctx: &CliContext, args: &[String], stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();
    let ws_skills = base.join("workspace").join("skills");

    if args.is_empty() {
        stderr.push_str("skills: missing subcommand (list, remove, install)\n");
        return 1;
    }

    match args[0].as_str() {
        "list" => cmd_skills_list(&base, stdout),
        "remove" => {
            if args.len() < 2 {
                stderr.push_str("skills remove: missing skill name\n");
                return 1;
            }
            let name = &args[1];
            let skill_dir = ws_skills.join(name);
            if !skill_dir.is_dir() {
                stderr.push_str(&format!("skill '{}' not found\n", name));
                return 1;
            }
            match std::fs::remove_dir_all(&skill_dir) {
                Ok(_) => {
                    stdout.push_str(&format!("'{}' removed successfully\n", name));
                    0
                }
                Err(e) => {
                    stderr.push_str(&format!("failed to remove skill '{}': {}\n", name, e));
                    1
                }
            }
        }
        "install" => {
            stderr.push_str("skills install: not yet implemented\n");
            1
        }
        other => {
            stderr.push_str(&format!("skills: unknown subcommand '{}'\n", other));
            1
        }
    }
}

fn cmd_status(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let base = ctx.base_dir();
    let config_path = base.join("config.json");

    stdout.push_str("quecto Status\n");
    stdout.push_str(&format!("  Config:    {}\n", config_path.display()));

    let config = if config_path.exists() {
        match Config::load(config_path.to_str().unwrap_or("")) {
            Ok(c) => c,
            Err(e) => {
                stderr.push_str(&format!("failed to load config: {}\n", e));
                return 1;
            }
        }
    } else {
        stderr.push_str("config not found; run 'quecto onboard' first\n");
        return 1;
    };

    let ws = config.workspace_path();
    stdout.push_str(&format!("  Workspace: {}\n", ws));
    stdout.push_str(&format!("  Model:     {}\n", config.agents.defaults.model));

    // Provider availability
    let openai_status = if config.providers.openai.api_key.is_empty() {
        "not set".to_string()
    } else {
        "configured".to_string()
    };
    let anthropic_status = if config.providers.anthropic.api_key.is_empty() {
        "not set".to_string()
    } else {
        "configured".to_string()
    };
    stdout.push_str(&format!("  OpenAI API:    {}\n", openai_status));
    stdout.push_str(&format!("  Anthropic API: {}\n", anthropic_status));

    // Telegram status
    let telegram_status = if config.channels.telegram.enabled {
        "enabled"
    } else {
        "disabled"
    };
    stdout.push_str(&format!("  Telegram:      {}\n", telegram_status));

    // Heartbeat status
    let heartbeat_status = if config.heartbeat.enabled {
        format!("enabled ({}s)", config.heartbeat.interval)
    } else {
        "disabled".to_string()
    };
    stdout.push_str(&format!("  Heartbeat:     {}\n", heartbeat_status));

    0
}

/// Run the gateway as a long-running async service.
/// This creates a tokio runtime and blocks until shutdown.
fn cmd_gateway_run(ctx: &CliContext) -> i32 {
    let base_dir = ctx.base_dir();
    let config_path = base_dir.join("config.json");

    if !config_path.exists() {
        eprintln!("config not found at {}", config_path.display());
        eprintln!("run 'quecto onboard' first");
        return 1;
    }

    // Load config with env overrides
    let env_overrides: HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();

    let config = match Config::load_with_env(config_path.to_str().unwrap_or(""), &env_overrides) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("failed to load config: {}", e);
            return 1;
        }
    };

    let gateway = Gateway::new(config, base_dir);

    let rt = tokio::runtime::Runtime::new().unwrap();
    match rt.block_on(gateway.run()) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("gateway error: {}", e);
            1
        }
    }
}

fn cmd_onboard(ctx: &CliContext, stdout: &mut String, stderr: &mut String) -> i32 {
    let base_dir = ctx.base_dir();
    match onboard::run_onboard(&base_dir) {
        Ok(result) => {
            if result.already_existed {
                stdout.push_str("Config already exists\n");
                stdout.push_str(&format!("  path: {}\n", result.config_path.display()));
            } else {
                stdout.push_str("quecto is ready!\n");
                stdout.push_str(&format!("  config:    {}\n", result.config_path.display()));
                stdout.push_str(&format!(
                    "  workspace: {}\n",
                    result.workspace_path.display()
                ));
            }
            0
        }
        Err(e) => {
            stderr.push_str(&format!("onboard failed: {e}\n"));
            1
        }
    }
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
mod tests {
    use super::*;

    fn args(s: &str) -> Vec<String> {
        let mut v = vec!["quecto".to_string()];
        if !s.is_empty() {
            v.extend(s.split_whitespace().map(String::from));
        }
        v
    }

    fn default_ctx() -> CliContext {
        CliContext::default()
    }

    fn assert_contains_all(haystack: &str, needles: &[&str]) {
        for needle in needles {
            assert!(
                haystack.contains(needle),
                "expected output to contain '{needle}', got:\n{haystack}"
            );
        }
    }

    #[test]
    fn test_no_args_triggers_repl_mode() {
        // run_with_output with no args delegates to run_repl_with_output,
        // which enters REPL mode with empty input (exits immediately on EOF).
        // Without a config file, the REPL outputs an error.
        let out = run_with_output(vec!["quecto".to_string()], &default_ctx());
        // Either exits 0 (with config) or 1 (without config, showing config error).
        // In default context without config, exit code is 1.
        assert!(out.exit_code == 0 || out.exit_code == 1);
    }

    #[test]
    fn test_help_command_shows_usage() {
        let out = run_with_output(args("help"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert_contains_all(
            &out.stdout,
            &[
                "Usage: quecto [command]",
                "onboard",
                "agent",
                "gateway",
                "status",
                "auth",
                "cron",
                "skills",
                "help",
                "version",
            ],
        );
    }

    #[test]
    fn test_version_command() {
        let out = run_with_output(args("version"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
        assert!(out.stdout.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn test_version_flag() {
        let out = run_with_output(args("--version"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
    }

    #[test]
    fn test_version_short_flag() {
        let out = run_with_output(args("-v"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto"));
    }

    #[test]
    fn test_unknown_command() {
        let out = run_with_output(args("foobar"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("Unknown command: foobar"));
        assert!(out.stdout.contains("Usage: quecto [command]"));
    }

    #[test]
    fn test_onboard_creates_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("onboard"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("quecto is ready"));
        assert!(tmp.path().join("config.json").exists());
        assert!(tmp.path().join("workspace").exists());
    }

    #[test]
    fn test_onboard_existing_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("onboard"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Config already exists"));
    }

    #[test]
    fn test_status_shows_summary() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "agents": { "defaults": { "model": "gpt-5.2" } },
            "providers": {
                "openai": { "api_key": "sk-test" },
                "anthropic": { "api_key": "" }
            }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert_contains_all(
            &out.stdout,
            &[
                "quecto Status",
                "Config:",
                "Workspace:",
                "Model:",
                "gpt-5.2",
                "OpenAI API:",
                "configured",
                "Anthropic API:",
                "not set",
            ],
        );
    }

    #[test]
    fn test_status_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("config not found"));
    }

    #[test]
    fn test_status_redacts_api_keys() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": {
                "openai": { "api_key": "sk-super-secret-12345" }
            }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(!out.stdout.contains("sk-super-secret-12345"));
        assert!(out.stdout.contains("configured"));
    }

    #[test]
    fn test_auth_missing_subcommand() {
        let out = run_with_output(args("auth"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_cron_not_implemented() {
        let out = run_with_output(args("cron"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not yet implemented"));
    }

    #[test]
    fn test_gateway_subcommand_hint() {
        let out = run_with_output(args("gateway"), &default_ctx());
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("gateway"));
    }

    #[test]
    fn test_agent_no_message_requires_m_flag() {
        // Without -m, agent should require non-interactive mode
        let out = run_with_output(args("agent"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("-m is required"));
    }

    #[test]
    fn test_agent_session_flag_missing_value() {
        let out = run_with_output(args("agent -s"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("-s requires a session name"));
    }

    #[test]
    fn test_agent_message_flag_missing_value() {
        let out = run_with_output(args("agent -m"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("-m requires a message"));
    }

    #[test]
    fn test_skills_no_subcommand() {
        let out = run_with_output(args("skills"), &default_ctx());
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_skills_unknown_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills foobar"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown subcommand"));
    }

    #[test]
    fn test_skills_list_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills list"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("No skills installed"));
    }

    #[test]
    fn test_skills_list_with_skills() {
        let tmp = tempfile::TempDir::new().unwrap();
        let skill_dir = tmp.path().join("workspace").join("skills").join("weather");
        std::fs::create_dir_all(&skill_dir).unwrap();
        std::fs::write(
            skill_dir.join("SKILL.md"),
            "---\nname: weather\ndescription: Weather forecasts\n---\nBody",
        )
        .unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills list"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("weather"));
        assert!(out.stdout.contains("Weather forecasts"));
    }

    #[test]
    fn test_skills_remove_missing_name() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills remove"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing skill name"));
    }

    #[test]
    fn test_skills_remove_not_found() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills remove nonexistent"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not found"));
    }

    #[test]
    fn test_skills_install_not_implemented() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("skills install"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("not yet implemented"));
    }

    #[test]
    fn test_status_shows_telegram_and_heartbeat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": { "openai": { "api_key": "sk-test" } },
            "channels": { "telegram": { "enabled": true, "token": "123:ABC" } },
            "heartbeat": { "enabled": true, "interval": 300 }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("Telegram:"));
        assert!(out.stdout.contains("enabled"));
        assert!(out.stdout.contains("Heartbeat:"));
        assert!(out.stdout.contains("300s"));
    }

    #[test]
    fn test_status_disabled_telegram_and_heartbeat() {
        let tmp = tempfile::TempDir::new().unwrap();
        let config_json = r#"{
            "providers": { "openai": { "api_key": "sk-test" } },
            "channels": { "telegram": { "enabled": false } },
            "heartbeat": { "enabled": false }
        }"#;
        std::fs::write(tmp.path().join("config.json"), config_json).unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("disabled"));
    }

    #[test]
    fn test_gateway_no_config() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        // cmd_gateway_run uses eprintln directly, so we test through run_with_output
        // (which routes "gateway" to a hint message since the real gateway path
        // goes through run() -> cmd_gateway_run)
        let out = run_with_output(args("gateway"), &ctx);
        assert_eq!(out.exit_code, 0);
    }

    #[test]
    fn test_help_text_includes_all_commands() {
        let mut out = String::new();
        help_text(&mut out);
        assert_contains_all(
            &out,
            &[
                "onboard", "agent", "auth", "gateway", "status", "cron", "skills", "help",
                "version",
            ],
        );
    }

    #[test]
    fn test_version_text_includes_semver() {
        let mut out = String::new();
        version_text(&mut out);
        assert!(out.starts_with("quecto "));
        // Should match semver pattern
        let version_part = out.trim().strip_prefix("quecto ").unwrap();
        let parts: Vec<&str> = version_part.split('.').collect();
        assert_eq!(parts.len(), 3, "expected semver, got: {}", version_part);
    }

    #[test]
    fn test_cli_context_default_base_dir() {
        let ctx = CliContext::default();
        let base = ctx.base_dir();
        // Should end with .quecto (either from home dir or fallback)
        assert!(
            base.to_string_lossy().contains(".quecto") || base.to_string_lossy().contains("quecto"),
            "base dir should contain 'quecto': {}",
            base.display()
        );
    }

    #[test]
    fn test_cli_context_override_base_dir() {
        let ctx = CliContext {
            base_dir: Some(PathBuf::from("/tmp/test-quecto")),
            ..Default::default()
        };
        assert_eq!(ctx.base_dir(), PathBuf::from("/tmp/test-quecto"));
    }

    // --- Auth CLI tests ---

    #[test]
    fn test_auth_login_stores_token_openai() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(
            args("auth login --provider openai --token sk-test-openai"),
            &ctx,
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("stored"));

        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        assert!(store.exists("openai").unwrap());
        let cred = store.get("openai").unwrap().unwrap();
        assert_eq!(cred.token, "sk-test-openai");
    }

    #[test]
    fn test_auth_login_stores_token_anthropic() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(
            args("auth login --provider anthropic --token sk-ant-test"),
            &ctx,
        );
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("stored"));
    }

    #[test]
    fn test_auth_login_missing_provider() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth login --token sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--provider"));
    }

    #[test]
    fn test_auth_login_missing_token() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth login --provider openai"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--token"));
    }

    #[test]
    fn test_auth_logout_removes_credential() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        // First store a credential
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "openai".to_string(),
                token: "sk-test".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let out = run_with_output(args("auth logout --provider openai"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("removed"));
        assert!(!store.exists("openai").unwrap());
    }

    #[test]
    fn test_auth_logout_nonexistent_is_noop() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth logout --provider openai"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("no credential"));
    }

    #[test]
    fn test_auth_status_shows_credentials() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "openai".to_string(),
                token: "sk-test".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: None,
            })
            .unwrap();

        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("openai"));
        assert!(out.stdout.contains("active"));
    }

    #[test]
    fn test_auth_status_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("no credentials"));
    }

    #[test]
    fn test_auth_status_expired() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = crate::infrastructure::auth::credential_store::CredentialStore::new(tmp.path());
        store
            .store(crate::infrastructure::auth::credential_store::Credential {
                provider: "anthropic".to_string(),
                token: "expired-tok".to_string(),
                method: crate::infrastructure::auth::credential_store::AuthMethod::Token,
                expires_at: Some(0),
            })
            .unwrap();

        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth status"), &ctx);
        assert_eq!(out.exit_code, 0);
        assert!(out.stdout.contains("anthropic"));
        assert!(out.stdout.contains("expired"));
    }

    #[test]
    fn test_auth_no_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("missing subcommand"));
    }

    #[test]
    fn test_auth_unknown_subcommand() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth foobar"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown subcommand"));
    }

    #[test]
    fn test_auth_login_unknown_provider_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth login --provider groq --token sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown provider"));
        assert!(out.stderr.contains("groq"));
    }

    #[test]
    fn test_auth_login_empty_token_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        // We pass a token that's all whitespace
        let v = vec![
            "quecto".to_string(),
            "auth".to_string(),
            "login".to_string(),
            "--provider".to_string(),
            "openai".to_string(),
            "--token".to_string(),
            "   ".to_string(),
        ];
        let out = run_with_output(v, &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("must not be empty"));
    }

    #[test]
    fn test_auth_login_unknown_flag_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth login --provider openai --tokn sk-test"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown flag"));
        assert!(out.stderr.contains("--tokn"));
    }

    #[test]
    fn test_auth_logout_unknown_flag_rejected() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("auth logout --provder openai"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("unknown flag"));
    }

    // ===================================================================
    // Agent headless one-shot mode tests
    // ===================================================================

    #[test]
    fn test_agent_no_message_shows_usage_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        // Write a config so it doesn't fail on "config not found"
        std::fs::write(
            tmp.path().join("config.json"),
            r#"{"providers":{"openai":{"api_key":"sk-test"}}}"#,
        )
        .unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("agent"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(
            out.stderr
                .contains("agent: -m is required for non-interactive mode"),
            "expected usage error, got stderr: {}",
            out.stderr
        );
    }

    #[test]
    fn test_agent_missing_config_shows_instructions() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No config file written
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("agent -m hello"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(
            out.stderr.contains("config not found"),
            "expected 'config not found', got stderr: {}",
            out.stderr
        );
        assert!(
            out.stderr.contains("quecto onboard"),
            "expected 'quecto onboard', got stderr: {}",
            out.stderr
        );
    }

    #[test]
    fn test_agent_no_providers_shows_error() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(
            tmp.path().join("config.json"),
            r#"{"providers":{"openai":{"api_key":""},"anthropic":{"api_key":""}}}"#,
        )
        .unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("agent -m hello"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(
            out.stderr.contains("no LLM providers"),
            "expected 'no LLM providers', got stderr: {}",
            out.stderr
        );
    }

    #[test]
    fn test_agent_parses_system_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        // No config — we just test flag parsing, not execution
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        // With no config, it should fail on "config not found" regardless of flags
        let out = run_with_output(
            vec![
                "quecto".into(),
                "agent".into(),
                "--system".into(),
                "You are a pirate".into(),
                "-m".into(),
                "Hello".into(),
            ],
            &ctx,
        );
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("config not found"));
    }

    #[test]
    fn test_agent_parses_model_flag() {
        let tmp = tempfile::TempDir::new().unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(
            vec![
                "quecto".into(),
                "agent".into(),
                "--model".into(),
                "gpt-5-mini".into(),
                "-m".into(),
                "Hello".into(),
            ],
            &ctx,
        );
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("config not found"));
    }

    #[test]
    fn test_agent_system_flag_missing_value() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("agent --system"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--system requires a value"));
    }

    #[test]
    fn test_agent_model_flag_missing_value() {
        let tmp = tempfile::TempDir::new().unwrap();
        std::fs::write(tmp.path().join("config.json"), "{}").unwrap();
        let ctx = CliContext {
            base_dir: Some(tmp.path().to_path_buf()),
            ..Default::default()
        };
        let out = run_with_output(args("agent --model"), &ctx);
        assert_eq!(out.exit_code, 1);
        assert!(out.stderr.contains("--model requires a value"));
    }

    #[test]
    fn test_agent_session_flag_parses_name() {
        let mut stderr = String::new();
        let a = vec!["-s".into(), "my-chat".into(), "-m".into(), "Hi".into()];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert_eq!(flags.session_name.as_deref(), Some("my-chat"));
    }

    #[test]
    fn test_agent_session_flag_ephemeral() {
        let mut stderr = String::new();
        let a = vec!["-s".into(), "-".into(), "-m".into(), "Hi".into()];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert_eq!(flags.session_name.as_deref(), Some("-"));
    }

    #[test]
    fn test_agent_session_flag_default_when_absent() {
        let mut stderr = String::new();
        let a = vec!["-m".into(), "Hi".into()];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert!(flags.session_name.is_none());
    }

    #[test]
    fn test_agent_session_key_derivation() {
        // Default: no -s flag -> cli:default
        assert_eq!(Session::build_key("cli", "default"), "cli:default");
        // Named: -s foo -> cli:foo
        assert_eq!(Session::build_key("cli", "my-chat"), "cli:my-chat");
    }

    // QUECTO_BASE_DIR env-var override is tested via BDD:
    // agent_cli.feature "QUECTO_BASE_DIR environment variable overrides default base directory"
    // Env-var tests live in BDD (not here) because set_var requires an unsound block.

    #[test]
    fn test_session_name_validation() {
        assert!(is_valid_session_name("my-chat"));
        assert!(is_valid_session_name("chat_1"));
        assert!(is_valid_session_name("ALLCAPS"));
        assert!(is_valid_session_name("-")); // ephemeral
        assert!(!is_valid_session_name("../../tmp/evil"));
        assert!(!is_valid_session_name("foo/bar"));
        assert!(!is_valid_session_name(".."));
        assert!(!is_valid_session_name(""));
        assert!(!is_valid_session_name("a b")); // spaces
        assert!(!is_valid_session_name("a:b")); // colons
    }

    #[test]
    fn test_agent_rejects_path_traversal_session_name() {
        let mut stderr = String::new();
        let a = vec![
            "-s".into(),
            "../../tmp/evil".into(),
            "-m".into(),
            "Hi".into(),
        ];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("alphanumeric"));
    }

    #[test]
    fn test_agent_parses_max_iterations_flag() {
        let mut stderr = String::new();
        let a: Vec<String> = vec![
            "--max-iterations".into(),
            "5".into(),
            "-m".into(),
            "Hi".into(),
        ];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert_eq!(flags.max_iterations, Some(5));
    }

    #[test]
    fn test_agent_parses_max_time_flag() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-time".into(), "30".into(), "-m".into(), "Hi".into()];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert_eq!(flags.max_time, Some(30));
    }

    #[test]
    fn test_agent_max_iterations_missing_value() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-iterations".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("--max-iterations requires a value"));
    }

    #[test]
    fn test_agent_max_time_missing_value() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-time".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("--max-time requires a value"));
    }

    #[test]
    fn test_agent_max_iterations_invalid_value() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-iterations".into(), "abc".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("positive integer"));
    }

    #[test]
    fn test_agent_max_time_invalid_value() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-time".into(), "xyz".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("positive integer"));
    }

    #[test]
    fn test_agent_max_iterations_zero_rejected() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-iterations".into(), "0".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("positive integer"));
    }

    #[test]
    fn test_agent_max_time_zero_rejected() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["--max-time".into(), "0".into()];
        let result = parse_agent_flags(&a, &mut stderr);
        assert!(result.is_none());
        assert!(stderr.contains("positive integer"));
    }

    #[test]
    fn test_agent_max_iterations_absent_is_none() {
        let mut stderr = String::new();
        let a: Vec<String> = vec!["-m".into(), "Hi".into()];
        let flags = parse_agent_flags(&a, &mut stderr).unwrap();
        assert!(flags.max_iterations.is_none());
        assert!(flags.max_time.is_none());
    }

    // Skill prompt loading tests moved to src/interface/shared.rs
}
