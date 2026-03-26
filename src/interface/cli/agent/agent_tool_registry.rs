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
    pub(super) notification_rx: Option<crate::infrastructure::tools::subagent_registry::NotificationRx>,
    pub(super) subagent_registry: Option<crate::infrastructure::tools::subagent_registry::SubagentRegistry>,
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
    // Build a workflow event emitter from the broadcast channel (#598).
    let wf_emitter: Option<crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter> =
        broadcast_tx.as_ref().map(|tx| {
            let tx = tx.clone();
            Arc::new(move |event: serde_json::Value| {
                let mut line = serde_json::to_string(&event).unwrap_or_default();
                line.push('\n');
                let _ = tx.send(line);
            })
                as crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter
        });
    let wf_state = if flags.workflow {
        match crate::interface::shared::register_workflow_tool(
            &mut registry,
            config.workflow.clone(),
            flags.workflow_guards,
            wf_emitter,
        ) {
            Ok(state) => Some(state),
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

