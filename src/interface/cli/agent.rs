use std::collections::HashMap;
use std::sync::Arc;

use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::extensions::registry::ExtensionRegistry;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::persistence::session_store::FileSessionStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;

/// Max byte length for `--socket` paths.  Linux allows 108, macOS 104;
/// we use the stricter limit for portability.
const MAX_SOCKET_PATH_BYTES: usize = 104;

/// Parsed flags for the `agent` subcommand.
pub(crate) struct AgentFlags {
    pub(crate) session_name: Option<String>,
    pub(crate) no_session: bool,
    pub(crate) message: Option<String>,
    pub(crate) system_prompt: Option<String>,
    pub(crate) model_override: Option<String>,
    pub(crate) max_iterations: Option<u32>,
    pub(crate) max_time: Option<u64>,
    pub(crate) uds_mode: bool,
    pub(crate) no_sandbox: bool,
    pub(crate) network: bool,
    pub(crate) socket_path: Option<std::path::PathBuf>,
    pub(crate) persist: bool,
    pub(crate) disabled_tools: Vec<String>,
    pub(crate) effort: Option<crate::domain::provider::EffortLevel>,
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

/// Parse an effort level string, writing an error to `stderr` on failure.
fn parse_effort_level(
    val: &str,
    stderr: &mut String,
) -> Option<crate::domain::provider::EffortLevel> {
    crate::domain::provider::EffortLevel::parse(val).or_else(|| {
        stderr.push_str(&format!(
            "agent: invalid effort level '{}'; expected one of: low, medium, high, max\n",
            val
        ));
        None
    })
}

/// Parse and validate a session name from `args[i+1]`.
fn parse_session_name(args: &[String], i: usize, stderr: &mut String) -> Option<String> {
    let name = next_arg(args, i, "-s requires a session name", stderr)?;
    if !super::is_valid_session_name(name) {
        stderr.push_str("agent: session name must contain only alphanumeric, '-', or '_'\n");
        return None;
    }
    Some(name.to_string())
}

/// Parse the `--mode` flag value.  Returns `Some(true)` for `"uds"`, `None`
/// (with an error written to `stderr`) for any unknown value.
fn parse_agent_mode(val: &str, stderr: &mut String) -> Option<bool> {
    match val {
        "uds" => Some(true),
        other => {
            stderr.push_str(&format!(
                "agent: --mode '{other}' is not valid; supported: uds\n"
            ));
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
    let mut uds_mode = false;
    let mut no_sandbox = false;
    let mut network = false;
    let mut socket_path: Option<std::path::PathBuf> = None;
    let mut persist = false;
    let mut disabled_tools: Vec<String> = Vec::new();
    let mut effort: Option<crate::domain::provider::EffortLevel> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            f @ ("--no-session" | "--no-sandbox" | "--network" | "--persist") => {
                *match f {
                    "--no-session" => &mut no_session,
                    "--no-sandbox" => &mut no_sandbox,
                    "--network" => &mut network,
                    _ => &mut persist,
                } = true;
                i += 1;
            }
            "-s" | "--session" => {
                session_name = Some(parse_session_name(args, i, stderr)?);
                i += 2;
            }
            "-m" | "--message" => {
                let val = next_arg(args, i, "-m requires a message", stderr)?;
                message = Some(val.to_string());
                i += 2;
            }
            f @ ("--system" | "--model") => {
                let msg = format!("{f} requires a value");
                let val = next_arg(args, i, &msg, stderr)?;
                *(match f {
                    "--system" => &mut system_prompt,
                    _ => &mut model_override,
                }) = Some(val.to_string());
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
                uds_mode = parse_agent_mode(val, stderr)?;
                i += 2;
            }
            "--socket" => {
                let val = next_arg(args, i, "--socket requires a path", stderr)?;
                socket_path = Some(std::path::PathBuf::from(val));
                i += 2;
            }
            "--disable-tool" => {
                let val = next_arg(args, i, "--disable-tool requires a tool name", stderr)?;
                disabled_tools.push(val.to_string());
                i += 2;
            }
            "--effort" => {
                let val = next_arg(args, i, "--effort requires a value", stderr)?;
                effort = Some(parse_effort_level(val, stderr)?);
                i += 2;
            }
            "--config" => {
                // Value consumed globally by extract_config_flag; validate here too.
                let _val = next_arg(args, i, "--config requires a path", stderr)?;
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    let flags = AgentFlags {
        session_name,
        no_session,
        message,
        system_prompt,
        model_override,
        max_iterations,
        max_time,
        uds_mode,
        no_sandbox,
        network,
        socket_path,
        persist,
        disabled_tools,
        effort,
    };
    validate_agent_flags(flags, stderr)
}

/// Post-parse validation of mutually exclusive / dependent flags.
fn validate_agent_flags(flags: AgentFlags, stderr: &mut String) -> Option<AgentFlags> {
    if flags.no_session && flags.session_name.is_some() {
        stderr.push_str("agent: --no-session and -s are mutually exclusive\n");
        return None;
    }
    if flags.persist && !flags.uds_mode {
        stderr.push_str("agent: --persist requires --mode uds\n");
        return None;
    }
    Some(flags)
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
    if flags.uds_mode {
        return cmd_agent_uds(ctx, flags, stderr);
    }

    // ── One-shot mode (default) ───────────────────────────────────────────────
    if flags.message.is_none() {
        stderr.push_str("agent: -m is required for non-interactive mode\n");
        return 1;
    }

    let base_dir = ctx.base_dir();
    let config_path = ctx.config_path();
    let build = match build_agent_from_config(&base_dir, &config_path, &flags, stderr) {
        Some(r) => r,
        None => return 1,
    };

    // Build system prompt: datetime preamble + skills + extensions + workflow + user prompt
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    let mut system =
        crate::interface::shared::build_system_prompt(&skill_prompt, &flags.system_prompt);
    crate::interface::shared::append_extension_prompt(
        &mut system,
        &build.extension_prompt_snippets,
    );
    crate::interface::shared::append_workflow_prompt(&mut system, &build.workflow_config);
    flags.system_prompt = Some(system);
    let mut out = AgentOutput { stdout, stderr };
    run_agent_session(&base_dir, build.agent, &flags, &mut out)
}

pub(crate) struct AgentBuildResult {
    pub agent: AgentLoopImpl,
    pub workflow_config: crate::domain::workflow::WorkflowConfig,
    pub extension_prompt_snippets: String,
    pub model: String,
    pub ext_registry: std::sync::Arc<std::sync::Mutex<ExtensionRegistry>>,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
}

pub(crate) fn build_agent_from_config(
    base_dir: &std::path::Path,
    config_path: &std::path::Path,
    flags: &AgentFlags,
    stderr: &mut String,
) -> Option<AgentBuildResult> {
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

    let http_client = crate::interface::shared::build_http_client();

    let provider = match build_agent_provider(&config, base_dir, &http_client) {
        Ok(p) => p,
        Err(msg) => {
            stderr.push_str(&format!("{}\n", msg));
            return None;
        }
    };

    let ToolRegistryBuild {
        registry,
        spill_store,
        session_key,
        model,
        ext_registry,
        extension_prompt_snippets,
        notification_rx,
        subagent_registry,
        workflow_state,
        workflow_config: _workflow_config,
    } = build_tool_registry(ToolRegistryArgs {
        base_dir,
        config: &config,
        http_client: &http_client,
        flags,
        stderr,
    });

    // Remove disabled tools before boxing the registry (#402).
    let mut registry = registry;
    let warnings = registry.remove_all(&flags.disabled_tools);
    for name in &warnings {
        stderr.push_str(&format!(
            "WARNING: --disable-tool: no tool named '{}' in the registry\n",
            name
        ));
    }

    let effort = flags.effort.or_else(|| {
        config.agents.defaults.effort.as_deref().and_then(|s| {
            crate::domain::provider::EffortLevel::parse(s).or_else(|| {
                stderr.push_str(&format!(
                    "WARNING: invalid effort level '{}' in config; ignoring\n",
                    s
                ));
                None
            })
        })
    });

    let wf_config = config.workflow.clone();
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key,
        context_collapse_after_turns: config.agents.defaults.context_collapse_after_turns,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort,
    })
    .with_max_tool_iterations(
        flags
            .max_iterations
            .unwrap_or(config.agents.defaults.max_tool_iterations),
    );

    Some(AgentBuildResult {
        agent,
        workflow_config: wf_config,
        extension_prompt_snippets,
        model,
        ext_registry: std::sync::Arc::new(std::sync::Mutex::new(ext_registry)),
        notification_rx,
        subagent_registry,
        workflow_state,
    })
}

struct ToolRegistryBuild {
    registry: ToolRegistryImpl,
    spill_store: Arc<FileContextSpillStore>,
    session_key: String,
    model: String,
    ext_registry: ExtensionRegistry,
    extension_prompt_snippets: String,
    notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    workflow_config: crate::domain::workflow::WorkflowConfig,              // #562
}

struct ToolRegistryArgs<'a> {
    base_dir: &'a std::path::Path,
    config: &'a Config,
    http_client: &'a reqwest::Client,
    flags: &'a AgentFlags,
    stderr: &'a mut String,
}

fn build_tool_registry(args: ToolRegistryArgs<'_>) -> ToolRegistryBuild {
    let ToolRegistryArgs {
        base_dir,
        config,
        http_client,
        flags,
        stderr,
    } = args;
    let workspace = crate::interface::shared::resolve_agent_workspace(
        &config.workspace_path(),
        flags.no_sandbox,
    );
    let model = flags
        .model_override
        .clone()
        .unwrap_or(config.agents.defaults.model.clone());
    let restrict_to_workspace = !flags.no_sandbox && config.agents.defaults.restrict_to_workspace;
    if flags.no_sandbox {
        stderr.push_str("WARNING: --no-sandbox is active — workspace path restriction disabled\n");
    }
    let sandbox = Sandbox::new(Some(workspace.clone()), restrict_to_workspace);
    let mut exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(config);
    if flags.network {
        exec_settings.network_passthrough = true;
        stderr
            .push_str("WARNING: --network is active — bash network namespace isolation disabled\n");
        tracing::warn!("--network: bash network namespace isolation disabled");
    }
    let effective_network = exec_settings.network_passthrough;
    let mut registry =
        ToolRegistryImpl::with_core_tools_and_exec_settings(workspace, sandbox, exec_settings);
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
    let subagent_registry = AgentCmdTool::new_registry();
    let socket_dir = crate::interface::shared::xdg_runtime_dir_or_temp();
    let (notify_tx, notify_rx) =
        crate::infrastructure::tools::subagent_registry::new_notification_channel();
    registry.register(Arc::new(
        SpawnTool::with_base_dir(vec![], restrict_to_workspace, base_dir.to_path_buf())
            .with_network(effective_network)
            .with_socket_dir(socket_dir)
            .with_registry(subagent_registry.clone())
            .with_notify_tx(notify_tx),
    ));
    let subagent_registry_for_protocol = subagent_registry.clone();
    registry.register(Arc::new(AgentCmdTool::new(subagent_registry)));
    let wf_state = crate::interface::shared::register_workflow_tool(
        &mut registry,
        &config.workflow,
        None, // Emitter wired separately in UDS mode (#562)
    );
    let wf_config = config.workflow.clone();

    let ext_registry =
        crate::interface::shared::build_and_register_native_extensions(config, http_client);
    let extension_prompt_snippets = ext_registry.system_prompt_snippets();
    crate::interface::shared::register_extension_tools(&mut registry, &ext_registry);

    let has_base_dir = !base_dir.as_os_str().is_empty();
    ToolRegistryBuild {
        registry,
        spill_store,
        session_key,
        model,
        ext_registry,
        extension_prompt_snippets,
        notification_rx: if has_base_dir { Some(notify_rx) } else { None },
        subagent_registry: if has_base_dir {
            Some(subagent_registry_for_protocol)
        } else {
            None
        },
        workflow_state: wf_state,
        workflow_config: wf_config,
    }
}

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
/// Return the XDG runtime directory if it is set, exists, and is writable by
/// the current process; otherwise fall back to [`std::env::temp_dir`].
///
/// The XDG Base Directory Specification requires `$XDG_RUNTIME_DIR` to be
/// owned by the user and mode `0700`.  We additionally verify it is writable
/// before using it so a misconfigured or container-injected value does not
/// cause a confusing bind error later.
/// Validates config/provider, then enters the async JSON-lines loop.
/// Returns an exit code.
fn cmd_agent_uds(ctx: &CliContext, flags: AgentFlags, stderr: &mut String) -> i32 {
    // Early validation for user-supplied --socket paths: check length before
    // doing any I/O (config load, agent build).  Auto-generated paths are
    // always short, so we only gate on explicitly provided paths here.
    if let Some(ref p) = flags.socket_path {
        if p.as_os_str().len() > MAX_SOCKET_PATH_BYTES {
            stderr
                .push_str("agent: --socket path exceeds the Unix socket path limit (104 bytes)\n");
            return 1;
        }
    }
    if flags.persist {
        stderr.push_str("WARNING: --persist keeps the agent alive indefinitely. Shutdown via SIGTERM/SIGINT only.\n");
    }

    let base_dir = ctx.base_dir();
    let config_path = ctx.config_path();
    let build = match build_agent_from_config(&base_dir, &config_path, &flags, stderr) {
        Some(r) => r,
        None => return 1,
    };
    let mut agent = build.agent;
    // Enable incremental streaming so the UDS layer emits token events.
    agent.set_streaming(true);

    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let session_key = if ephemeral {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        crate::domain::session::Session::build_key("cli", name)
    };

    let model = build.model.clone();

    // Build system prompt: datetime preamble + skills + extensions + workflow + user prompt.
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    let mut system_prompt =
        crate::interface::shared::build_system_prompt(&skill_prompt, &flags.system_prompt);
    crate::interface::shared::append_extension_prompt(
        &mut system_prompt,
        &build.extension_prompt_snippets,
    );
    crate::interface::shared::append_workflow_prompt(&mut system_prompt, &build.workflow_config);

    // Use --socket path if provided; otherwise auto-generate in $XDG_RUNTIME_DIR or temp.
    let socket_path = flags.socket_path.clone().unwrap_or_else(|| {
        let dir = crate::interface::shared::xdg_runtime_dir_or_temp();
        // Best-effort: remove stale quecto-agent-*.sock files older than 24 h.
        // Drop guards do not run on SIGKILL so stale sockets can accumulate.
        crate::interface::cli::uds::reap_stale_sockets(
            &dir,
            std::time::Duration::from_secs(86_400),
        );
        let id = uuid::Uuid::new_v4();
        dir.join(format!("quecto-agent-{id}.sock"))
    });

    crate::interface::cli::uds::run_uds_loop(crate::interface::cli::uds::UdsLoopArgs {
        agent,
        base_dir: &base_dir,
        session_key,
        model,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override: None,
        session_store_override: None,
        ext_registry: Some(build.ext_registry),
        persist: flags.persist,
        notification_rx: build.notification_rx,
        subagent_registry: build.subagent_registry,
        workflow_state: build.workflow_state,
        workflow_config: Some(build.workflow_config),
    })
}

#[path = "agent_provider.rs"]
mod agent_provider;
pub use agent_provider::build_agent_provider;
#[cfg(test)]
#[path = "agent_config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "agent_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "agent_no_sandbox_tests.rs"]
mod no_sandbox_tests;
#[cfg(test)]
#[path = "agent_no_session_tests.rs"]
mod no_session_tests;
#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
