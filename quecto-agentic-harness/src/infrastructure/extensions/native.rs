// Native extension: wraps a compiled-in Tool as an Extension.
//
// Native extensions are pure Rust implementations registered conditionally
// based on config. They have zero overhead when disabled.

use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::extension::Extension;
use crate::domain::tool::Tool;

/// A compiled-in extension that wraps a `Tool` implementation.
///
/// Native extensions are registered conditionally at startup based on config
/// (e.g., `tools.web.brave.enabled`). They:
/// - Execute in-process (no subprocess, no external runtime)
/// - Share the process's `reqwest::Client` and other resources
pub struct NativeExtension {
    name: String,
    description: String,
    tools: Vec<Arc<dyn Tool>>,
    system_prompt: Option<String>,
}

impl std::fmt::Debug for NativeExtension {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("NativeExtension")
            .field("name", &self.name)
            .field("description", &self.description)
            .field("tool_count", &self.tools.len())
            .finish()
    }
}

impl NativeExtension {
    /// Create a new native extension wrapping a single tool.
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        tool: Arc<dyn Tool>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tools: vec![tool],
            system_prompt: None,
        }
    }

    /// Create a native extension wrapping multiple tools.
    pub fn with_tools(
        name: impl Into<String>,
        description: impl Into<String>,
        tools: Vec<Arc<dyn Tool>>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            tools,
            system_prompt: None,
        }
    }

    /// Set an optional system prompt snippet.
    pub fn with_system_prompt(mut self, prompt: impl Into<String>) -> Self {
        self.system_prompt = Some(prompt.into());
        self
    }
}

impl Extension for NativeExtension {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.description
    }

    fn tools(&self) -> Vec<Arc<dyn Tool>> {
        self.tools.clone()
    }

    fn system_prompt_snippet(&self) -> Option<String> {
        self.system_prompt.clone()
    }
}

/// Build the bundled native extensions that provide Quecto's official tools.
///
/// This is the native-provider seam for official/default capabilities: callers
/// consume `Extension` objects and register their tools through the same
/// descriptor/policy registry path used by runtime UDS tools.
pub struct OfficialToolDeps {
    pub workspace: PathBuf,
    pub sandbox: crate::infrastructure::security::sandbox::Sandbox,
    pub exec_options: crate::infrastructure::tools::bash::ExecOptions,
    pub docs_content_policy: crate::infrastructure::tools::docs::DocsContentPolicy,
    pub python_lab_config: crate::infrastructure::tools::python_lab::PythonLabConfig,
}

pub fn build_official_tool_extensions(deps: OfficialToolDeps) -> Vec<Arc<dyn Extension>> {
    let sandbox = Arc::new(deps.sandbox);
    let workspace = Arc::new(deps.workspace);

    vec![Arc::new(NativeExtension::with_tools(
        "quecto:official-tools",
        "Bundled Quecto tool capabilities",
        vec![
            Arc::new(crate::infrastructure::tools::bash::ExecTool::with_options(
                workspace.clone(),
                sandbox.clone(),
                deps.exec_options,
            )),
            Arc::new(crate::infrastructure::tools::filesystem::ReadTool::new(
                workspace.clone(),
                sandbox.clone(),
            )),
            Arc::new(crate::infrastructure::tools::filesystem::WriteTool::new(
                workspace.clone(),
                sandbox.clone(),
            )),
            Arc::new(crate::infrastructure::tools::filesystem::EditTool::new(
                workspace.clone(),
                sandbox.clone(),
            )),
            Arc::new(crate::infrastructure::tools::filesystem::LsTool::new(
                workspace.clone(),
                sandbox.clone(),
            )),
            Arc::new(crate::infrastructure::tools::grep::GrepTool::new(
                workspace.clone(),
                sandbox.clone(),
            )),
            Arc::new(
                crate::infrastructure::tools::python_lab::PythonLabTool::new(
                    workspace.clone(),
                    sandbox.clone(),
                    deps.python_lab_config,
                ),
            ),
            Arc::new(crate::infrastructure::tools::find::FindTool::new(
                workspace, sandbox,
            )),
            // Quecto operating manual, embedded in the binary. Runtime profile
            // policy owns availability; the docs tool only receives explicit
            // content policy for parent-only quick-start filtering (#1319/#1334).
            Arc::new(
                crate::infrastructure::tools::docs::DocsTool::with_content_policy(
                    deps.docs_content_policy,
                ),
            ),
        ],
    ))]
}

pub struct SessionToolDeps {
    pub spill_store: Arc<dyn crate::domain::session::ContextSpillStore>,
    pub session_key: String,
}

pub fn build_session_tool_extensions(deps: SessionToolDeps) -> Vec<Arc<dyn Extension>> {
    vec![Arc::new(NativeExtension::new(
        "quecto:session-tools",
        "Bundled Quecto session memory tools",
        Arc::new(crate::infrastructure::tools::recall::RecallTool::new(
            deps.spill_store,
            deps.session_key,
        )),
    ))]
}

pub struct AgentControlToolDeps {
    pub base_dir: PathBuf,
    pub socket_dir: PathBuf,
    pub restrict_to_workspace: bool,
    pub broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    pub parent_session_name: Option<String>,
    pub inherited_tool_policy:
        Option<crate::infrastructure::tools::inherited_tool_policy::InheritedToolPolicySnapshot>,
    /// The parent agent's own config path (#1369 follow-up): container spawns
    /// without an explicit `config` argument fall back to it.
    pub parent_config_path: Option<PathBuf>,
}

pub struct AgentControlToolBuild {
    pub extensions: Vec<Arc<dyn Extension>>,
    pub subagent_registry: crate::infrastructure::tools::subagent_registry::SubagentRegistry,
    pub notification_tx: crate::infrastructure::tools::subagent_registry::NotificationTx,
    pub notification_rx: crate::infrastructure::tools::subagent_registry::NotificationRx,
}

pub fn build_agent_control_tool_extensions(deps: AgentControlToolDeps) -> AgentControlToolBuild {
    let registry = crate::infrastructure::tools::agent_cmd::AgentCmdTool::new_registry();
    let (notification_tx, notification_rx) = tokio::sync::mpsc::channel(64);
    let environment_registry = crate::domain::environment_registry::EnvironmentRegistry::new();

    let mut spawn = crate::infrastructure::tools::spawn::SpawnTool::with_base_dir(
        Vec::new(),
        deps.restrict_to_workspace,
        deps.base_dir,
    )
    .with_socket_dir(deps.socket_dir)
    .with_environment_registry(environment_registry.clone())
    .with_parent_config_path(deps.parent_config_path);
    if let Some(snapshot) = deps.inherited_tool_policy {
        spawn = spawn.with_inherited_tool_policy(snapshot);
    }
    let spawn = spawn
        .with_registry(registry.clone())
        .with_notify_tx(notification_tx.clone())
        .with_event_forwarding(deps.broadcast_tx.clone(), deps.parent_session_name);
    let environment_control = std::sync::Arc::new(
        crate::environment_control_app::EnvironmentControlUseCase::new(
            environment_registry,
            std::sync::Arc::new(
                crate::infrastructure::tools::environment_kill::ScriptEnvironmentKill::new(
                    registry.clone(),
                    deps.broadcast_tx.clone(),
                ),
            ),
        ),
    );
    let agent_cmd = crate::infrastructure::tools::agent_cmd::AgentCmdTool::new(registry.clone())
        .with_broadcast(deps.broadcast_tx)
        .with_environment_control(environment_control);

    AgentControlToolBuild {
        extensions: vec![Arc::new(NativeExtension::with_tools(
            "quecto:agent-control",
            "Bundled Quecto subagent control tools",
            vec![Arc::new(spawn), Arc::new(agent_cmd)],
        ))],
        subagent_registry: registry,
        notification_tx,
        notification_rx,
    }
}

pub struct WorkflowToolDeps {
    pub engine: crate::infrastructure::tools::workflow_tool::WorkflowEngineHandle,
    pub event_emitter: Option<crate::infrastructure::tools::workflow_tool::WorkflowEventEmitter>,
}

pub fn build_workflow_tool_extension(deps: WorkflowToolDeps) -> Arc<dyn Extension> {
    let tool: Arc<dyn Tool> = match deps.event_emitter {
        Some(emitter) => Arc::new(
            crate::infrastructure::tools::workflow_tool::WorkflowTool::with_event_emitter(
                deps.engine,
                emitter,
            ),
        ),
        None => {
            Arc::new(crate::infrastructure::tools::workflow_tool::WorkflowTool::new(deps.engine))
        }
    };
    Arc::new(NativeExtension::new(
        "quecto:workflow",
        "Bundled Quecto workflow tool",
        tool,
    ))
}

pub fn register_bundled_native_tools(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    extensions: Vec<Arc<dyn Extension>>,
) {
    register_bundled_native_tools_with_scope(registry, extensions, None);
}

pub fn register_bundled_native_tools_with_scope(
    registry: &mut crate::infrastructure::tools::registry::ToolRegistryImpl,
    extensions: Vec<Arc<dyn Extension>>,
    profile_scope: Option<crate::domain::tool_descriptor::ProfileAvailabilityScope>,
) {
    for extension in extensions {
        let provider_id = extension.name().to_string();
        for tool in extension.tools() {
            let mut metadata =
                crate::infrastructure::tools::registry::ToolRegistration::official_native()
                    .with_provider_id(provider_id.clone());
            metadata.profile_scope = profile_scope;
            metadata.profile_enabled = profile_scope.map(|scope| scope.is_enabled());
            registry.register_with_metadata(tool, metadata);
        }
    }
}

pub fn build_official_tool_registry(
    workspace: PathBuf,
    sandbox: crate::infrastructure::security::sandbox::Sandbox,
    exec_options: crate::infrastructure::tools::bash::ExecOptions,
) -> crate::infrastructure::tools::registry::ToolRegistryImpl {
    let mut registry = crate::infrastructure::tools::registry::ToolRegistryImpl::new();
    register_bundled_native_tools(
        &mut registry,
        build_official_tool_extensions(OfficialToolDeps {
            workspace,
            sandbox,
            exec_options,
            docs_content_policy: crate::infrastructure::tools::docs::DocsContentPolicy::Parent,
            python_lab_config: crate::infrastructure::tools::python_lab::PythonLabConfig::default(),
        }),
    );
    registry
}

/// Build native extensions from web tool config.
///
/// Builds a single `"web"` extension containing whichever web tools are
/// enabled in config:
/// - `web_search` — registered when `brave.enabled` or `duckduckgo.enabled`
/// - `web_fetch` — registered when `fetch.enabled`
///
/// Returns bundled native extension providers. Caller is responsible for
/// preserving prompt snippets in `ExtensionRegistry` if needed and registering
/// their tools through the bundled-native registry path, not the UDS/runtime
/// lifecycle path.
pub fn build_native_extensions(
    web_config: &crate::infrastructure::config::WebToolConfig,
    http_client: &reqwest::Client,
) -> Vec<Arc<dyn Extension>> {
    let mut web_tools: Vec<Arc<dyn Tool>> = Vec::new();

    // Web search: Brave or DuckDuckGo
    if web_config.brave.enabled || web_config.duckduckgo.enabled {
        let api_key = if web_config.brave.enabled && !web_config.brave.api_key.is_empty() {
            Some(web_config.brave.api_key.clone())
        } else {
            None
        };
        web_tools.push(Arc::new(
            crate::infrastructure::tools::web_search::WebSearchTool::with_client(
                api_key,
                http_client.clone(),
            ),
        ));
    }

    // Web fetch
    if web_config.fetch.enabled {
        web_tools.push(Arc::new(
            crate::infrastructure::tools::web_fetch::WebFetchTool::with_client(
                http_client.clone(),
                web_config.fetch.max_response_kb,
            ),
        ));
    }

    let mut extensions: Vec<Arc<dyn Extension>> = Vec::new();
    if !web_tools.is_empty() {
        extensions.push(Arc::new(NativeExtension::with_tools(
            "web",
            "Web search and fetch",
            web_tools,
        )));
    }

    extensions
}

#[cfg(test)]
#[path = "native_cov_tests.rs"]
mod cov_tests;

#[cfg(test)]
#[path = "native_tests.rs"]
mod tests;
