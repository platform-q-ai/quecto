use std::collections::HashMap;
use std::sync::Arc;

use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;
use crate::domain::provider::LlmProvider;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::auth::credential_store::CredentialStore;
use crate::infrastructure::config::Config;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::providers;
use crate::infrastructure::providers::fallback::FallbackProvider;
use crate::infrastructure::providers::refreshable::{RefreshableConfig, RefreshableProvider};
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;

/// Operating mode for the `agent` subcommand.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) enum AgentMode {
    /// One-shot mode (default): run one prompt then exit.
    #[default]
    OneShot,
    /// UDS mode: read JSON commands from a Unix domain socket, stream events back.
    Uds,
}

/// Parsed flags for the `agent` subcommand.
pub(crate) struct AgentFlags {
    /// Session name for persistence. `None` = "default", `Some("-")` = ephemeral.
    pub(crate) session_name: Option<String>,
    /// When true, run in ephemeral mode: no session is loaded or saved.
    /// Mutually exclusive with `session_name`.
    pub(crate) no_session: bool,
    pub(crate) message: Option<String>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) model_override: Option<String>,
    /// Override max tool iterations (takes precedence over config).
    pub(crate) max_iterations: Option<u32>,
    /// Wall-clock timeout in seconds for the entire agent run.
    pub(crate) max_time: Option<u64>,
    /// Operating mode (default: one-shot).
    pub(crate) mode: AgentMode,
    /// When true, disable workspace path restriction for all filesystem tools.
    /// Overrides `config.agents.defaults.restrict_to_workspace`.
    /// WARNING: allows the agent to read/write any path on the system.
    pub(crate) no_sandbox: bool,
    /// When true, enable network access inside bash tool calls by disabling
    /// nsjail's network namespace isolation (`--disable_clone_newnet`).
    /// Overrides `config.tools.exec.network_passthrough`.
    /// WARNING: allows bash commands to make outbound network connections.
    pub(crate) network: bool,
}

/// Bundles the stdout/stderr pair passed through the agent pipeline.
pub(crate) struct AgentOutput<'a> {
    pub(crate) stdout: &'a mut String,
    pub(crate) stderr: &'a mut String,
}

/// Outcome of a deadline-bounded agent run.
pub(crate) enum DeadlineResult {
    /// Agent completed (successfully or with error) within the deadline.
    Completed(Result<crate::domain::agent::AgentResult, crate::domain::error::DomainError>),
    /// The deadline expired before the agent finished.
    TimedOut,
}

/// Return `args[i+1]` or push `err_msg` to stderr and return `None`.
fn next_arg<'a>(
    args: &'a [String],
    i: usize,
    err_msg: &str,
    stderr: &mut String,
) -> Option<&'a str> {
    if i + 1 < args.len() {
        Some(args[i + 1].as_str())
    } else {
        stderr.push_str(&format!("agent: {err_msg}\n"));
        None
    }
}

/// Parse a positive non-zero u32 for `--max-iterations`.
fn parse_pos_u32(val: &str, flag: &str, stderr: &mut String) -> Option<u32> {
    match val.parse::<u32>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

/// Parse a positive non-zero u64 for `--max-time`.
fn parse_pos_u64(val: &str, flag: &str, stderr: &mut String) -> Option<u64> {
    match val.parse::<u64>() {
        Ok(n) if n > 0 => Some(n),
        _ => {
            stderr.push_str(&format!("agent: {flag} requires a positive integer\n"));
            None
        }
    }
}

pub(crate) fn parse_agent_flags(args: &[String], stderr: &mut String) -> Option<AgentFlags> {
    let mut session_name: Option<String> = None;
    let mut no_session = false;
    let mut message: Option<String> = None;
    let mut system_prompt: Option<String> = None;
    let mut model_override: Option<String> = None;
    let mut max_iterations: Option<u32> = None;
    let mut max_time: Option<u64> = None;
    let mut mode = AgentMode::OneShot;
    let mut no_sandbox = false;
    let mut network = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--no-session" => {
                no_session = true;
                i += 1;
            }
            "--no-sandbox" => {
                no_sandbox = true;
                i += 1;
            }
            "--network" => {
                network = true;
                i += 1;
            }
            "-s" | "--session" => {
                let name = next_arg(args, i, "-s requires a session name", stderr)?;
                if !super::is_valid_session_name(name) {
                    stderr.push_str(
                        "agent: session name must contain only alphanumeric, '-', or '_'\n",
                    );
                    return None;
                }
                session_name = Some(name.to_string());
                i += 2;
            }
            "-m" | "--message" => {
                let val = next_arg(args, i, "-m requires a message", stderr)?;
                message = Some(val.to_string());
                i += 2;
            }
            "--system" => {
                let val = next_arg(args, i, "--system requires a value", stderr)?;
                system_prompt = Some(val.to_string());
                i += 2;
            }
            "--model" => {
                let val = next_arg(args, i, "--model requires a value", stderr)?;
                model_override = Some(val.to_string());
                i += 2;
            }
            "--max-iterations" => {
                let val = next_arg(args, i, "--max-iterations requires a value", stderr)?;
                max_iterations = Some(parse_pos_u32(val, "--max-iterations", stderr)?);
                i += 2;
            }
            "--max-time" => {
                let val = next_arg(args, i, "--max-time requires a value", stderr)?;
                max_time = Some(parse_pos_u64(val, "--max-time", stderr)?);
                i += 2;
            }
            "--mode" => {
                let val = next_arg(args, i, "--mode requires a value (e.g. uds)", stderr)?;
                mode = match val {
                    "uds" => AgentMode::Uds,
                    other => {
                        stderr.push_str(&format!(
                            "agent: --mode '{other}' is not valid; supported: uds\n"
                        ));
                        return None;
                    }
                };
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    if no_session && session_name.is_some() {
        stderr.push_str("agent: --no-session and -s are mutually exclusive\n");
        return None;
    }

    Some(AgentFlags {
        session_name,
        no_session,
        message,
        system_prompt,
        model_override,
        max_iterations,
        max_time,
        mode,
        no_sandbox,
        network,
    })
}

pub(crate) fn cmd_agent(
    ctx: &CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let mut flags = match parse_agent_flags(args, stderr) {
        Some(f) => f,
        None => return 1,
    };

    // ── UDS mode ──────────────────────────────────────────────────────────────
    if flags.mode == AgentMode::Uds {
        return cmd_agent_uds(ctx, flags, stderr);
    }

    // ── One-shot mode (default) ───────────────────────────────────────────────
    if flags.message.is_none() {
        stderr.push_str("agent: -m is required for non-interactive mode\n");
        return 1;
    }

    let base_dir = ctx.base_dir();
    let agent = match build_agent_from_config(&base_dir, &flags, stderr) {
        Some(a) => a,
        None => return 1,
    };

    // Build system prompt: datetime preamble + skills + user prompt
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    flags.system_prompt = Some(crate::interface::shared::build_system_prompt(
        &skill_prompt,
        &flags.system_prompt,
    ));

    let mut out = AgentOutput { stdout, stderr };
    run_agent_session(&base_dir, agent, &flags, &mut out)
}

/// Load config, build provider, and construct the agent loop. Returns None on error.
pub(crate) fn build_agent_from_config(
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

    let workspace = crate::interface::shared::resolve_agent_workspace(
        &config.workspace_path(),
        flags.no_sandbox,
    );
    let model = flags
        .model_override
        .clone()
        .unwrap_or(config.agents.defaults.model.clone());
    // --no-sandbox overrides config: disables workspace path restriction for all
    // filesystem tools. The dangerous-command denylist remains active regardless.
    let restrict_to_workspace = !flags.no_sandbox && config.agents.defaults.restrict_to_workspace;
    if flags.no_sandbox {
        stderr.push_str("WARNING: --no-sandbox is active — workspace path restriction disabled\n");
    }
    let sandbox = Sandbox::new(Some(workspace.clone()), restrict_to_workspace);
    // Build exec settings from config first, then apply CLI overrides.
    // ORDERING: overrides must happen before with_core_tools_and_exec_settings — do not reorder.
    let mut exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    // --network overrides config: enables network access inside bash tool calls by
    // disabling nsjail's network namespace isolation. nsjail still runs for all other
    // isolation (filesystem, PIDs, memory, CPU). The dangerous-command denylist
    // remains active regardless.
    if flags.network {
        exec_settings.network_passthrough = true;
        stderr
            .push_str("WARNING: --network is active — bash network namespace isolation disabled\n");
        tracing::warn!("--network: bash network namespace isolation disabled");
    }
    // Capture network_passthrough before exec_settings is moved into the registry.
    let effective_network = exec_settings.network_passthrough;
    let registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);
    let mut registry = registry;
    let session_key = if flags.no_session || flags.session_name.as_deref() == Some("-") {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };
    let spill_store = Arc::new(FileContextSpillStore::new(base_dir.to_path_buf()));
    registry.register(Arc::new(RecallTool::new(
        spill_store.clone(),
        session_key.clone(),
    )));
    // Propagate network_passthrough (which reflects both config and --network override)
    // so child agents spawned via SpawnTool inherit the same network posture as the
    // parent. The flag is also forwarded via argv (--network) so grandchild agents
    // are covered too.
    registry.register(Arc::new(
        SpawnTool::with_base_dir(vec![], restrict_to_workspace, base_dir.to_path_buf())
            .with_network(effective_network),
    ));
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model,
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key,
        context_collapse_after_turns: config.agents.defaults.context_collapse_after_turns,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
    })
    .with_max_tool_iterations(
        flags
            .max_iterations
            .unwrap_or(config.agents.defaults.max_tool_iterations),
    );

    Some(agent)
}

/// Run the agent loop with session load/save. Returns the exit code.
pub(crate) fn run_agent_session(
    base_dir: &std::path::Path,
    agent: AgentLoopImpl,
    flags: &AgentFlags,
    out: &mut AgentOutput<'_>,
) -> i32 {
    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };

    let session_store = FileSessionStore::new(base_dir);
    let rt = match super::build_tokio_runtime() {
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
        messages.push(Message::system(
            flags.system_prompt.as_deref().unwrap_or("").to_string(),
        ));
        Some(idx)
    } else {
        None
    };

    let message = flags.message.as_deref().unwrap_or("");
    messages.push(Message::user(message.to_string()));

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

/// Run the agent with a wall-clock deadline.
///
/// Uses `thread::scope` + `recv_timeout` so the deadline is enforced without
/// requiring `tokio::time::timeout` (which needs an active reactor context
/// that may conflict with test harness runtimes). After timeout, the scoped
/// thread still runs until the in-flight LLM/tool call completes (bounded by
/// per-tool and HTTP client timeouts), then the scope exits.
pub(crate) fn run_with_deadline(
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

/// Run the agent in UDS mode.
///
/// Validates config/provider, then enters the async JSON-lines loop.
/// Returns an exit code.
fn cmd_agent_uds(ctx: &CliContext, flags: AgentFlags, stderr: &mut String) -> i32 {
    let base_dir = ctx.base_dir();
    let agent = match build_agent_from_config(&base_dir, &flags, stderr) {
        Some(a) => a,
        None => return 1,
    };

    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        crate::domain::session::Session::build_key("cli", name)
    };

    let model = flags
        .model_override
        .clone()
        .or_else(|| {
            // Re-read config to get the default model.
            let config_path = base_dir.join("config.json");
            let env_overrides: HashMap<String, String> = std::env::vars()
                .filter(|(k, _)| k.starts_with("QUECTO_"))
                .collect();
            crate::infrastructure::config::Config::load_with_env(
                config_path.to_str().unwrap_or(""),
                &env_overrides,
            )
            .ok()
            .map(|c| c.agents.defaults.model)
        })
        .unwrap_or_else(|| "default".to_string());

    // Build system prompt (datetime preamble + skills + user-supplied --system),
    // mirroring what one-shot mode does before run_agent_session.
    // build_system_prompt() always returns a non-empty string (contains at
    // least the datetime preamble), so the UdsLoopArgs field is a plain String.
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    let system_prompt =
        crate::interface::shared::build_system_prompt(&skill_prompt, &flags.system_prompt);

    crate::interface::cli::uds::run_uds_loop(crate::interface::cli::uds::UdsLoopArgs {
        agent,
        base_dir: &base_dir,
        session_key,
        model,
        ephemeral,
        system_prompt,
        stdin_override: None,
        stdout_override: None,
        session_store_override: None,
    })
}

/// Build a FallbackProvider from config + credential store, suitable for the agent CLI.
///
/// OAuth-backed providers are wrapped in [`RefreshableProvider`] so that
/// expired tokens are automatically refreshed mid-session on 401 (issue #255).
pub fn build_agent_provider(
    config: &Config,
    base_dir: &std::path::Path,
) -> Result<Arc<dyn LlmProvider>, String> {
    let store = CredentialStore::new(base_dir);

    // Build a temporary runtime for token refresh if needed
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("failed to create runtime for token refresh: {}", e))?;

    let mut provider_list: Vec<Arc<dyn crate::domain::provider::LlmProvider>> = Vec::new();

    // Shared HTTP client for all providers — avoids duplicate connection pools and TLS contexts.
    let http_client = reqwest::Client::new();
    let store_arc = Arc::new(CredentialStore::new(base_dir));
    let refresh_fn = crate::interface::shared::make_oauth_refresh_fn();

    // Try OpenAI (with auto-refresh for expired OAuth tokens)
    let openai_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.openai.api_key,
        &store,
        "openai",
        &rt,
    );
    if !openai_key.is_empty() {
        let is_oauth = store.get("openai").ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });
        let openai_base = if config.providers.openai.api_base.is_empty() {
            None
        } else {
            Some(config.providers.openai.api_base.clone())
        };
        let inner = build_single_provider("openai", &openai_key, &openai_base, &http_client)?;
        if is_oauth {
            let factory = crate::interface::shared::make_provider_factory(
                "openai",
                openai_base,
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "openai".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
        } else {
            provider_list.push(inner);
        }
    }

    // Try Anthropic (with auto-refresh for expired OAuth tokens)
    let anthropic_key = crate::interface::shared::resolve_api_key_with_refresh(
        &config.providers.anthropic.api_key,
        &store,
        "anthropic",
        &rt,
    );
    if !anthropic_key.is_empty() {
        let is_oauth = store.get("anthropic").ok().flatten().is_some_and(|c| {
            c.method == crate::infrastructure::auth::credential_store::AuthMethod::OAuth
        });
        let anthropic_base = if config.providers.anthropic.api_base.is_empty() {
            None
        } else {
            Some(config.providers.anthropic.api_base.clone())
        };
        let inner =
            build_single_provider("anthropic", &anthropic_key, &anthropic_base, &http_client)?;
        if is_oauth {
            let factory = crate::interface::shared::make_provider_factory(
                "anthropic",
                anthropic_base,
                http_client.clone(),
            );
            provider_list.push(Arc::new(RefreshableProvider::new(RefreshableConfig {
                inner,
                store: store_arc.clone(),
                provider_name: "anthropic".to_string(),
                refresh_fn: refresh_fn.clone(),
                factory,
            })));
        } else {
            provider_list.push(inner);
        }
    }

    if provider_list.is_empty() {
        return Err(
            "no LLM providers configured (set an API key or run 'quecto auth login')".to_string(),
        );
    }

    Ok(Arc::new(FallbackProvider::new(provider_list)))
}

/// Build a single provider from name, key, and base URL.
fn build_single_provider(
    name: &str,
    api_key: &str,
    api_base: &Option<String>,
    http_client: &reqwest::Client,
) -> Result<Arc<dyn LlmProvider>, String> {
    if name == "openai" {
        let account_id = crate::infrastructure::auth::oauth::extract_openai_account_id(api_key);
        if let Some(acct) = account_id {
            return Ok(providers::create_codex_provider_with_client(
                api_key.to_string(),
                acct,
                http_client.clone(),
            ));
        }
    }
    let base = api_base.clone();
    providers::create_provider_with_client(name, api_key.to_string(), base, http_client.clone())
        .map_err(|e| format!("{} provider configuration error: {}", name, e))
}

#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "agent_integration_tests.rs"]
mod integration_tests;

#[cfg(test)]
#[path = "agent_no_session_tests.rs"]
mod no_session_tests;

#[cfg(test)]
#[path = "agent_no_sandbox_tests.rs"]
mod no_sandbox_tests;
