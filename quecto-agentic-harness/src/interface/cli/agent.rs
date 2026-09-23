use super::CliContext;
use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::configuration::dto::ConfigSelection;
use crate::infrastructure::config::Config;
use crate::infrastructure::extensions::registry::ExtensionRegistry;
use crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle;
use crate::infrastructure::tools::subagent_registry::{NotificationRx, SubagentRegistry};
use std::sync::Arc;
/// Max byte length for `--socket` paths (the portable macOS/Linux limit).
const MAX_SOCKET_PATH_BYTES: usize = 104;
pub(crate) struct AgentOutput<'a> {
    pub(crate) stdout: &'a mut String,
    pub(crate) stderr: &'a mut String,
}
mod agent_deadline;
mod flag_parse;
mod startup_effort;
mod startup_prompt;
mod swarm_runtime;
pub(crate) use agent_deadline::{DeadlineResult, run_with_deadline};
mod flag_private;
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
    let mut socket_path: Option<std::path::PathBuf> = None;
    let mut persist = false;
    let mut disabled_tools: Vec<String> = Vec::new();
    let mut effort: Option<crate::domain::provider::EffortLevel> = None;
    let mut workflow = false;
    let mut no_workflow_requested = false;
    let mut workflow_guards = false;
    let mut workflow_spec_path: Option<std::path::PathBuf> = None;
    let mut parent_id: Option<String> = None;
    let mut admission_context: Option<std::path::PathBuf> = None;
    let mut parent_control: Option<std::path::PathBuf> = None;
    let mut inherited_tool_policy_path: Option<std::path::PathBuf> = None;
    let mut spawned = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            f @ ("--no-session" | "--persist" | "--workflow" | "--workflow-guards"
            | "--spawned") => {
                *match f {
                    "--no-session" => &mut no_session,
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
            f @ ("--admission-context" | "--parent-control") => {
                let msg = format!("{f} requires a path");
                let val = next_arg(args, i, &msg, stderr)?;
                *(match f {
                    "--admission-context" => &mut admission_context,
                    _ => &mut parent_control,
                }) = Some(std::path::PathBuf::from(val));
                i += 2;
            }
            "--inherited-tool-policy-snapshot" => {
                inherited_tool_policy_path =
                    Some(flag_private::parse_snapshot_path(args, i, stderr)?);
                i += 2;
            }
            "--network" => {
                stderr.push_str(
                    "agent: WARNING: --network is deprecated and ignored; network isolation flag has been removed\n",
                );
                i += 1;
            }
            other if other.starts_with("--") || other.starts_with('-') => {
                stderr.push_str(&format!("agent: unknown flag '{other}'\n"));
                return None;
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
    let mut flags = AgentFlags {
        session_name,
        no_session,
        message,
        system_prompt,
        model_override,
        max_iterations,
        max_time,
        uds_mode,
        socket_path,
        persist,
        disabled_tools,
        effort,
        workflow,
        workflow_guards,
        workflow_disabled: no_workflow_requested,
        swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation::shared(),
        workflow_spec_path,
        inherited_tool_policy: None,
        parent_id,
        spawned,
        parent_identity_override: None,
        session_key_override: None,
        cwd_override: None,
        web_fetch_tool_factory: None,
        kill_tool: None,
        retention: None,
        catalogue: None,
        provider_runtime: None,
        tool_policy_persistence: None,
        configuration: None,
        admission: None,
        container_configs: None,
        environment_registry: None,
        stdin_is_tty: false,
        admission_context,
        parent_control,
    };
    flags = flag_parse::validate_agent_flags(flags, stderr)?;
    if let Some(path) = inherited_tool_policy_path {
        flag_private::load_inherited_tool_policy_for_valid_child(&path, stderr, &mut flags)?;
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

    let Some(mut flags) = parse_agent_flags(args, stderr) else {
        return 1;
    };
    flags.adopt_context(ctx);
    if !swarm_runtime::admit(&mut flags, stderr) {
        return 1;
    }

    if flags.uds_mode {
        return cmd_agent_uds(ctx, flags, stderr);
    }

    // ── One-shot mode (default) ───────────────────────────────────────────────
    if flags.message.is_none() {
        stderr.push_str("agent: -m is required for non-interactive mode\n");
        return 1;
    }

    let agents_instructions = match startup_prompt::load_agents_instructions(ctx, stderr) {
        Some(instructions) => instructions,
        None => return 1,
    };
    let parent_playbook = match startup_prompt::load_parent_playbook(ctx, flags.spawned, stderr) {
        Some(playbook) => playbook,
        None => return 1,
    };

    let base_dir = ctx.base_dir();
    let selection = match ctx.config_selection() {
        Ok(selection) => selection,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
    let build = match build_agent_from_config(&base_dir, &selection, &flags, stderr, None) {
        Some(r) => r,
        None => return 1,
    };

    flags.system_prompt = Some(startup_prompt::compose(
        agents_instructions.as_deref(),
        flags.system_prompt.as_deref(),
        flags.spawned,
        &build.extension_prompt_snippets,
        &parent_playbook,
    ));
    let mut out = AgentOutput { stdout, stderr };
    // The interface never constructs a session store (#1970): without
    // composition's builder there is nothing to persist against.
    let Some(sessions) = ctx.sessions else {
        out.stderr
            .push_str("agent: sessions capability not composed\n");
        return 1;
    };
    let code = run_agent_session(
        &base_dir,
        sessions,
        build.agent,
        &build.retention,
        &flags,
        &mut out,
    );
    admission_startup::shutdown();
    code
}
pub(crate) struct AgentBuildResult {
    pub agent: AgentLoopImpl,
    /// The run's catalogue handles (#1845, #1848), the same instances the
    /// spawn tool and the startup effort admission used.
    pub catalogue: crate::interface::cli::catalogue_handles::CatalogueHandles,
    /// The run's retained-context handles (D9 #1978): the store the loop's
    /// session recovers through and the scrub the ephemeral exit reaches.
    pub retention: crate::interface::cli::retention_handles::RetentionHandles,
    pub workflow_config: Option<crate::domain::workflow::WorkflowConfig>,
    pub extension_prompt_snippets: String,
    pub model: String,
    pub ext_registry: Arc<std::sync::Mutex<ExtensionRegistry>>,
    pub notification_rx: Option<NotificationRx>,
    pub subagent_registry: Option<SubagentRegistry>,
    pub harness_lifecycle: Option<SharedHarnessLifecycle>,
    /// The environment control slot (#2070) the loop hands its teardown.
    pub environment_control:
        Option<crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub workspace: std::path::PathBuf,
}
pub(crate) fn build_agent_from_config(
    base_dir: &std::path::Path,
    selection: &ConfigSelection,
    flags: &AgentFlags,
    stderr: &mut String,
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
) -> Option<AgentBuildResult> {
    let config_path = selection.path();
    // An explicit --config must exist; only a missing GLOBAL config falls
    // back to zero-config defaults.
    if let Some(msg) = super::selected_config_missing(config_path, selection.must_exist()) {
        stderr.push_str(&msg);
        stderr.push('\n');
        return None;
    }
    // Zero-config: a missing default config file loads defaults (no
    // onboarding step). The overlay (#2024) merges over it when trusted; a
    // one-shot run from a terminal may be asked to trust it, a UDS or
    // spawned run never prompts.
    let env_overrides = super::config_loading::quecto_env_overrides();
    let prompt_for_trust = !flags.uds_mode && !flags.spawned && flags.stdin_is_tty;
    let Some(build_configuration) = flags.configuration else {
        stderr.push_str("agent: configuration capability not composed\n");
        return None;
    };
    let loaded = match super::config_loading::load_selected_config(
        build_configuration,
        base_dir,
        selection,
        prompt_for_trust,
        &env_overrides,
        flags.admission_context.is_some(),
    ) {
        Ok(loaded) => loaded,
        Err(error) => {
            stderr.push_str(&error);
            stderr.push('\n');
            return None;
        }
    };
    for line in super::config_loading::layer_diagnostics(&loaded.sources) {
        stderr.push_str(&line);
        stderr.push('\n');
    }
    let config_sources = loaded.sources;
    let config = loaded.config;
    // The provider runtime (#1849): composed through the injected builder;
    // the interface never constructs provider state itself. Checked before
    // admission negotiation so a mis-composed binary fails without side
    // effects (no sidecar bound, no client built).
    let Some(build_provider) = flags.provider_runtime else {
        stderr.push_str("agent: provider runtime capability not composed\n");
        return None;
    };
    // The catalogue handles (#1845, #1848) are built once per run, after the
    // provider, before the tools and the loop that consume them; the
    // interface never constructs a catalogue use case.
    let Some(build_catalogue) = flags.catalogue else {
        stderr.push_str("agent: catalogue capability not composed\n");
        return None;
    };
    let Some(build_tool_policy_persistence) = flags.tool_policy_persistence else {
        stderr.push_str("agent: tool-policy persistence capability not composed\n");
        return None;
    };
    let Some(build_container_config_handles) = flags.container_configs else {
        stderr.push_str("agent: container-config handles capability not composed\n");
        return None;
    };
    if !admission_startup::negotiate_from_flags(&config, flags, stderr) {
        return None;
    }
    let http_client = crate::interface::shared::build_http_client();
    let provider = match build_provider(&config, base_dir, &http_client) {
        Ok(p) => p,
        Err(msg) => {
            stderr.push_str(&format!("{}\n", msg));
            return None;
        }
    };
    // Workflow templates resolve against CWD and home.
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let home_dir = crate::infrastructure::tools::path_utils::home_dir();
    let web_fetch_tool = config.tools.web.fetch.enabled.then(|| {
        assert!(
            flags.web_fetch_tool_factory.is_some(),
            "web-fetch factory missing"
        );
        flags.web_fetch_tool_factory.expect("checked")(
            http_client.clone(),
            config.tools.web.fetch.max_response_kb,
        )
    });
    // The run's reloadable configuration (#1849): the reload use case is
    // seeded now, after the startup composition read the same files.
    let runtime_inputs = crate::interface::cli::catalogue_handles::RuntimeConfigurationInputs {
        selection: selection.clone(),
        inherited_child: crate::infrastructure::admission::process::current()
            .is_some_and(|admission| admission.inherits_authority()),
        env_overrides: env_overrides.clone(),
        http_client: http_client.clone(),
        provider_runtime: build_provider,
        configuration: build_configuration,
    };
    let catalogue = build_catalogue(base_dir, Some(&runtime_inputs));
    // The container-config handles (#2024 S4a, S4c) are composed over this
    // run's own selection: the working directory's trusted overlay, never
    // the base directory, decides `container: true`, and the same layers
    // are what `get_container_configs` and the spawn description list.
    let container_configs = build_container_config_handles(base_dir, selection);
    let ToolRegistryBuild {
        registry,
        retention,
        session_key,
        model,
        ext_registry,
        extension_prompt_snippets,
        notification_rx,
        subagent_registry,
        harness_lifecycle,
        environment_control,
        workflow_state,
        workspace,
    } = match build_tool_registry(ToolRegistryArgs {
        base_dir,
        config_path,
        config: &config,
        http_client: &http_client,
        web_fetch_tool,
        effort_control: catalogue.effort.clone(),
        container_configs,
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
    let effort = startup_effort::admit(&catalogue.effort, flags.effort, &config, &model, stderr)?;
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
    // #935/#1044: the startup model's declared output cap (clamps max_tokens
    // so low-limit models never get a larger value) and context window
    // (bounds the budget) come from the change-active-model use case — the
    // same read a later set_model performs (#1847).
    let limits = catalogue.model.startup_limits(&model);
    let (cap, window) = (limits.max_output_tokens, limits.context_window);
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        retention: Some(retention.context.clone()),
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
    let mut agent = super::swarm_composition::wire_agent(
        agent,
        crate::interface::tool_runtime::swarm_context(),
    );
    // Durable `set_tool_policy … persist` writes into the run's config
    // file through composition's persistence hook (#1849).
    agent.set_tool_policy_persistence(Some(build_tool_policy_persistence(
        base_dir,
        &config_sources,
    )));
    Some(AgentBuildResult {
        agent,
        catalogue,
        retention,
        workflow_config: wf_config,
        extension_prompt_snippets,
        model,
        ext_registry: Arc::new(std::sync::Mutex::new(ext_registry)),
        notification_rx,
        subagent_registry,
        harness_lifecycle,
        environment_control,
        workflow_state,
        workspace,
    })
}
mod agent_tool_registry;
use agent_tool_registry::{ToolRegistryArgs, ToolRegistryBuild, build_tool_registry};
#[path = "agent/run_session.rs"]
mod run_session;
pub(crate) use run_session::run_agent_session;

#[path = "agent/startup_identity.rs"]
mod startup_identity;
use startup_identity::resolve_startup_identity;

fn cmd_agent_uds(ctx: &CliContext, mut flags: AgentFlags, stderr: &mut String) -> i32 {
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
    let Some((parent_control, lifetime)) = parent_control_startup::consume_and_resolve_lifetime(
        flags.parent_control.as_deref(),
        flags.persist,
        ctx.teardown_graph.is_some(),
        stderr,
    ) else {
        return 1;
    };

    // The interface never constructs a session store (#1970): the loop is
    // composed over the builder `main` handed in.
    let Some(sessions) = ctx.sessions else {
        stderr.push_str("agent: sessions capability not composed\n");
        return 1;
    };

    let ephemeral = flags.no_session || flags.session_name.as_deref() == Some("-");
    let Some(session_identity) = resolve_startup_identity(ctx, &flags, ephemeral, stderr) else {
        return 1;
    };
    let session_key = session_identity.runtime_key().to_string();
    if flags.session_name.is_none() && !ephemeral && flags.parent_identity_override.is_none() {
        flags.parent_identity_override = Some(session_key.clone());
        flags.session_key_override = Some(session_key.clone());
    }

    let agents_instructions = match startup_prompt::load_agents_instructions(ctx, stderr) {
        Some(instructions) => instructions,
        None => return 1,
    };
    let parent_playbook = match startup_prompt::load_parent_playbook(ctx, flags.spawned, stderr) {
        Some(playbook) => playbook,
        None => return 1,
    };

    let base_dir = ctx.base_dir();
    let selection = match ctx.config_selection() {
        Ok(selection) => selection,
        Err(error) => {
            stderr.push_str(&format!("{error}\n"));
            return 1;
        }
    };
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
        &selection,
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

    agent.set_session_key(session_identity.clone());

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
                    Arc::new(log) as Arc<dyn crate::application::audit::ports::AuditSink>
                ));
            }
            Err(e) => {
                stderr.push_str(&format!("WARNING: failed to open audit log: {e}\n"));
            }
        }
    }

    super::agent_commander_wiring::attach(&mut agent, &base_dir, &flags, stderr);

    let model = build.model.clone();

    // Build the base system prompt. It is static for the lifetime of the
    // session (#1113): workflow state is never appended, so the provider-side
    // cached prefix survives every workflow step. Dynamic workflow state
    // reaches the model through tool results and idle-boundary nudges.
    let system_prompt = startup_prompt::compose(
        agents_instructions.as_deref(),
        flags.system_prompt.as_deref(),
        flags.spawned,
        &build.extension_prompt_snippets,
        &parent_playbook,
    );

    // Use --socket path if provided; otherwise auto-generate in $XDG_RUNTIME_DIR or temp.
    let socket_path = flags
        .socket_path
        .clone()
        .unwrap_or_else(parent_control_startup::auto_socket_path);

    if !swarm_runtime::bind_socket(&socket_path, stderr) {
        return 1;
    }
    let retention = build.retention;
    let code = crate::interface::cli::uds::run_uds_loop(crate::interface::cli::uds::UdsLoopArgs {
        agent,
        retention: Some(retention.clone()),
        base_dir: &base_dir,
        workspace: &build.workspace,
        identity: session_identity,
        model,
        ephemeral,
        system_prompt,
        socket_path,
        socket_override: None,
        sessions,
        catalogue: build.catalogue,
        ext_registry: Some(build.ext_registry),
        lifetime,
        notification_rx: build.notification_rx,
        subagent_registry: build.subagent_registry,
        harness_lifecycle: build.harness_lifecycle,
        workflow_state: build.workflow_state,
        workflow_config: build.workflow_config,
        broadcast_tx,
        parent_control,
        teardown_graph: ctx.teardown_graph,
        environment_control: build.environment_control,
    });
    admission_startup::shutdown();
    // An ephemeral UDS server persisted spill content only for in-run recall.
    retention.recall.scrub_ephemeral(ephemeral);
    code
}

#[path = "agent/admission_startup.rs"]
mod admission_startup;
#[cfg(test)]
#[path = "agent_agents_md_tests.rs"]
mod agents_md_tests;
#[cfg(test)]
#[path = "agent_build_tests.rs"]
mod build_tests;
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
#[path = "agent_no_session_tests.rs"]
mod no_session_tests;
#[path = "agent/parent_control_startup.rs"]
mod parent_control_startup;
#[cfg(test)]
#[path = "agent_startup_identity_tests.rs"]
mod startup_identity_tests;
#[cfg(test)]
#[path = "agent_tests.rs"]
mod tests;
#[cfg(test)]
#[path = "agent_workflow_discovery_tests.rs"]
mod workflow_discovery_tests;
#[cfg(test)]
#[path = "agent_workflow_spec_tests.rs"]
mod workflow_spec_tests;
