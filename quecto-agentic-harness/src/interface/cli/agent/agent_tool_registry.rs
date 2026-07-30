use super::*;
use crate::infrastructure::extensions::native::{
    AgentControlToolDeps, SessionToolDeps, build_agent_control_tool_extensions,
    build_session_tool_extensions, register_bundled_native_tools,
};
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

pub(super) struct ToolRegistryBuild {
    pub(super) registry: ToolRegistryImpl,
    pub(super) spill_store: Arc<FileContextSpillStore>,
    pub(super) session_key: String,
    pub(super) model: String,
    pub(super) ext_registry: ExtensionRegistry,
    pub(super) extension_prompt_snippets: String,
    pub(super) notification_rx:
        Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub(super) subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub(super) workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
}

pub(super) struct ToolRegistryArgs<'a> {
    pub(super) base_dir: &'a std::path::Path,
    pub(super) config: &'a Config,
    pub(super) http_client: &'a reqwest::Client,
    pub(super) flags: &'a AgentFlags,
    pub(super) stderr: &'a mut String,
    /// Broadcast channel sender for workflow_state events (#598).
    pub(super) broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// Working directory workflow template discovery resolves against
    /// (slice 2): `workflow.dir` and `./.quecto/workflows` are relative to it.
    pub(super) cwd: &'a std::path::Path,
    /// Home directory for the `~/.quecto/workflows` discovery fallback.
    pub(super) home_dir: Option<&'a std::path::Path>,
}

/// Resolve the model a CLI agent (or spawned child) runs on, honouring the
/// documented precedence: an explicit `--model` (which a spawned child receives
/// via spawn's forwarded `--model`, #881) wins over the `--config`-supplied
/// default. Centralised here so the precedence is unit-testable rather than
/// asserted only by a comment.
pub(crate) fn resolve_agent_model(model_override: Option<&str>, config_default: &str) -> String {
    match model_override {
        Some(m) => m.to_string(),
        None => config_default.to_string(),
    }
}

/// Build the tool registry for an agent session. Fails fast (per slice-1
/// conventions) when workflow template directory discovery hits a load error,
/// returning an error message naming the offending file — the caller aborts
/// startup rather than running with a partial template library.
pub(super) fn build_tool_registry(args: ToolRegistryArgs<'_>) -> Result<ToolRegistryBuild, String> {
    let ToolRegistryArgs {
        base_dir,
        config,
        http_client,
        flags,
        stderr,
        broadcast_tx,
        cwd,
        home_dir,
    } = args;
    let workspace = crate::interface::shared::resolve_agent_workspace(
        &config.workspace_path(),
        flags.no_sandbox,
    );
    let model = resolve_agent_model(
        flags.model_override.as_deref(),
        &config.agents.defaults.model,
    );
    let restrict_to_workspace = !flags.no_sandbox && config.agents.defaults.restrict_to_workspace;
    if flags.no_sandbox {
        stderr.push_str("WARNING: --no-sandbox is active — workspace path restriction disabled\n");
    }
    let sandbox = Sandbox::for_agent_workspace(config, workspace.clone(), flags.no_sandbox);
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(config);
    let exec_options = crate::infrastructure::tools::bash::ExecOptions {
        max_capture_bytes: exec_settings,
        ..crate::infrastructure::tools::bash::ExecOptions::default()
    };
    let mut registry = ToolRegistryImpl::with_core_tools_and_exec_options_spawned(
        workspace,
        sandbox,
        exec_options,
        flags.spawned,
    );
    let session_key = if flags.no_session || flags.session_name.as_deref() == Some("-") {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };
    let spill_store = Arc::new(FileContextSpillStore::new(base_dir.to_path_buf()));
    // Session + agent-control tools are supplied via the bundled native provider
    // seam (#1276 Phase 3). Registration still uses `register` (not
    // `register_extension`) so these tools stay non-unloadable official tools.
    register_bundled_native_tools(
        &mut registry,
        build_session_tool_extensions(SessionToolDeps {
            spill_store: spill_store.clone(),
            session_key: session_key.clone(),
        }),
    );
    let socket_dir = crate::interface::shared::xdg_runtime_dir_or_temp();
    let agent_control = build_agent_control_tool_extensions(AgentControlToolDeps {
        base_dir: base_dir.to_path_buf(),
        socket_dir,
        restrict_to_workspace,
        broadcast_tx: broadcast_tx.clone(),
        // Forward spawned children's workflow_state events onto this agent's
        // stream, tagged with the child id + this agent's id (PRD Stage B).
        parent_session_name: flags.session_name.clone(),
    });
    register_bundled_native_tools(&mut registry, agent_control.extensions);
    let notify_rx = agent_control.notification_rx;
    // notify_tx is retained by SpawnTool inside the provider-built extension.
    let _ = agent_control.notification_tx;
    let subagent_registry_for_protocol = agent_control.subagent_registry;
    // Build a workflow event emitter from the broadcast channel (#598).
    // Stamp emitted workflow_state events with this agent's identity (its
    // session name) and its parent (PRD Stage B), so consumers can rebuild the
    // unit tree from the stream.
    let emitter_agent_id = flags.session_name.clone();
    let emitter_parent_id = flags.parent_id.clone();
    let wf_emitter = broadcast_tx.map(|tx| {
        crate::infrastructure::tools::workflow_tool::broadcast_emitter(
            tx,
            emitter_agent_id,
            emitter_parent_id,
        )
    });
    // A by-value workflow spec (`--workflow-spec`) binds this agent to exactly
    // one template, started directly in Active mode (no model-driven selection),
    // overriding the config's template library for this run.
    let spec_requested = flags.workflow_spec_path.is_some();
    let bound_spec = flags
        .workflow_spec_path
        .as_ref()
        .and_then(|p| match load_workflow_spec(p) {
            Ok(spec) => Some(spec),
            Err(err) => {
                stderr.push_str(&format!(
                    "failed to load workflow spec '{}': {}\n",
                    p.display(),
                    err
                ));
                None
            }
        });
    // Fail closed: a spec was assigned but could not be loaded. The parent
    // validated it before writing, so this is corruption/IO loss — do NOT
    // silently degrade into a free-selection workflow agent.
    if spec_requested && bound_spec.is_none() {
        stderr.push_str(
            "workflow spec was assigned but could not be loaded; refusing to start a workflow\n",
        );
    }
    let workflow_available = flags.uds_mode
        && !(spec_requested && bound_spec.is_none())
        && (!flags.workflow_disabled || bound_spec.is_some());
    let wf_state = if workflow_available {
        // For a bound run, use a config containing ONLY the assigned template
        // (avoids cloning the whole default library just to discard it).
        // A bound spec bypasses directory discovery entirely (AC6): the
        // assigned template is the whole library, and no shadowing warning
        // can apply.
        let wf_config = match &bound_spec {
            Some(spec) => crate::domain::workflow::WorkflowConfig {
                auto_continue: config.workflow.auto_continue,
                completion_nudge: config.workflow.completion_nudge,
                selector_prompt: None,
                dir: None,
                templates: vec![spec.template.clone()],
            },
            None => {
                // Slice 2: resolve the session's template library from the
                // workflow directory precedence chain (workflow.dir →
                // ./.quecto/workflows → ~/.quecto/workflows → inline).
                let discovery = crate::infrastructure::config::discover_workflow_templates(
                    config, cwd, home_dir,
                )
                .map_err(|error| error.to_string())?;
                if let Some(warning) = &discovery.warning {
                    stderr.push_str(&format!("WARNING: {warning}\n"));
                }
                crate::domain::workflow::WorkflowConfig {
                    auto_continue: config.workflow.auto_continue,
                    completion_nudge: config.workflow.completion_nudge,
                    selector_prompt: config.workflow.selector_prompt.clone(),
                    // Already resolved; the engine consumes the template list.
                    dir: None,
                    templates: discovery.templates,
                }
            }
        };
        let state = crate::interface::shared::register_workflow_tool(
            &mut registry,
            wf_config,
            flags.workflow_guards,
            wf_emitter,
        )
        .map_err(|error| format!("failed to initialize workflow: {error}"))?;
        // Bind: pre-select the assigned template (Active mode) and lock the
        // engine so the model cannot reset or switch templates. Initialization
        // and binding errors abort startup; an explicitly requested workflow
        // must never degrade into a session without its workflow.
        if let Some(spec) = bound_spec {
            let mut engine = state.lock().map_err(|_| {
                "failed to bind workflow template: engine lock poisoned".to_string()
            })?;
            engine
                .select_template(&spec.template.id, None)
                .map_err(|error| {
                    format!(
                        "failed to bind workflow template '{}': {error}",
                        spec.template.id
                    )
                })?;
            engine.set_bound(true);
        }
        Some(state)
    } else {
        None
    };

    let ext_registry =
        crate::interface::shared::build_and_register_native_extensions(config, http_client);
    let extension_prompt_snippets = ext_registry.system_prompt_snippets();
    crate::interface::shared::register_extension_tools(&mut registry, &ext_registry);

    // #926: the parent is always spawn-capable — `SpawnTool` (with `notify_tx`)
    // and `AgentCmdTool` (with the protocol registry) are registered above
    // UNCONDITIONALLY. So the notification receiver and the protocol registry
    // must ALWAYS be live: a live `notify_tx` paired with a dropped `notify_rx`
    // means a child's completion is emitted into a channel with no receiver and
    // the idle parent is NEVER woken. Keep tx and rx wired together so a child
    // completing reliably wakes the parent in every config.
    Ok(ToolRegistryBuild {
        registry,
        spill_store,
        session_key,
        model,
        ext_registry,
        extension_prompt_snippets,
        notification_rx: Some(notify_rx),
        subagent_registry: Some(subagent_registry_for_protocol),
        workflow_state: wf_state,
    })
}

/// Load and parse a by-value workflow spec file (`--workflow-spec <path>`).
///
/// Bounds the file size before reading (defense-in-depth against a hostile or
/// corrupt spec) and removes the single-use file once read so it does not
/// linger beside the socket.
#[cfg(test)]
#[path = "agent_tool_registry_cov_tests.rs"]
mod cov_tests;

fn load_workflow_spec(
    path: &std::path::Path,
) -> Result<crate::domain::workflow::WorkflowSpec, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let max = crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES as u64;
    if len > max {
        return Err(format!("workflow spec too large: {len} bytes (max {max})"));
    }
    let raw = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    // Single-use: best-effort cleanup once consumed.
    let _ = std::fs::remove_file(path);
    serde_json::from_str::<crate::domain::workflow::WorkflowSpec>(&raw).map_err(|e| e.to_string())
}
