#[cfg(test)]
use super::spawn_entry::{child_session_key, child_sidecar_filename};
#[cfg(test)]
use super::spawn_entry::{effective_config_path, inherited_runtime_config_path};
use super::spawn_input::parse_container_selection;
#[cfg(test)]
use super::spawn_launch_args::write_private_new;
pub use super::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::domain::environment_registry::EnvironmentRegistry;
use crate::domain::error::DomainError;
use crate::domain::ids::AgentUuid;
use crate::domain::subagent::{
    DisplayNameResolutionEntry, DisplayNameResolveError, SubagentConfig,
    assert_display_name_available_for_spawn, validate_agent_id,
};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use crate::domain::tool_descriptor::ProfileAvailabilityScope;
use crate::subagent_launch_app::SubagentLaunchUseCase;
use std::collections::{BTreeMap, HashMap};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::spawn_entry::{InitialRegistryEntrySpec, child_socket_path, initial_registry_entry};
pub use super::spawn_registry::{register_and_broadcast, shutdown_all, shutdown_all_with_count};
use super::subagent_registry::NotificationTx;
#[cfg(test)]
pub use super::subagent_registry::SubagentStatus;

const EMPTY_ROSTER: &str = "none configured";

/// Session-start roster line for the tool description (#1410): the available
/// container configs from the parent's effective config, with the default
/// marked. Composition-time IO — called once when the tool is built, never
/// from `definition()`. Deliberately uses the same loader (`Config::load`) as
/// the spawn-time `load_container_config`, so the roster and spawn selection
/// cannot diverge on loader behavior.
fn container_config_roster(parent_config_path: Option<&Path>) -> String {
    let Some(path) = parent_config_path else {
        return EMPTY_ROSTER.to_string();
    };
    let Ok(cfg) = crate::infrastructure::config::Config::load(&path.to_string_lossy()) else {
        return "unavailable (config failed to load)".to_string();
    };
    let names = super::spawn_container::container_config_names(&cfg);
    if names.is_empty() {
        return EMPTY_ROSTER.to_string();
    }
    names
        .iter()
        .map(|name| {
            if cfg.container_configs[name].default {
                format!("{name} (default)")
            } else {
                name.clone()
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// non-string entry is an LLM-addressable error, not a silent skip.
fn parse_disable_tools(args: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut tools: Vec<String> = Vec::new();
    let push_unique = |name: &str, tools: &mut Vec<String>| {
        if !tools.iter().any(|t| t == name) {
            tools.push(name.to_string());
        }
    };

    if let Some(v) = args.get("read_only").filter(|v| !v.is_null()) {
        if v.as_bool().ok_or("read_only must be a boolean")? {
            push_unique("write", &mut tools);
            push_unique("edit", &mut tools);
        }
    }
    if let Some(v) = args.get("disable_tools").filter(|v| !v.is_null()) {
        let arr = v
            .as_array()
            .ok_or("disable_tools must be an array of tool names")?;
        for entry in arr {
            let name = entry
                .as_str()
                .ok_or("disable_tools entries must be strings (tool names)")?;
            push_unique(name, &mut tools);
        }
    }
    Ok(tools)
}

fn validate_config_path(s: &str) -> Result<PathBuf, String> {
    let p = PathBuf::from(s);
    for component in p.components() {
        if component == std::path::Component::ParentDir {
            return Err(format!(
                "config path '{}' contains '..' which is not allowed",
                s
            ));
        }
    }
    Ok(p)
}

/// Tool that spawns a child `quecto agent` process in UDS mode.
///
/// When executed, validates the request, launches the child as a
#[derive(Debug)]
pub struct SpawnTool {
    /// Allowlist of agent IDs that can be spawned.
    pub(super) allowed_agents: Vec<String>,
    /// Whether workspace restriction should be inherited.
    pub(super) restrict_to_workspace: bool,
    /// Base directory for the child agent process.
    pub(super) base_dir: PathBuf,
    /// Directory for UDS sockets (e.g. `$XDG_RUNTIME_DIR` or temp).
    pub(super) socket_dir: PathBuf,
    /// Shared registry of spawned subagents.
    pub(super) registry: SubagentRegistry,
    pub(super) notify_tx: Option<NotificationTx>,
    pub(super) broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// child events (PRD Stage B).
    pub(super) parent_id: Option<String>,
    pub(super) inherited_tool_policy: super::spawn_inherited_policy::InheritedToolPolicyState,
    /// Session-scoped script-managed environment registry (ADR-0021: built
    /// once at composition and injected).
    pub(super) environment_registry: EnvironmentRegistry,
    /// The parent agent's own config path, plumbed from the composition root
    /// (#1369 follow-up). Container spawns without an explicit `config`
    /// argument fall back to it for loading `container_configs`.
    pub(super) parent_config_path: Option<PathBuf>,
    /// Session-start snapshot of the configured container configs, baked into
    /// the tool description so agents see the menu from turn one (#1410).
    /// Staleness is accepted: the config file is still consulted at spawn
    /// time, and selection errors enumerate the live names.
    pub(super) container_config_roster: String,
}

impl SpawnTool {
    pub fn new(allowed_agents: Vec<String>, restrict_to_workspace: bool) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
            base_dir: PathBuf::new(),
            socket_dir: PathBuf::new(),
            registry: Arc::new(Mutex::new(HashMap::new())),
            notify_tx: None,
            broadcast_tx: None,
            parent_id: None,
            inherited_tool_policy: super::spawn_inherited_policy::new_state(),
            environment_registry: EnvironmentRegistry::new(),
            parent_config_path: None,
            container_config_roster: EMPTY_ROSTER.to_string(),
        }
    }

    /// Create with a base directory for subprocess spawning.
    pub fn with_base_dir(
        allowed_agents: Vec<String>,
        restrict_to_workspace: bool,
        base_dir: PathBuf,
    ) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
            base_dir,
            socket_dir: PathBuf::new(),
            registry: Arc::new(Mutex::new(HashMap::new())),
            notify_tx: None,
            broadcast_tx: None,
            parent_id: None,
            inherited_tool_policy: super::spawn_inherited_policy::new_state(),
            environment_registry: EnvironmentRegistry::new(),
            parent_config_path: None,
            container_config_roster: EMPTY_ROSTER.to_string(),
        }
    }

    /// Plumb the parent agent's own config path from the composition root so
    /// container spawns can fall back to it when `config` is omitted
    /// (#1369 follow-up). `None` leaves only the inherited runtime config
    /// (`QUECTO_RUNTIME_CONFIG_PATH`) as a fallback source.
    pub fn with_parent_config_path(mut self, parent_config_path: Option<PathBuf>) -> Self {
        self.container_config_roster = container_config_roster(parent_config_path.as_deref());
        self.parent_config_path = parent_config_path;
        self
    }

    /// Inject the session-scoped environment registry built at composition.
    pub fn with_environment_registry(mut self, environment_registry: EnvironmentRegistry) -> Self {
        self.environment_registry = environment_registry;
        self
    }

    /// The session-scoped environment registry this tool commits to. Shared
    /// with the environment control use case at composition (#1369 slice 2).
    pub fn environment_registry(&self) -> &EnvironmentRegistry {
        &self.environment_registry
    }

    /// Set the directory for child agent UDS sockets.
    pub fn with_socket_dir(mut self, socket_dir: PathBuf) -> Self {
        self.socket_dir = socket_dir;
        self
    }

    pub(crate) fn with_inherited_tool_policy(
        self,
        snapshot: super::inherited_tool_policy::InheritedToolPolicySnapshot,
    ) -> Self {
        super::spawn_inherited_policy::replace_state(&self.inherited_tool_policy, snapshot);
        self
    }

    pub fn with_registry(mut self, registry: SubagentRegistry) -> Self {
        self.registry = registry;
        self
    }

    pub fn with_notify_tx(mut self, tx: NotificationTx) -> Self {
        self.notify_tx = Some(tx);
        self
    }

    pub fn with_event_forwarding(
        mut self,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
        parent_id: Option<String>,
    ) -> Self {
        self.broadcast_tx = broadcast_tx;
        self.parent_id = parent_id;
        self
    }

    #[cfg(test)]
    pub(crate) async fn send_initial_prompt_for_test(
        &self,
        socket_path: &std::path::Path,
        task: &str,
    ) -> Result<(), DomainError> {
        send_initial_prompt_to_socket(socket_path, task).await
    }

    pub fn registry(&self) -> &SubagentRegistry {
        &self.registry
    }

    /// Real launch-port adapter handle for the shared `tests/contracts/`
    /// suite, so local and script-managed launches run through one behavioral
    /// contract. Not part of the runtime API.
    #[doc(hidden)]
    pub fn launch_ports_for_contract(
        &self,
    ) -> impl crate::domain::subagent_launch::SubagentLaunchPorts<Prepared: Send> + Send + '_ {
        super::spawn_launch_ports::SpawnLaunchPorts::new(self)
    }

    /// Parse spawn arguments and return the resulting config.
    #[cfg(any(test, feature = "test-support"))]
    pub fn parse_args_for_test(&self, arguments: &str) -> Result<SubagentConfig, String> {
        self.parse_args(arguments)
    }

    /// Parse the tool arguments and create a SubagentConfig.
    fn parse_args(&self, arguments: &str) -> Result<SubagentConfig, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let task = args.get("task").and_then(|v| v.as_str()).map(String::from);

        let container = parse_container_selection(&args)?;

        let agent_id = args
            .get("agent_id")
            .and_then(|v| v.as_str())
            .map(String::from);

        let system = args
            .get("system")
            .and_then(|v| v.as_str())
            .map(String::from);

        let config_path = args
            .get("config")
            .and_then(|v| v.as_str())
            .map(validate_config_path)
            .transpose()?;

        let workflow = args
            .get("workflow")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let workflow_guards = args
            .get("workflow_guards")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        // child process fail with an opaque CLI error.
        if workflow_guards && !workflow {
            return Err("workflow_guards requires workflow to also be true".to_string());
        }

        let workflow_spec = match args.get("workflow_spec") {
            Some(v) if !v.is_null() => {
                use serde::Deserialize;
                let spec = crate::domain::workflow::WorkflowSpec::deserialize(v)
                    .map_err(|e| format!("invalid workflow_spec: {}", e))?;
                Some(spec)
            }
            _ => None,
        };

        // explicit model > forwarded --config > built-in default.
        let model_arg = crate::domain::subagent::parse_model_arg(
            args.get("model").and_then(|v| v.as_str()),
            args.get("provider").and_then(|v| v.as_str()),
            args.get("model_id").and_then(|v| v.as_str()),
        )
        .map_err(|e| format!("invalid model: {e}"))?;
        let model = model_arg.map(|m| m.to_model_string());

        let effort =
            super::spawn_launch_args::parse_effort_arg(args.get("effort"), model.as_deref())?;

        if let Some(ref id) = agent_id {
            super::subagent_registry::validate_agent_id_format(id)?;
            if !self.allowed_agents.is_empty() {
                validate_agent_id(id, &self.allowed_agents).map_err(|e| e.to_string())?;
            }
        }

        let disable_tools = parse_disable_tools(&args)?;

        let read_only = {
            let has = |name: &str| disable_tools.iter().any(|t| t == name);
            has("write") && has("edit")
        };

        Ok(SubagentConfig {
            task,
            container,
            agent_id,
            restrict_to_workspace: self.restrict_to_workspace,
            system,
            config_path,
            workflow,
            workflow_guards,
            workflow_spec,
            model,
            effort,
            disable_tools,
            read_only,
        })
    }

    async fn launch_uds_agent(&self, config: &SubagentConfig) -> Result<ToolResult, DomainError> {
        let session_name = config.agent_id.as_deref().unwrap_or("subagent");
        let duplicate = {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            let resolution_entries: Vec<_> = entries
                .iter()
                .map(|(key, entry)| DisplayNameResolutionEntry {
                    agent_uuid: entry.agent_uuid.clone(),
                    display_name: entry.effective_display_name(key).to_string(),
                    live: entry.status != super::subagent_registry::SubagentStatus::Exited,
                })
                .collect();
            assert_display_name_available_for_spawn(&resolution_entries, session_name).err()
        };
        if let Some(
            DisplayNameResolveError::AmbiguousLiveMatch { display_name }
            | DisplayNameResolveError::NoLiveMatch { display_name },
        ) = duplicate
        {
            return Ok(ToolResult {
                content: format!(
                    "Failed to spawn subagent: duplicate live subagent display label '{}'",
                    display_name
                ),
                is_error: true,
                image_blocks: vec![],
            });
        }
        SubagentLaunchUseCase::new(super::spawn_launch_ports::SpawnLaunchPorts::new(self))
            .execute(config)
            .await
    }

    /// Poll until the UDS socket is connectable (up to 10s).
    pub(super) async fn wait_for_socket(&self, path: &std::path::Path) -> Result<(), DomainError> {
        use tokio::time::Instant;

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut interval = tokio::time::interval_at(
            Instant::now() + Duration::from_millis(100),
            Duration::from_millis(100),
        );
        loop {
            interval.tick().await;
            if tokio::net::UnixStream::connect(path).await.is_ok() {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(DomainError::Tool(format!(
                    "subagent socket {} did not become ready within 10s",
                    path.display()
                )));
            }
        }
    }

    pub(super) async fn wait_for_socket_or_child_exit(
        &self,
        path: &std::path::Path,
        child: &mut tokio::process::Child,
    ) -> Result<(), DomainError> {
        tokio::select! {
            socket_result = self.wait_for_socket(path) => socket_result,
            child_status = child.wait() => {
                let detail = match child_status {
                    Ok(status) => format!(" with status {status}"),
                    Err(error) => format!(": failed to observe exit status: {error}"),
                };
                Err(DomainError::Tool(format!(
                    "subagent exited before socket ready{}",
                    detail
                )))
            }
        }
    }
}

pub(super) async fn send_initial_prompt_to_socket(
    socket_path: &std::path::Path,
    task: &str,
) -> Result<(), DomainError> {
    let cmd = serde_json::json!({"type": "prompt", "message": task, "ack": "accept"});
    super::subagent_registry::send_subagent_uds_command_with_timeout(
        socket_path,
        &cmd.to_string(),
        super::subagent_registry::INSPECTOR_RESPONSE_TIMEOUT,
    )
    .await
    .map(|_| ())
    .map_err(|e| DomainError::Tool(format!("failed to send prompt to subagent: {e}")))
}

impl Tool for SpawnTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: format!(
                "Spawn a subagent as a background UDS-mode process. \
                Returns as soon as the child socket is ready — not when the task finishes. \
                REQUIRED sequence: spawn → end this turn (or do other non-blocking work) → \
                on your NEXT turn a passive one-line completion note arrives → then \
                agent_cmd get_messages (count 1-5) for the report. Do NOT poll \
                get_subagents/get_subagents_all/get_state, sleep, or busy-wait in this turn. \
                The note is a lifecycle summary only, not the child's answer. Multiple \
                completions may be deduped into one note. \
                Available container configs: {}.",
                self.container_config_roster
            )
            .into(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"Initial task to send to the subagent (optional — starts idle if omitted)"},"agent_id":{"type":"string","description":"Session name for the subagent (used to address it via agent_cmd)"},"system":{"type":"string","description":"System prompt for the subagent"},"config":{"type":"string","description":"Path to a config file to pass to the child agent via --config (optional). For NEW-environment container spawns (container true or mode \"new\") container_configs load from a trusted absolute path: an explicit config here wins; when omitted, the spawn falls back to the parent's own effective config path, so you normally do NOT need to pass it. Whichever path applies must be absolute. Joins via mode \"existing\" use the environment's retained container config and never need it."},"container":{"description":"Launch adapter selection. Omitted or false: local child process (default). true: new container via the container config labeled default (see the tool description for the available container configs). {\"mode\":\"new\",\"container_config\"?,\"name\"?}: new container via the named config, with an optional container name for later joins/kills. A container config is self-contained: its repository and auth are baked into the config itself — there is NO repo field, and the parent's location or checkout is irrelevant. A config with no repository is a sandbox (empty workspace). {\"mode\":\"existing\",\"ref\"|\"name\"}: add this agent to a running container by its session ref (e.g. C1) or unambiguous name. A successful container spawn returns environment_ref=CN; list containers with agent_cmd get_containers and stop them with kill_container. Unknown fields are rejected."},"model":{"type":"string","description":"Model for the child in provider/model form (e.g. 'openai/gpt-5.5'), same format as agent_cmd set_model. Forwarded to the child as --model at launch so its FIRST turn runs on this model. Precedence: explicit model > --config > built-in default. Invalid combinations are rejected with a clear error."},"effort":{"type":"string","description":"Reasoning effort for the child. Must be one of: none, low, medium, high, xhigh, max. Forwarded as --effort at launch. Precedence: explicit spawn effort > child forwarded agents.defaults.effort > inherited QUECTO_AGENTS_DEFAULTS_EFFORT > provider default."},"provider":{"type":"string","description":"Provider name for the child model (alternative to model; must be paired with model_id)"},"model_id":{"type":"string","description":"Model id for the child model (used with provider)"},"workflow":{"type":"boolean","description":"Start the child agent with --workflow (requires --mode uds, always enabled for spawned agents)"},"workflow_guards":{"type":"boolean","description":"Start the child agent with --workflow-guards (requires --workflow)"},"disable_tools":{"type":"array","items":{"type":"string"},"description":"Tool names to disable and hide from the child model before its session starts (forwarded as --disable-tool per entry), e.g. [\"write\",\"edit\"]. Disabled tools remain described for policy/UI callers, reject execution, and cannot be re-registered at runtime. Entries must be strings."},"read_only":{"type":"boolean","description":"Convenience that disables the 'write' and 'edit' tools in the child (equivalent to disable_tools:[\"write\",\"edit\"]); unions with any explicit disable_tools. Use to launch read-only children such as reviewers."},"workflow_spec":{"type":"object","description":"Assign a binding workflow to the child by value. Provide the full template inline: {\"template\":{\"id\":...,\"label\":...,\"description\":...,\"steps\":[{\"key\":...,\"label\":...,\"phase\":...}]}}. The child runs exactly this template in Active mode (no template selection) and it overrides the child's default template library.","properties":{"template":{"type":"object"}}}}}"#.into(),
        }
    }

    fn set_inherited_child_policy_snapshot_for_spawn(
        &self,
        snapshot: BTreeMap<String, ProfileAvailabilityScope>,
    ) {
        super::spawn_inherited_policy::set_from_tools(&self.inherited_tool_policy, snapshot);
    }

    fn inherited_child_policy_snapshot_for_spawn(
        &self,
    ) -> Option<BTreeMap<String, ProfileAvailabilityScope>> {
        super::spawn_inherited_policy::tools(&self.inherited_tool_policy)
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            match self.parse_args(&args) {
                Ok(config) => {
                    if self.base_dir.as_os_str().is_empty() {
                        let session_name = config.agent_id.as_deref().unwrap_or("subagent");

                        {
                            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
                            if entries.values().any(|entry| {
                                entry.display_name == session_name
                                    && entry.status
                                        != super::subagent_registry::SubagentStatus::Exited
                            }) {
                                return Ok(ToolResult {
                                    content: format!(
                                        "Failed to spawn subagent: duplicate live subagent display label '{}'",
                                        session_name
                                    ),
                                    is_error: true,
                                    image_blocks: vec![],
                                });
                            }
                        }

                        let agent_uuid = AgentUuid::mint();
                        let stub_entry = initial_registry_entry(InitialRegistryEntrySpec {
                            agent_uuid: agent_uuid.clone(),
                            display_name: session_name.to_string(),
                            socket_path: child_socket_path(Path::new("/stub"), &agent_uuid),
                            pid: 0,
                            parent_id: self.parent_id.clone(),
                            config: &config,
                            exit_signal_tx: None,
                            cleanup_environment_id: None,
                            cleanup_argv: Vec::new(),
                            environment_registry: None,
                            environment_ref: None,
                        });
                        if let Err(e) = register_and_broadcast(
                            &self.registry,
                            self.broadcast_tx.as_ref(),
                            session_name,
                            stub_entry,
                        ) {
                            return Ok(ToolResult {
                                content: format!("Failed to spawn subagent: {e}"),
                                is_error: true,
                                image_blocks: vec![],
                            });
                        }

                        let msg = format!(
                            "Subagent '{}' is running (uuid={}). Use agent_cmd to interact.",
                            session_name, agent_uuid,
                        );
                        Ok(ToolResult {
                            content: msg,
                            is_error: false,
                            image_blocks: vec![],
                        })
                    } else {
                        self.launch_uds_agent(&config).await
                    }
                }
                Err(e) => Ok(ToolResult {
                    content: format!("Failed to spawn subagent: {}", e),
                    is_error: true,
                    image_blocks: vec![],
                }),
            }
        })
    }
}

#[cfg(test)]
#[path = "tests/spawn_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_cov_tests.rs"]
mod cov_tests;
