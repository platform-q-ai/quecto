use crate::domain::tool::ToolProfileContext;
use crate::domain::tool_descriptor::ProfileAvailabilityScope;

/// Entrypoint policy selector for the shared tool runtime/catalogue builder.
///
/// The value describes the supported composition root, not a separate tool
/// construction path. Entrypoints feed this policy into
/// [`build_tool_runtime`], which performs provider discovery/registration in one
/// place and then applies entrypoint defaults as catalogue policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolEntrypoint {
    /// One-shot CLI agent invocation (`quecto agent ...`).
    CliAgent,
    /// UDS-backed agent session (`quecto agent --mode uds ...`).
    UdsAgent,
    /// Interactive no-args REPL.
    Repl,
}

impl ToolEntrypoint {
    pub fn agent_control_default_enabled(self) -> bool {
        matches!(self, Self::CliAgent | Self::UdsAgent)
    }

    pub fn web_default_enabled(self) -> bool {
        matches!(self, Self::CliAgent | Self::UdsAgent)
    }

    pub fn workflow_supported(self) -> bool {
        matches!(self, Self::UdsAgent)
    }
}

/// Effective entrypoint defaults selected while building a tool runtime.
///
/// This is intentionally small for #1276 Phase 2: richer configured/profile /
/// persisted policy state is a later phase. The fields here make today's
/// user-visible CLI/UDS/REPL differences explicit instead of encoding them as
/// omitted provider construction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ToolRuntimePolicyState {
    pub entrypoint: ToolEntrypoint,
    pub agent_control_default_enabled: bool,
    pub web_default_enabled: bool,
    pub workflow_supported: bool,
    pub configured_enabled: Option<bool>,
    pub profile_enabled: Option<bool>,
    pub session_enabled: Option<bool>,
}

impl ToolRuntimePolicyState {
    fn for_entrypoint(entrypoint: ToolEntrypoint) -> Self {
        Self {
            entrypoint,
            agent_control_default_enabled: entrypoint.agent_control_default_enabled(),
            web_default_enabled: entrypoint.web_default_enabled(),
            workflow_supported: entrypoint.workflow_supported(),
            configured_enabled: None,
            profile_enabled: None,
            session_enabled: None,
        }
    }
}

/// Workflow-specific policy inputs for [`build_tool_runtime`].
pub(crate) struct ToolRuntimeWorkflowPolicy<'a> {
    pub workflow_disabled: bool,
    pub workflow_guards: bool,
    pub workflow_spec_path: Option<&'a std::path::Path>,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub emitter_agent_id: Option<String>,
    pub emitter_parent_id: Option<String>,
    pub cwd: &'a std::path::Path,
    pub home_dir: Option<&'a std::path::Path>,
}

impl<'a> ToolRuntimeWorkflowPolicy<'a> {
    pub fn disabled(cwd: &'a std::path::Path, home_dir: Option<&'a std::path::Path>) -> Self {
        Self {
            workflow_disabled: true,
            workflow_guards: false,
            workflow_spec_path: None,
            broadcast_tx: None,
            emitter_agent_id: None,
            emitter_parent_id: None,
            cwd,
            home_dir,
        }
    }
}

/// Runtime profile selected for model-visible tool policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ToolRuntimeProfileContext {
    Parent,
    Child,
}

impl ToolRuntimeProfileContext {
    pub(crate) fn from_spawned(spawned: bool) -> Self {
        if spawned { Self::Child } else { Self::Parent }
    }

    fn is_child(self) -> bool {
        matches!(self, Self::Child)
    }

    fn profile_context(self) -> ToolProfileContext {
        match self {
            Self::Parent => ToolProfileContext::Parent,
            Self::Child => ToolProfileContext::Child,
        }
    }
}

/// Inputs for the shared tool runtime/catalogue builder.
pub(crate) struct ToolRuntimeBuildArgs<'a> {
    pub entrypoint: ToolEntrypoint,
    pub profile_context: ToolRuntimeProfileContext,
    pub base_dir: &'a std::path::Path,
    pub config: &'a crate::infrastructure::config::Config,
    pub http_client: &'a reqwest::Client,
    pub workspace: std::path::PathBuf,
    pub sandbox: crate::infrastructure::security::sandbox::Sandbox,
    pub exec_options: crate::infrastructure::tools::bash::ExecOptions,
    pub session_key: String,
    pub spawned: bool,
    pub restrict_to_workspace: bool,
    pub parent_session_name: Option<String>,
    pub disabled_tools: &'a [String],
    pub workflow: ToolRuntimeWorkflowPolicy<'a>,
    pub stderr: &'a mut String,
}

/// Result of the shared tool runtime/catalogue builder.
pub(crate) struct ToolRuntimeBuild {
    pub registry: crate::infrastructure::tools::registry::ToolRegistryImpl,
    pub spill_store:
        std::sync::Arc<crate::infrastructure::persistence::context_spill::FileContextSpillStore>,
    pub session_key: String,
    pub ext_registry: crate::infrastructure::extensions::registry::ExtensionRegistry,
    pub extension_prompt_snippets: String,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    pub policy_state: ToolRuntimePolicyState,
    pub catalogue_entries: Vec<crate::domain::tool_descriptor::ToolCatalogueEntry>,
}

/// Build the complete shared tool runtime/catalogue for CLI, UDS and REPL.
///
/// All production entrypoints use this pipeline to register bundled-native
/// providers (official/core, recall/session, agent-control, workflow and
/// config-gated web). Entrypoint-specific differences are then applied as policy:
/// REPL keeps today's model-visible surface by default-disabling agent-control
/// and web tools after registration rather than silently omitting their
/// providers, while UDS remains the only entrypoint that supports the workflow
/// runtime.
pub(crate) fn build_tool_runtime(
    args: ToolRuntimeBuildArgs<'_>,
) -> Result<ToolRuntimeBuild, String> {
    use crate::infrastructure::extensions::native::{
        AgentControlToolDeps, OfficialToolDeps, SessionToolDeps,
        build_agent_control_tool_extensions, build_official_tool_extensions,
        build_session_tool_extensions, register_bundled_native_tools,
        register_bundled_native_tools_with_scope,
    };
    use crate::infrastructure::persistence::context_spill::FileContextSpillStore;

    let ToolRuntimeBuildArgs {
        entrypoint,
        profile_context,
        base_dir,
        config,
        http_client,
        workspace,
        sandbox,
        exec_options,
        session_key,
        spawned,
        restrict_to_workspace,
        parent_session_name,
        disabled_tools,
        workflow,
        stderr,
    } = args;

    let policy_state = ToolRuntimePolicyState::for_entrypoint(entrypoint);
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    register_bundled_native_tools_with_scope(
        &mut registry,
        build_official_tool_extensions(OfficialToolDeps {
            workspace,
            sandbox,
            exec_options,
            docs_content_policy: if profile_context.is_child() {
                crate::infrastructure::tools::docs::DocsContentPolicy::Child
            } else {
                crate::infrastructure::tools::docs::DocsContentPolicy::Parent
            },
        }),
        Some(match profile_context {
            ToolRuntimeProfileContext::Parent => ProfileAvailabilityScope::Parent,
            ToolRuntimeProfileContext::Child => ProfileAvailabilityScope::Child,
        }),
    );

    let spill_store = std::sync::Arc::new(FileContextSpillStore::new(base_dir.to_path_buf()));
    register_bundled_native_tools(
        &mut registry,
        build_session_tool_extensions(SessionToolDeps {
            spill_store: spill_store.clone(),
            session_key: session_key.clone(),
        }),
    );

    // Agent-control tools are supplied through the same bundled-native provider
    // for every entrypoint. REPL's current public surface is preserved below by
    // policy-disabling the tools after registration.
    let agent_control = build_agent_control_tool_extensions(AgentControlToolDeps {
        base_dir: base_dir.to_path_buf(),
        socket_dir: crate::interface::shared::xdg_runtime_dir_or_temp(),
        restrict_to_workspace,
        broadcast_tx: workflow.broadcast_tx.clone(),
        parent_session_name,
    });
    register_bundled_native_tools_with_scope(
        &mut registry,
        agent_control.extensions,
        Some(ProfileAvailabilityScope::Parent),
    );
    let notify_rx = agent_control.notification_rx;
    let _ = agent_control.notification_tx;
    let subagent_registry_for_protocol = agent_control.subagent_registry;
    if !policy_state.agent_control_default_enabled {
        registry.disable_tool_by_entrypoint_default("spawn");
        registry.disable_tool_by_entrypoint_default("agent_cmd");
    }

    let wf_state = build_workflow_runtime(&mut registry, entrypoint, config, workflow, stderr)?;

    let ext_registry =
        crate::interface::shared::build_and_register_native_extensions(config, http_client);
    let extension_prompt_snippets = ext_registry.system_prompt_snippets();
    crate::interface::shared::register_bundled_native_extension_tools(&mut registry, &ext_registry);
    if !policy_state.web_default_enabled {
        registry.disable_tool_by_entrypoint_default("web_search");
        registry.disable_tool_by_entrypoint_default("web_fetch");
    }

    // Apply explicit startup restrictions after every startup provider has had a
    // chance to register, so descriptors remain available while model-visible
    // definitions and execution follow policy.
    let warnings = if spawned {
        registry.apply_spawn_tool_restrictions(disabled_tools)
    } else {
        registry.apply_startup_tool_restrictions(disabled_tools)
    };
    for name in &warnings {
        stderr.push_str(&format!(
            "WARNING: --disable-tool: no tool named '{}' in the registry\n",
            name
        ));
    }

    registry.set_execution_profile_context(profile_context.profile_context());

    let catalogue_entries = registry.catalogue_entries();

    Ok(ToolRuntimeBuild {
        registry,
        spill_store,
        session_key,
        ext_registry,
        extension_prompt_snippets,
        notification_rx: Some(notify_rx),
        subagent_registry: Some(subagent_registry_for_protocol),
        workflow_state: wf_state,
        policy_state,
        catalogue_entries,
    })
}

fn build_workflow_runtime(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    entrypoint: ToolEntrypoint,
    config: &crate::infrastructure::config::Config,
    workflow: ToolRuntimeWorkflowPolicy<'_>,
    stderr: &mut String,
) -> Result<Option<crate::interface::shared::WorkflowStateHandle>, String> {
    if !entrypoint.workflow_supported() {
        return Ok(None);
    }

    let spec_requested = workflow.workflow_spec_path.is_some();
    let bound_spec = workflow
        .workflow_spec_path
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
    if spec_requested && bound_spec.is_none() {
        stderr.push_str(
            "workflow spec was assigned but could not be loaded; refusing to start a workflow\n",
        );
    }
    let workflow_available = !(spec_requested && bound_spec.is_none())
        && (!workflow.workflow_disabled || bound_spec.is_some());
    if !workflow_available {
        return Ok(None);
    }

    let wf_emitter = workflow.broadcast_tx.map(|tx| {
        crate::infrastructure::tools::workflow_tool::broadcast_emitter(
            tx,
            workflow.emitter_agent_id,
            workflow.emitter_parent_id,
        )
    });
    let wf_config = match &bound_spec {
        Some(spec) => crate::domain::workflow::WorkflowConfig {
            auto_continue: config.workflow.auto_continue,
            completion_nudge: config.workflow.completion_nudge,
            selector_prompt: None,
            dir: None,
            templates: vec![spec.template.clone()],
        },
        None => {
            let discovery = crate::infrastructure::config::discover_workflow_templates(
                config,
                workflow.cwd,
                workflow.home_dir,
            )
            .map_err(|error| error.to_string())?;
            if let Some(warning) = &discovery.warning {
                stderr.push_str(&format!("WARNING: {warning}\n"));
            }
            crate::domain::workflow::WorkflowConfig {
                auto_continue: config.workflow.auto_continue,
                completion_nudge: config.workflow.completion_nudge,
                selector_prompt: config.workflow.selector_prompt.clone(),
                dir: None,
                templates: discovery.templates,
            }
        }
    };
    let state = crate::interface::shared::register_workflow_tool(
        registry,
        wf_config,
        workflow.workflow_guards,
        wf_emitter,
    )
    .map_err(|error| format!("failed to initialize workflow: {error}"))?;
    if let Some(spec) = bound_spec {
        let mut engine = state
            .lock()
            .map_err(|_| "failed to bind workflow template: engine lock poisoned".to_string())?;
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
    Ok(Some(state))
}

#[cfg(test)]
#[path = "tool_runtime_catalogue_tests.rs"]
mod catalogue_tests;

#[cfg(test)]
#[path = "tool_runtime_profile_tests.rs"]
mod profile_tests;

pub(crate) fn load_workflow_spec(
    path: &std::path::Path,
) -> Result<crate::domain::workflow::WorkflowSpec, String> {
    let len = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
    let max = crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES as u64;
    if len > max {
        return Err(format!(
            "workflow spec too large: {len} bytes, exceeding the {max} byte limit"
        ));
    }
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(path);
    serde_json::from_slice(&bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "tool_runtime_cov_tests.rs"]
mod cov_tests;
