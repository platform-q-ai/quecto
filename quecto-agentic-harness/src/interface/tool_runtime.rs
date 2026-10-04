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
}

impl ToolEntrypoint {
    pub fn agent_control_default_enabled(self) -> bool {
        true
    }

    pub fn web_default_enabled(self) -> bool {
        true
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
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ToolRuntimePolicyState {
    pub entrypoint: ToolEntrypoint,
    pub agent_control_default_enabled: bool,
    pub web_default_enabled: bool,
    pub workflow_supported: bool,
    pub configured_enabled: Option<bool>,
    pub profile_enabled: Option<bool>,
    pub session_enabled: Option<bool>,
    pub inherited_tool_policy:
        Option<crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot>,
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
            inherited_tool_policy: None,
        }
    }
}

/// Workflow-specific policy inputs for [`build_tool_runtime`].
pub(crate) struct ToolRuntimeWorkflowPolicy<'a> {
    pub workflow_disabled: bool,
    /// The launch asked for workflow mode (`--workflow`); guards and a
    /// bound spec ask for it too (#2216).
    pub workflow_requested: bool,
    pub workflow_guards: bool,
    pub workflow_spec_path: Option<&'a std::path::Path>,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub emitter_agent_id: Option<String>,
    pub emitter_parent_id: Option<String>,
    pub cwd: &'a std::path::Path,
    pub home_dir: Option<&'a std::path::Path>,
}

impl<'a> ToolRuntimeWorkflowPolicy<'a> {
    #[cfg(test)]
    pub fn disabled(cwd: &'a std::path::Path, home_dir: Option<&'a std::path::Path>) -> Self {
        Self {
            workflow_disabled: true,
            workflow_requested: false,
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
    /// Explicit launch context; reusable runtime construction never discovers ambient membership.
    pub swarm_context: Option<crate::infrastructure::tools::swarm_bridge::SwarmContext>,
    /// The process's shared swarm participation (#1715), injected into the
    /// spawn, swarm and workflow tools.
    pub swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation,
    pub entrypoint: ToolEntrypoint,
    pub profile_context: ToolRuntimeProfileContext,
    pub base_dir: &'a std::path::Path,
    pub config: &'a crate::infrastructure::config::Config,
    pub http_client: &'a reqwest::Client,
    /// Completed web-fetch graph, assembled by the outer composition boundary.
    pub web_fetch_tool: Option<std::sync::Arc<dyn crate::application::tools::ports::Tool>>,
    pub workspace: std::path::PathBuf,
    pub sandbox: crate::infrastructure::security::sandbox::Sandbox,
    pub exec_options: crate::infrastructure::tools::bash::ExecOptions,
    pub session_key: String,
    /// The recall use case the `recall` tool adapts (D9 #1978), composed
    /// over the run's retention store.
    pub recall: std::sync::Arc<crate::application::sessions::use_cases::RecallContext>,
    pub spawned: bool,
    pub parent_session_name: Option<String>,
    /// The parent agent's own config path, forwarded so container spawns can
    /// fall back to it when the spawn call omits `config` (#1369 follow-up).
    pub parent_config_path: Option<std::path::PathBuf>,
    /// Whether this agent launches configured extensions (#2446).
    pub launch_extensions: bool,
    /// The change-reasoning-effort use case (#1848) the spawn tool validates
    /// an explicit-model `effort` against.
    pub effort_control:
        Option<std::sync::Arc<crate::application::catalogue::use_cases::ChangeReasoningEffort>>,
    /// Composition's container-config handles (#2024 S4a, S4c): the
    /// selection the spawn tool resolves `container: true` through and the
    /// discovery query it and agent_cmd list through; `None` refuses new
    /// containers and lists nothing.
    pub container_configs:
        Option<crate::interface::cli::container_config_handles::ContainerConfigHandles>,
    /// Composition's durable environment registry (#2024 S4d) the spawn
    /// tool commits to; `None` (unit rigs) builds an in-memory one.
    pub environment_registry: Option<crate::domain::environment_registry::EnvironmentRegistry>,
    /// Composition's builder of the `agent_cmd kill` owner (#1936); `None`
    /// leaves `kill` unavailable.
    pub kill_tool: Option<crate::interface::cli::KillToolBuilder>,
    pub disabled_tools: &'a [String],
    pub inherited_tool_policy:
        Option<crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot>,
    pub workflow: ToolRuntimeWorkflowPolicy<'a>,
    pub stderr: &'a mut String,
}

/// Result of the shared tool runtime/catalogue builder.
pub(crate) struct ToolRuntimeBuild {
    pub registry: crate::infrastructure::tools::registry::ToolRegistryImpl,
    pub session_key: String,
    pub ext_registry: crate::infrastructure::extensions::registry::ExtensionRegistry,
    pub extension_prompt_snippets: String,
    pub notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub subagent_registry:
        Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
    /// The harness lifecycle cell the spawn tool admits against (#1938).
    pub harness_lifecycle:
        Option<crate::infrastructure::tools::harness_lifecycle::SharedHarnessLifecycle>,
    /// The environment control slot (#2070) the dispatch loop hands the
    /// teardown, for the emptied `retained` environments an owner exit ends.
    pub environment_control:
        Option<crate::infrastructure::tools::agent_cmd_containers::EnvironmentControlSlot>,
    pub workflow_state: Option<crate::interface::shared::WorkflowStateHandle>,
    pub policy_state: ToolRuntimePolicyState,
    pub catalogue_entries: Vec<crate::domain::tool_descriptor::ToolCatalogueEntry>,
}

/// Report persisted `tools.policy` entries that matched no registered tool
/// (#2217): the application's one triage; only a typo — a restriction that
/// never applies — is a start-up warning.
fn report_unmatched_policy_entries(stable_ids: Vec<String>, stderr: &mut String) {
    use crate::application::tools::unmatched_policy::split_unmatched_policy_entries;
    use crate::domain::tool_policy_catalogue::unknown_policy_entry_warning;
    for stable_id in split_unmatched_policy_entries(stable_ids).unknown {
        stderr.push_str(&format!(
            "WARNING: {}\n",
            unknown_policy_entry_warning(&stable_id)
        ));
    }
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

    let ToolRuntimeBuildArgs {
        swarm_context,
        swarm_participation,
        entrypoint,
        profile_context,
        base_dir,
        config,
        http_client,
        web_fetch_tool,
        workspace,
        sandbox,
        exec_options,
        session_key,
        recall,
        spawned,
        parent_session_name,
        parent_config_path,
        launch_extensions,
        effort_control,
        container_configs,
        environment_registry,
        kill_tool,
        disabled_tools,
        inherited_tool_policy,
        workflow,
        stderr,
    } = args;

    // Entrypoints supply actual launch context; isolated runtime consumers
    // have no implicit enrollment side effects from their parent environment.
    // Workflow eligibility follows swarm participation, not containerization
    // (#1715): the join answers whether this container's run was created.
    let swarm_agent = match &swarm_context {
        Some(context) => crate::infrastructure::tools::swarm_lifecycle::join_current_process(
            context,
            crate::infrastructure::tools::swarm_bridge::process_socket(),
            swarm_participation.clone(),
            crate::infrastructure::tools::swarm_lifecycle::is_creator(),
        )
        .map_err(|e| e.to_string())?,
        None => false,
    };
    let workflow_engine: crate::infrastructure::tools::swarm_bridge::WorkflowEngineSlot =
        Default::default();
    crate::domain::swarm::validate_workflow(
        swarm_agent,
        workflow.workflow_guards || workflow.workflow_spec_path.is_some(),
    )
    .map_err(|e| e.to_string())?;

    // PR #1401 review: the parent may have been launched with a RELATIVE
    // `--config` (or hit the relative `.quecto` base-dir fallback). Container
    // spawns reuse this path as their trusted-config fallback, which demands
    // an absolute path — canonicalize once at the composition root so the
    // documented zero-config container spawn holds regardless of how the
    // parent was launched. An uncanonicalizable path is forwarded verbatim so
    // the spawn-time absolute-path error still fires with the real value.
    let parent_config_path = canonical_parent_config_path(parent_config_path);

    let mut policy_state = ToolRuntimePolicyState::for_entrypoint(entrypoint);
    policy_state.workflow_supported &= !swarm_agent;
    policy_state.inherited_tool_policy = inherited_tool_policy.clone();
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    // Fault injection for the #2192 process tests; not compiled in production.
    #[cfg(feature = "test-support")]
    crate::infrastructure::tools::panic_probe::register_if_requested(&mut registry);
    register_bundled_native_tools_with_scope(
        &mut registry,
        build_official_tool_extensions(OfficialToolDeps {
            find_tool: crate::composition::find::build_find_tool(
                std::sync::Arc::new(workspace.clone()),
                std::sync::Arc::new(sandbox.clone()),
            ),
            grep: Some(crate::infrastructure::search::GrepWiring {
                config: config.tools.grep.clone(),
                base_dir: base_dir.to_path_buf(),
                session_key: session_key.clone(),
            }),
            swarm_context: swarm_context.clone(),
            swarm_participation: swarm_participation.clone(),
            workflow_engine: workflow_engine.clone(),
            workspace,
            sandbox,
            exec_options,
            docs_content_policy: if profile_context.is_child() {
                crate::infrastructure::tools::docs::DocsContentPolicy::Child
            } else {
                crate::infrastructure::tools::docs::DocsContentPolicy::Parent
            },
        }),
        match profile_context {
            // A fresh top-level session has no user/profile policy yet. Leaving
            // the profile field absent serializes the registry default as Both,
            // so the TUI's first Ctrl+T render shows unrestricted tools as [PC].
            ToolRuntimeProfileContext::Parent => None,
            ToolRuntimeProfileContext::Child => Some(ProfileAvailabilityScope::Child),
        },
    );

    register_bundled_native_tools(
        &mut registry,
        build_session_tool_extensions(SessionToolDeps {
            recall,
            session_key: session_key.clone(),
        }),
    );

    // Agent-control tools are supplied through the same bundled-native provider
    // for every entrypoint. REPL's current public surface is preserved below by
    // policy-disabling the tools after registration.
    let agent_control = build_agent_control_tool_extensions(AgentControlToolDeps {
        swarm_participation: swarm_participation.clone(),
        swarm_context,
        base_dir: base_dir.to_path_buf(),
        socket_dir: crate::interface::shared::xdg_runtime_dir_or_temp(),
        broadcast_tx: workflow.broadcast_tx.clone(),
        parent_session_name,
        inherited_tool_policy: None,
        parent_config_path,
        launch_extensions,
        owned_child_supervisor:
            crate::infrastructure::processes::owned_child_supervisor::OwnedChildSupervisor::process_wide(),
        effort_control,
        container_config_selection: container_configs
            .as_ref()
            .map(|handles| handles.selection.clone()),
        container_config_roster: container_configs.map(|handles| handles.roster),
        environment_registry,
    });
    // The agent-control use cases — `kill`, the environment member
    // shutdown, the swarm member termination, the spawn lifecycle and the
    // environment control — are composed over the tools' own registry,
    // channels, lifecycle cell, environment registry and slots (#1936,
    // #1939).
    let environment_control = agent_control.termination_slots.environments.clone();
    if let Some(install_termination_owners) = kill_tool {
        let installed = install_termination_owners(crate::interface::cli::KillToolWiring {
            owner: crate::domain::ids::AgentUuid::new(if session_key.is_empty() {
                "harness".to_string()
            } else {
                session_key.clone()
            }),
            registry: agent_control.subagent_registry.clone(),
            broadcast_tx: workflow.broadcast_tx.clone(),
            notify_tx: Some(agent_control.notification_tx.clone()),
            harness_lifecycle: agent_control.harness_lifecycle.clone(),
            environment_registry: agent_control.environment_registry.clone(),
            slots: agent_control.termination_slots.clone(),
            base_dir: base_dir.to_path_buf(),
        });
        debug_assert!(
            installed,
            "the termination owners are composed once per runtime"
        );
    }
    register_bundled_native_tools_with_scope(&mut registry, agent_control.extensions, None);
    let notify_rx = agent_control.notification_rx;
    let _ = agent_control.notification_tx;
    let subagent_registry_for_protocol = agent_control.subagent_registry;
    let harness_lifecycle = agent_control.harness_lifecycle;
    if !policy_state.agent_control_default_enabled {
        registry.disable_tool_by_entrypoint_default("spawn");
        registry.disable_tool_by_entrypoint_default("agent_cmd");
    }

    let workflow_requested = workflow.workflow_requested
        || workflow.workflow_guards
        || workflow.workflow_spec_path.is_some();
    let WorkflowRuntime {
        engine: wf_state,
        spec_error,
    } = build_workflow_runtime(
        &mut registry,
        entrypoint,
        config,
        workflow,
        stderr,
        swarm_participation.clone(),
    )?;
    // #2216: a spawned child explicitly asked for workflow whose runtime
    // builds workflow (not a swarm member) expects an engine; when it could
    // not be built (its spec failed to load) the child refuses to start with
    // the reason, so its parent's spawn fails instead of reporting success.
    // A top-level agent keeps its fail-closed start without an engine.
    let engine_expected = spawned && workflow_requested && policy_state.workflow_supported;
    let engine_missing = wf_state.is_none();
    if engine_expected && engine_missing {
        return Err(format!(
            "workflow mode was requested, but the workflow engine could not be built: {}; refusing to start",
            spec_error.as_deref().unwrap_or("no workflow runtime")
        ));
    }
    // #2216: a runtime whose entrypoint builds workflow, or a swarm member,
    // withholds it when it did not build it (`--no-workflow`, a swarm run, a
    // spec that failed to load): its children stay closed to workflow. Only
    // an entrypoint that never builds it (the one-shot CLI) leaves it to its
    // children's own policy.
    if wf_state.is_none() && (entrypoint.workflow_supported() || swarm_agent) {
        registry.withhold_entrypoint_only_tool(
            crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME,
        );
    }
    // A swarm cannot be created while this composition's workflow is engaged,
    // and once this process is a swarm agent the selector nudge stops.
    if let Some(engine) = &wf_state {
        let _ = workflow_engine.set(engine.clone());
        let engine = engine.clone();
        swarm_participation.on_participation(move || {
            crate::domain::workflow::lock_engine(&engine).set_selector_nudge(false);
        });
    }

    let ext_registry = crate::interface::shared::build_and_register_native_extensions(
        config,
        http_client,
        web_fetch_tool,
    );
    let extension_prompt_snippets = ext_registry.system_prompt_snippets();
    crate::interface::shared::register_bundled_native_extension_tools(&mut registry, &ext_registry);
    if !policy_state.web_default_enabled {
        registry.disable_tool_by_entrypoint_default("web_search");
        registry.disable_tool_by_entrypoint_default("web_fetch");
    }

    if let Some(snapshot) = inherited_tool_policy.as_ref() {
        let warnings = registry.apply_inherited_tool_policy_snapshot(snapshot);
        for name in &warnings {
            stderr.push_str(&format!(
                "WARNING: inherited tool policy: no tool named '{}' in the registry\n",
                name
            ));
        }
    }

    let persisted_unknown = registry.apply_persisted_tool_policy(&config.tools.policy);
    report_unmatched_policy_entries(persisted_unknown, stderr);

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
    registry.refresh_spawn_inherited_child_policy_snapshot();
    require_requested_workflow_tool(
        &registry,
        profile_context.profile_context(),
        spawned && workflow_requested && wf_state.is_some(),
    )?;

    let catalogue_entries = registry.catalogue_entries();

    Ok(ToolRuntimeBuild {
        registry,
        session_key,
        ext_registry,
        extension_prompt_snippets,
        notification_rx: Some(notify_rx),
        subagent_registry: Some(subagent_registry_for_protocol),
        harness_lifecycle: Some(harness_lifecycle),
        environment_control: Some(environment_control),
        workflow_state: wf_state,
        policy_state,
        catalogue_entries,
    })
}

/// #2216: a spawned child launched in workflow mode refuses to start when its
/// tool policy hides the workflow tool from its model. Its engine would
/// otherwise nudge the model toward a tool it cannot call, and its parent
/// would see success; a refusal before readiness reaches the parent's
/// `spawn` as the child's stderr instead. A top-level agent's user sees and
/// owns its own policy, so its launch is left alone.
fn require_requested_workflow_tool(
    registry: &crate::infrastructure::tools::registry::ToolRegistryImpl,
    context: ToolProfileContext,
    workflow_engaged: bool,
) -> Result<(), String> {
    if workflow_engaged {
        let visible = registry.definitions_for(context).iter().any(|definition| {
            definition.name.as_ref()
                == crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME
        });
        return if visible {
            Ok(())
        } else {
            Err("workflow mode was requested, but this agent's tool policy denies the workflow tool; refusing to start".into())
        };
    }
    Ok(())
}

/// What [`build_workflow_runtime`] built: the engine, if any, and why a
/// requested bound spec could not be loaded.
#[derive(Debug)]
struct WorkflowRuntime {
    engine: Option<crate::interface::shared::WorkflowStateHandle>,
    spec_error: Option<String>,
}

impl WorkflowRuntime {
    fn none() -> Self {
        Self {
            engine: None,
            spec_error: None,
        }
    }
}

fn build_workflow_runtime(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    entrypoint: ToolEntrypoint,
    config: &crate::infrastructure::config::Config,
    workflow: ToolRuntimeWorkflowPolicy<'_>,
    stderr: &mut String,
    swarm_participation: crate::infrastructure::tools::swarm_bridge::Participation,
) -> Result<WorkflowRuntime, String> {
    let swarm_agent = swarm_participation.participating();
    crate::domain::swarm::validate_workflow(
        swarm_agent,
        workflow.workflow_guards || workflow.workflow_spec_path.is_some(),
    )
    .map_err(|e| e.to_string())?;
    if swarm_agent {
        return Ok(WorkflowRuntime::none());
    }
    if !entrypoint.workflow_supported() {
        return Ok(WorkflowRuntime::none());
    }

    let spec_requested = workflow.workflow_spec_path.is_some();
    let mut spec_error = None;
    let bound_spec = workflow
        .workflow_spec_path
        .and_then(|p| match load_workflow_spec(p) {
            Ok(spec) => Some(spec),
            Err(err) => {
                let error = format!("failed to load workflow spec '{}': {}", p.display(), err);
                stderr.push_str(&format!("{error}\n"));
                spec_error = Some(error);
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
        return Ok(WorkflowRuntime {
            engine: None,
            spec_error,
        });
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
    let state = crate::interface::shared::register_workflow_tool_with_participation(
        registry,
        wf_config,
        workflow.workflow_guards,
        wf_emitter,
        swarm_participation,
    )
    .map_err(|error| format!("failed to initialize workflow: {error}"))?;
    debug_assert!(
        registry.registers_entrypoint_only(
            crate::infrastructure::tools::workflow_tool::WORKFLOW_TOOL_NAME
        ),
        "the workflow tool must register as the allowlisted entrypoint-only tool"
    );
    if let Some(spec) = bound_spec {
        let mut engine = crate::domain::workflow::lock_engine(&state);
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
    Ok(WorkflowRuntime {
        engine: Some(state),
        spec_error: None,
    })
}

#[cfg(test)]
#[path = "tool_runtime_catalogue_tests.rs"]
mod catalogue_tests;

#[cfg(test)]
#[path = "tool_runtime_profile_tests.rs"]
mod profile_tests;

#[cfg(test)]
#[path = "tool_runtime_swarm_board_tests.rs"]
mod swarm_board_tests;

#[cfg(test)]
#[path = "tool_runtime_inherited_workflow_tests.rs"]
mod inherited_workflow_tests;

#[cfg(test)]
#[path = "tool_runtime_workflow_denial_tests.rs"]
mod workflow_denial_tests;

#[cfg(test)]
#[path = "tool_runtime_workflow_engine_tests.rs"]
mod workflow_engine_tests;

/// Canonicalize the parent's own config path before it is plumbed into the
/// tool runtime (PR #1401 review): container spawns fall back to this path
/// and require it to be absolute, but the parent may have been started with a
/// relative `--config`. A path that cannot be canonicalized (e.g. it no
/// longer exists) is kept verbatim so downstream errors name the real value.
fn canonical_parent_config_path(path: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    path.map(|p| std::fs::canonicalize(&p).unwrap_or(p))
}

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

/// This process's swarm context: its container contract, over the board
/// admission bound for it (#2278). `None` outside a swarm container, and
/// before (or without) a bound board.
pub(crate) fn swarm_context() -> Option<crate::infrastructure::tools::swarm_bridge::SwarmContext> {
    let board = crate::infrastructure::tools::swarm_bridge::process_board()?;
    crate::infrastructure::tools::swarm_bridge::SwarmContext::discover(
        std::sync::Arc::new(crate::application::swarm::LifecycleService),
        board.clone(),
    )
}
