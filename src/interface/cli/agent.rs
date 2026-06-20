use std::collections::HashMap;
use std::sync::Arc;

use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::extensions::registry::ExtensionRegistry;
use crate::infrastructure::persistence::session_store::FileSessionStore;

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
    pub(crate) socket_path: Option<std::path::PathBuf>,
    pub(crate) persist: bool,
    pub(crate) disabled_tools: Vec<String>,
    pub(crate) effort: Option<crate::domain::provider::EffortLevel>,
    pub(crate) workflow: bool,
    pub(crate) workflow_guards: bool,
    pub(crate) workflow_disabled: bool,
    pub(crate) workflow_spec_path: Option<std::path::PathBuf>,
    /// `--parent-id`: the spawning agent's id, stamped onto this agent's emitted
    /// events so consumers can reconstruct the unit tree (PRD Stage B). `None`
    /// at the root.
    pub(crate) parent_id: Option<String>,
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

mod flag_parse;
use flag_parse::{
    next_arg, parse_agent_mode, parse_effort_level, parse_pos_u32, parse_pos_u64,
    parse_session_name,
};

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
    let mut socket_path: Option<std::path::PathBuf> = None;
    let mut persist = false;
    let mut disabled_tools: Vec<String> = Vec::new();
    let mut effort: Option<crate::domain::provider::EffortLevel> = None;
    let mut workflow = false;
    let mut no_workflow_requested = false;
    let mut workflow_guards = false;
    let mut workflow_spec_path: Option<std::path::PathBuf> = None;
    let mut parent_id: Option<String> = None;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            f @ ("--no-session" | "--no-sandbox" | "--persist" | "--workflow"
            | "--workflow-guards") => {
                *match f {
                    "--no-session" => &mut no_session,
                    "--no-sandbox" => &mut no_sandbox,
                    "--persist" => &mut persist,
                    "--workflow" => &mut workflow,
                    _ => &mut workflow_guards,
                } = true;
                if f == "--workflow" {
                    no_workflow_requested = false;
                }
                i += 1;
            }
            "--no-workflow" => {
                workflow = false;
                no_workflow_requested = true;
                workflow_guards = false;
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
            "--workflow-spec" => {
                let val = next_arg(args, i, "--workflow-spec requires a path", stderr)?;
                workflow_spec_path = Some(std::path::PathBuf::from(val));
                i += 2;
            }
            "--parent-id" => {
                let val = next_arg(args, i, "--parent-id requires a value", stderr)?;
                parent_id = Some(val.to_string());
                i += 2;
            }
            _ => {
                i += 1;
            }
        }
    }

    if (workflow || no_workflow_requested || workflow_guards || workflow_spec_path.is_some())
        && !uds_mode
    {
        stderr.push_str(
            "agent: --workflow, --no-workflow, --workflow-guards, and --workflow-spec require --mode uds\n",
        );
        return None;
    }

    if workflow_spec_path.is_some() && no_workflow_requested {
        stderr.push_str("agent: --workflow-spec cannot be combined with --no-workflow\n");
        return None;
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
        socket_path,
        persist,
        disabled_tools,
        effort,
        workflow,
        workflow_guards,
        workflow_disabled: no_workflow_requested,
        workflow_spec_path,
        parent_id,
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
    if flags.workflow_guards && flags.workflow_disabled {
        stderr.push_str("agent: --workflow-guards cannot be used with --no-workflow\n");
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
    // Headless mode logs to stderr; install the API-key-redacting subscriber so
    // any secret that reaches a log line is scrubbed. No-op unless RUST_LOG is set.
    crate::infrastructure::logging::install_redacting_subscriber();

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
    let build = match build_agent_from_config(
        &base_dir,
        &config_path,
        ctx.config_path.is_some(),
        &flags,
        stderr,
        None,
    ) {
        Some(r) => r,
        None => return 1,
    };

    // Build system prompt: datetime preamble + skills + extensions + user prompt.
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    let mut system =
        crate::interface::shared::build_system_prompt(&skill_prompt, &flags.system_prompt);
    crate::interface::shared::append_extension_prompt(
        &mut system,
        &build.extension_prompt_snippets,
    );
    flags.system_prompt = Some(system);
    let mut out = AgentOutput { stdout, stderr };
    run_agent_session(&base_dir, build.agent, &flags, &mut out)
}

pub(crate) struct AgentBuildResult {
    pub agent: AgentLoopImpl,
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,
    pub extension_prompt_snippets: String,
    pub model: String,
    pub ext_registry: std::sync::Arc<std::sync::Mutex<ExtensionRegistry>>,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workflow_prompt_initially_active: bool,
}

pub(crate) fn build_agent_from_config(
    base_dir: &std::path::Path,
    config_path: &std::path::Path,
    config_explicit: bool,
    flags: &AgentFlags,
    stderr: &mut String,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> Option<AgentBuildResult> {
    // An explicitly-provided --config path must exist; only a missing DEFAULT
    // config falls back to zero-config defaults.
    if let Some(msg) = super::explicit_config_missing(config_path, config_explicit) {
        stderr.push_str(&msg);
        stderr.push('\n');
        return None;
    }
    // Zero-config: a missing default config file loads defaults (no onboarding step).
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
    } = build_tool_registry(ToolRegistryArgs {
        base_dir,
        config: &config,
        http_client: &http_client,
        flags,
        stderr,
        broadcast_tx,
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

    let workflow_prompt_initially_active = flags.workflow;
    let wf_config = workflow_state.as_ref().map(|_| config.workflow.clone());
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
        system_prompt_provider: None,
        audit_log: None,
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
        workflow_prompt_initially_active,
    })
}

mod agent_tool_registry;
use agent_tool_registry::{ToolRegistryArgs, ToolRegistryBuild, build_tool_registry};

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

    if !ephemeral && !messages.is_empty() {
        rt.block_on(agent.prune_resumed_context(&mut messages));
    }

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
                    workflow_run: None,
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
/// Resolve the UDS session key. Ephemeral → empty (no persistence). An explicit
/// `--session` keeps the `cli:` namespace so internal sessions (sub-agents,
/// agent-manager) stay out of the user-facing `/resume` list. With no
/// `--session`, start a fresh per-launch user chat (`chat-` namespace) so each
/// interactive launch is a distinct, resumable conversation (PRD: new chat per
/// launch).
fn resolve_uds_session_key(ephemeral: bool, session_name: Option<&str>) -> String {
    if ephemeral {
        String::new()
    } else if let Some(name) = session_name {
        crate::domain::session::Session::build_key("cli", name)
    } else {
        crate::interface::shared::generate_chat_key()
    }
}

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
    // Create the broadcast channel early so the WorkflowTool emitter can
    // send workflow_state events from the moment it is constructed (#598).
    let workflow_available = flags.uds_mode && !flags.workflow_disabled;
    let broadcast_tx = if workflow_available {
        let (tx, _) = tokio::sync::broadcast::channel::<String>(
            crate::interface::cli::uds_multi::BROADCAST_CHANNEL_CAPACITY,
        );
        Some(tx)
    } else {
        None
    };
    let build = match build_agent_from_config(
        &base_dir,
        &config_path,
        ctx.config_path.is_some(),
        &flags,
        stderr,
        broadcast_tx.clone(),
    ) {
        Some(r) => r,
        None => return 1,
    };
    let mut agent = build.agent;
    // Enable incremental streaming so the UDS layer emits token events.
    agent.set_streaming(true);

    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let session_key = resolve_uds_session_key(ephemeral, flags.session_name.as_deref());

    // Keep durable audit logging tied to explicit workflow-driven mode. Normal UDS
    // makes workflow available, but should not add audit I/O/privacy overhead before
    // the user opts into autonomous workflow behavior.
    if flags.workflow && !ephemeral && !session_key.is_empty() {
        match crate::infrastructure::persistence::audit_log::AuditLog::open_sync(
            &base_dir,
            &session_key,
        ) {
            Ok(log) => {
                agent.set_audit_log(Some(
                    Arc::new(log) as Arc<dyn crate::domain::audit::AuditSink>
                ));
            }
            Err(e) => {
                stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
            }
        }
    }

    let model = build.model.clone();

    // Build the base system prompt; workflow is appended dynamically before each UDS turn.
    let skill_prompt = crate::interface::shared::load_skill_prompt(&base_dir);
    let mut system_prompt =
        crate::interface::shared::build_system_prompt(&skill_prompt, &flags.system_prompt);
    crate::interface::shared::append_extension_prompt(
        &mut system_prompt,
        &build.extension_prompt_snippets,
    );
    if let Some(workflow) = build.workflow_state.clone() {
        let base_prompt = system_prompt.clone();
        let workflow_for_provider = workflow.clone();
        let force_workflow_selector = build.workflow_prompt_initially_active;
        agent.set_system_prompt_provider(Some(Arc::new(move || {
            let mut prompt = base_prompt.clone();
            crate::interface::shared::append_workflow_prompt_if_active(
                &mut prompt,
                &workflow_for_provider,
                force_workflow_selector,
            );
            prompt
        })));
    }

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
        workflow_config: build.workflow_config,
        broadcast_tx,
    })
}

#[path = "agent_provider.rs"]
mod agent_provider;
pub use agent_provider::build_agent_provider;
#[cfg(test)]
#[path = "agent_config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "agent_cov_tests.rs"]
mod cov_tests;
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
#[cfg(test)]
#[path = "agent_workflow_spec_tests.rs"]
mod workflow_spec_tests;
