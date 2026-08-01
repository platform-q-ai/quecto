use std::collections::HashMap;
use std::sync::Arc;

use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::domain::agent::AgentLoop;
use crate::domain::message::Message;
use crate::domain::session::{Session, SessionStore};
use crate::infrastructure::config::Config;
use crate::infrastructure::extensions::registry::ExtensionRegistry;
use crate::infrastructure::model_registry::ModelRegistry;
use crate::infrastructure::persistence::session_store::FileSessionStore;

/// Max byte length for `--socket` paths.  Linux allows 108, macOS 104;
/// we use the stricter limit for portability.
const MAX_SOCKET_PATH_BYTES: usize = 104;

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
pub(crate) use flag_parse::AgentFlags;
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
    let mut spawned = false;
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            f @ ("--no-session" | "--no-sandbox" | "--persist" | "--workflow"
            | "--workflow-guards" | "--spawned") => {
                *match f {
                    "--no-session" => &mut no_session,
                    "--no-sandbox" => &mut no_sandbox,
                    "--persist" => &mut persist,
                    "--workflow" => &mut workflow,
                    "--spawned" => &mut spawned,
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
        spawned,
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

    // Build system prompt: top-level parent guidance, or minimal child prompt (#1319).
    let mut system =
        crate::interface::shared::build_system_prompt(&flags.system_prompt, flags.spawned);
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
    pub provider_reload: crate::interface::cli::provider_reload::ProviderReload,
    pub provider_reload_inputs: crate::interface::cli::provider_reload::ProviderReloadInputs,
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
    let provider_reload = crate::interface::cli::provider_reload::seeded_provider_reload_with_base(
        config_path,
        Some(base_dir.to_path_buf()),
        provider.clone(),
    );
    let provider_reload_inputs = crate::interface::cli::provider_reload::ProviderReloadInputs::new(
        config_path.to_path_buf(),
        base_dir.to_path_buf(),
        env_overrides.clone(),
        http_client.clone(),
    );

    // Workflow template discovery (slice 2) resolves against the process
    // working directory and the user's home directory.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home_dir = crate::infrastructure::tools::path_utils::home_dir();
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
    } = match build_tool_registry(ToolRegistryArgs {
        base_dir,
        config: &config,
        http_client: &http_client,
        flags,
        stderr,
        broadcast_tx,
        cwd: &cwd,
        home_dir,
    }) {
        Ok(build) => build,
        Err(error) => {
            // Fail fast at startup: a broken workflow template file must not
            // silently degrade into a session with a partial library.
            stderr.push_str(&format!("{error}\n"));
            return None;
        }
    };

    let effort = flags.effort.or_else(|| {
        config.agents.defaults.effort.as_deref().and_then(|s| {
            crate::domain::provider::EffortLevel::parse(s).or_else(|| {
                // Defensive only: Config::load rejects unknown efforts (#1066).
                let valid = crate::domain::provider::EffortLevel::VALID_VALUES;
                stderr.push_str(&format!(
                    "WARNING: invalid effort level '{s}' in config; expected one of: {valid}; ignoring\n"
                ));
                None
            })
        })
    });

    // #1113: an explicit `--workflow` session arms the idle-boundary template
    // selector nudge — the selector reaches the model through the nudge
    // channel and the workflow tool description, never through the system
    // prompt, which stays byte-identical for the whole session.
    if flags.workflow {
        if let Some(ws) = &workflow_state {
            if let Ok(mut engine) = ws.lock() {
                engine.set_selector_nudge(true);
            }
        }
    }
    let wf_config = workflow_state.as_ref().map(|_| config.workflow.clone());
    // #935/#1044: one registry load supplies the per-model output cap (clamps
    // max_tokens so low-limit models never get a larger value; set_model
    // re-derives on switch) and the known context window (bounds the budget).
    let (cap, window) = ModelRegistry::model_limits_from_base_dir(base_dir, &model);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key,
        context_collapse_after_tool_calls: config.agents.defaults.context_collapse_after_tool_calls,
        max_context_tokens: config.agents.defaults.max_context_tokens,
        progress_callback: None,
        streaming: false,
        effort,
        audit_log: None,
        // #1044/#1045/#1046: constructor fields — config cannot be dropped.
        pin_recent_turns: config.agents.defaults.pin_recent_turns,
        context_collapse_after_messages: config.agents.defaults.context_collapse_after_messages,
        model_context_window: window,
        tool_profile_context: if flags.spawned {
            crate::domain::tool::ToolProfileContext::Child
        } else {
            crate::domain::tool::ToolProfileContext::Parent
        },
    })
    .with_max_tool_iterations(
        flags
            .max_iterations
            .unwrap_or(config.agents.defaults.max_tool_iterations),
    )
    .with_model_max_tokens(cap);

    Some(AgentBuildResult {
        agent,
        workflow_config: wf_config,
        extension_prompt_snippets,
        model,
        ext_registry: std::sync::Arc::new(std::sync::Mutex::new(ext_registry)),
        notification_rx,
        subagent_registry,
        workflow_state,
        provider_reload,
        provider_reload_inputs,
    })
}

mod agent_tool_registry;
use agent_tool_registry::{ToolRegistryArgs, ToolRegistryBuild, build_tool_registry};

pub(crate) fn run_agent_session(
    base_dir: &std::path::Path,
    mut agent: AgentLoopImpl,
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

    // System prompt is injected at call time, never persisted. Track its
    // message ID, not its position: mid-run pruning can shift indices left,
    // making a positional `remove(idx)` delete the wrong message (#1073).
    let system_prompt_id = flags.system_prompt.as_deref().map(|sp| {
        let msg = Message::system(sp.to_string());
        let id = msg.id();
        messages.push(msg);
        id
    });

    let message = flags.message.as_deref().unwrap_or("");
    messages.push(Message::user(message.to_string()));

    let agent_result = if let Some(secs) = flags.max_time {
        match run_with_deadline(&rt, &mut agent, &mut messages, secs) {
            DeadlineResult::Completed(inner) => inner,
            DeadlineResult::TimedOut => {
                out.stderr.push_str("max-time exceeded\n");
                scrub_ephemeral_spill(base_dir, ephemeral);
                return 2;
            }
        }
    } else {
        rt.block_on(agent.process(&mut messages))
    };
    // Nothing an ephemeral run spilled for in-run recall may outlive the run.
    scrub_ephemeral_spill(base_dir, ephemeral);

    match agent_result {
        Ok(result) => {
            if !ephemeral {
                // Identity-based removal: immune to index shifts from
                // mid-run pruning (a no-op if pruning dropped it).
                if let Some(id) = system_prompt_id
                    && let Some(idx) = messages.iter().position(|m| m.id() == id)
                {
                    messages.remove(idx);
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

use crate::interface::shared::scrub_ephemeral_spill;

/// Run the agent with a wall-clock deadline.
///
/// Uses `thread::scope` + `recv_timeout` so the deadline is enforced without
/// requiring `tokio::time::timeout` (which needs an active reactor context
/// that may conflict with test harness runtimes). After timeout, the scoped
/// thread still runs until the in-flight LLM/tool call completes (bounded by
/// per-tool and HTTP client timeouts), then the scope exits.
pub(crate) fn run_with_deadline(
    rt: &tokio::runtime::Runtime,
    agent: &mut AgentLoopImpl,
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
    agent.set_session_key(session_key.clone());

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

    // Build the base system prompt. It is static for the lifetime of the
    // session (#1113): workflow state is never appended, so the provider-side
    // cached prefix survives every workflow step. Dynamic workflow state
    // reaches the model through tool results and idle-boundary nudges.
    let mut system_prompt =
        crate::interface::shared::build_system_prompt(&flags.system_prompt, flags.spawned);
    crate::interface::shared::append_extension_prompt(
        &mut system_prompt,
        &build.extension_prompt_snippets,
    );

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

    let mut provider_reload = build.provider_reload;
    let code = crate::interface::cli::uds::run_uds_loop(crate::interface::cli::uds::UdsLoopArgs {
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
        provider_reload: Some(&mut provider_reload),
        provider_reload_inputs: Some(&build.provider_reload_inputs),
    });
    // An ephemeral UDS server persisted spill content only for in-run recall.
    scrub_ephemeral_spill(&base_dir, ephemeral);
    code
}

#[path = "agent_provider.rs"]
mod agent_provider;
pub use agent_provider::build_agent_provider;
#[cfg(test)]
#[path = "agent_935_clamp_tests.rs"]
mod clamp_935_tests;
#[cfg(test)]
#[path = "agent_config_tests.rs"]
mod config_tests;
#[cfg(test)]
#[path = "agent_cov_tests.rs"]
mod cov_tests;
#[cfg(test)]
#[path = "agent_1048_ctx_wiring_tests.rs"]
mod ctx_wiring_1048_tests;
#[cfg(test)]
#[path = "agent_integration_tests.rs"]
mod integration_tests;
#[cfg(test)]
#[path = "agent_926_tests.rs"]
mod issue_926_tests;
#[cfg(test)]
#[path = "agent_no_sandbox_tests.rs"]
mod no_sandbox_tests;
#[cfg(test)]
#[path = "agent_no_session_tests.rs"]
mod no_session_tests;
#[cfg(test)]
#[path = "agent_provider_1066_tests.rs"]
mod provider_1066_tests;
#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_workflow_discovery_tests.rs"]
mod workflow_discovery_tests;
#[cfg(test)]
#[path = "agent_workflow_spec_tests.rs"]
mod workflow_spec_tests;
