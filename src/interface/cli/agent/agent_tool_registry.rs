use super::*;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::agent_cmd::AgentCmdTool;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;
use crate::infrastructure::tools::spawn::SpawnTool;

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
}

pub(super) fn build_tool_registry(args: ToolRegistryArgs<'_>) -> ToolRegistryBuild {
    let ToolRegistryArgs {
        base_dir,
        config,
        http_client,
        flags,
        stderr,
        broadcast_tx,
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
    let sandbox = Sandbox::for_agent_workspace(config, workspace.clone(), flags.no_sandbox);
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(config);
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
            .with_socket_dir(socket_dir)
            .with_registry(subagent_registry.clone())
            .with_notify_tx(notify_tx)
            // Forward spawned children's workflow_state events onto this agent's
            // stream, tagged with the child id + this agent's id (PRD Stage B).
            .with_event_forwarding(broadcast_tx.clone(), flags.session_name.clone()),
    ));
    let subagent_registry_for_protocol = subagent_registry.clone();
    registry.register(Arc::new(AgentCmdTool::new(subagent_registry)));
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
        let wf_config = match &bound_spec {
            Some(spec) => crate::domain::workflow::WorkflowConfig {
                auto_continue: config.workflow.auto_continue,
                completion_nudge: config.workflow.completion_nudge,
                selector_prompt: None,
                templates: vec![spec.template.clone()],
            },
            None => config.workflow.clone(),
        };
        match crate::interface::shared::register_workflow_tool(
            &mut registry,
            wf_config,
            flags.workflow_guards,
            wf_emitter,
        ) {
            Ok(state) => {
                // Bind: pre-select the assigned template (Active mode) and lock
                // the engine so the model cannot reset or switch templates.
                if let Some(spec) = bound_spec {
                    match state.lock() {
                        Ok(mut engine) => match engine.select_template(&spec.template.id, None) {
                            Ok(()) => engine.set_bound(true),
                            Err(err) => stderr.push_str(&format!(
                                "failed to bind workflow template '{}': {}\n",
                                spec.template.id, err
                            )),
                        },
                        Err(_) => {
                            stderr.push_str(
                                "failed to bind workflow template: engine lock poisoned\n",
                            );
                        }
                    }
                }
                Some(state)
            }
            Err(err) => {
                stderr.push_str(&format!("failed to initialize workflow: {}\n", err));
                None
            }
        }
    } else {
        None
    };

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
    }
}

/// Load and parse a by-value workflow spec file (`--workflow-spec <path>`).
///
/// Bounds the file size before reading (defense-in-depth against a hostile or
/// corrupt spec) and removes the single-use file once read so it does not
/// linger beside the socket.
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
