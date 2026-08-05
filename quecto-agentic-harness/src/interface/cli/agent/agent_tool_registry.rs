use super::*;
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
    pub(super) container_registry:
        Option<crate::infrastructure::tools::container_registry::ContainerRegistry>,
    pub(super) workflow_state: Option<crate::interface::shared::WorkflowStateHandle>, // #562
    pub(super) workspace: std::path::PathBuf,
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
    let session_key = if flags.no_session || flags.session_name.as_deref() == Some("-") {
        String::new()
    } else {
        let name = flags.session_name.as_deref().unwrap_or("default");
        Session::build_key("cli", name)
    };
    let entrypoint = if flags.uds_mode {
        crate::interface::shared::ToolEntrypoint::UdsAgent
    } else {
        crate::interface::shared::ToolEntrypoint::CliAgent
    };
    let runtime = crate::interface::shared::build_tool_runtime(
        crate::interface::shared::ToolRuntimeBuildArgs {
            entrypoint,
            profile_context:
                crate::interface::tool_runtime::ToolRuntimeProfileContext::from_spawned(
                    flags.spawned,
                ),
            base_dir,
            config,
            http_client,
            workspace: workspace.clone(),
            sandbox,
            exec_options,
            session_key,
            spawned: flags.spawned,
            restrict_to_workspace,
            parent_session_name: flags.session_name.clone(),
            disabled_tools: &flags.disabled_tools,
            inherited_tool_policy: flags.inherited_tool_policy.clone(),
            workflow: crate::interface::shared::ToolRuntimeWorkflowPolicy {
                workflow_disabled: flags.workflow_disabled,
                workflow_guards: flags.workflow_guards,
                workflow_spec_path: flags.workflow_spec_path.as_deref(),
                broadcast_tx,
                emitter_agent_id: flags.session_name.clone(),
                emitter_parent_id: flags.parent_id.clone(),
                cwd,
                home_dir,
            },
            stderr,
        },
    )?;

    let _policy_state = runtime.policy_state;
    let _catalogue_entries = runtime.catalogue_entries;

    // #926: the parent is always spawn-capable — `SpawnTool` (with `notify_tx`)
    // and `AgentCmdTool` (with the protocol registry) are constructed by the
    // shared runtime builder for CLI/UDS. A live `notify_tx` paired with a
    // dropped `notify_rx` means a child's completion is emitted into a channel
    // with no receiver and the idle parent is NEVER woken, so keep the returned
    // receiver/registry wired into the protocol runtime.
    Ok(ToolRegistryBuild {
        registry: runtime.registry,
        spill_store: runtime.spill_store,
        session_key: runtime.session_key,
        model,
        ext_registry: runtime.ext_registry,
        extension_prompt_snippets: runtime.extension_prompt_snippets,
        notification_rx: runtime.notification_rx,
        subagent_registry: runtime.subagent_registry,
        container_registry: runtime.container_registry,
        workflow_state: runtime.workflow_state,
        workspace,
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
