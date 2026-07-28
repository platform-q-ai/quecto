pub use super::subagent_registry::{SubagentEntry, SubagentRegistry};
use crate::domain::error::DomainError;
use crate::domain::subagent::{SubagentConfig, validate_agent_id};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};
use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::spawn_launch_args::write_private_new;
pub use super::spawn_registry::{register_and_broadcast, shutdown_all, shutdown_all_with_count};
use super::subagent_lifecycle::{SubagentLifecycleEvent, apply_lifecycle_event};
#[cfg(test)]
pub use super::subagent_registry::SubagentStatus;
use super::subagent_registry::{ExitSignal, ExitSignalTx, NotificationTx, new_exit_signal_channel};

/// Build the registry entry used at spawn registration (production after socket
/// ready, and stub mode). Shared so the task-dependent initial status (#1049)
/// cannot drift between branches.
fn initial_registry_entry(
    socket_path: PathBuf,
    pid: u32,
    parent_id: Option<String>,
    config: &SubagentConfig,
    exit_signal_tx: Option<ExitSignalTx>,
) -> SubagentEntry {
    let mut entry = SubagentEntry::new(socket_path, pid);
    entry.exit_signal_tx = exit_signal_tx;
    // Stamp the child's parent as THIS agent's own id (#820 panel tree).
    entry.parent_id = parent_id;
    // Record whether this child is a read-only observer (#966 / #957).
    entry.read_only = config.read_only;
    if config.task.is_none() {
        // #1049: task-less → Idle (cascade/TUI); with-task stays Starting.
        entry.status =
            apply_lifecycle_event(&mut entry.lifecycle, SubagentLifecycleEvent::RunEnded);
    }
    super::subagent_registry::seed_bound_workflow(&mut entry, config.workflow_spec.as_ref());
    entry
}

fn inherited_runtime_config_path() -> Option<PathBuf> {
    std::env::var("QUECTO_RUNTIME_CONFIG_PATH")
        .ok()
        .filter(|path| !path.trim().is_empty())
        .map(PathBuf::from)
}

fn effective_config_path(
    explicit_config_path: Option<&PathBuf>,
    inherited_config_path: Option<PathBuf>,
) -> Option<PathBuf> {
    explicit_config_path.cloned().or(inherited_config_path)
}

/// Compute the effective set of tools to disable in the child registry (#957):
/// the explicit `disable_tools` array unioned with the `read_only` convenience
/// (which expands to `write` + `edit`), de-duplicated (read-only tools first). A
/// non-string entry is an LLM-addressable error, not a silent skip.
fn parse_disable_tools(args: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut tools: Vec<String> = Vec::new();
    let push_unique = |name: &str, tools: &mut Vec<String>| {
        if !tools.iter().any(|t| t == name) {
            tools.push(name.to_string());
        }
    };

    // Treat a malformed read_only safety flag as an error rather than silently
    // dropping it; valid true expands first, then unions disable_tools.
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
/// `--mode uds --persist` background process, waits for the UDS socket
/// to become ready, and registers the child in a shared [`SubagentRegistry`]
/// so the companion [`super::agent_cmd::AgentCmdTool`] can interact with it.
#[derive(Debug)]
pub struct SpawnTool {
    /// Allowlist of agent IDs that can be spawned.
    allowed_agents: Vec<String>,
    /// Whether workspace restriction should be inherited.
    restrict_to_workspace: bool,
    /// Base directory for the child agent process.
    base_dir: PathBuf,
    /// Directory for UDS sockets (e.g. `$XDG_RUNTIME_DIR` or temp).
    socket_dir: PathBuf,
    /// Shared registry of spawned subagents.
    registry: SubagentRegistry,
    /// Optional notification sender for parent LLM auto-notify (#523).
    notify_tx: Option<NotificationTx>,
    /// Parent's broadcast channel, so the child monitor can forward the child's
    /// workflow_state events onto the parent's stream (PRD Stage B / R-B2).
    broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
    /// This (parent) agent's own id, stamped as the `parent_id` on forwarded
    /// child events (PRD Stage B).
    parent_id: Option<String>,
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
        }
    }

    /// Set the directory for child agent UDS sockets.
    pub fn with_socket_dir(mut self, socket_dir: PathBuf) -> Self {
        self.socket_dir = socket_dir;
        self
    }

    /// Inject a shared subagent registry (used when wiring spawn + agent_cmd together).
    pub fn with_registry(mut self, registry: SubagentRegistry) -> Self {
        self.registry = registry;
        self
    }

    /// Set the notification sender for auto-notifying the parent LLM (#523).
    pub fn with_notify_tx(mut self, tx: NotificationTx) -> Self {
        self.notify_tx = Some(tx);
        self
    }

    /// Set the parent's broadcast channel + own id so spawned children forward
    /// their workflow_state events onto the parent's stream (PRD Stage B).
    pub fn with_event_forwarding(
        mut self,
        broadcast_tx: Option<tokio::sync::broadcast::Sender<String>>,
        parent_id: Option<String>,
    ) -> Self {
        self.broadcast_tx = broadcast_tx;
        self.parent_id = parent_id;
        self
    }

    /// Return a reference to the shared registry (for testing / wiring).
    pub fn registry(&self) -> &SubagentRegistry {
        &self.registry
    }

    /// Parse spawn arguments and return the resulting config.
    /// Available in tests and the `test-support` feature so BDD steps can
    /// inspect parsed values without promoting the method to the full public API.
    #[cfg(any(test, feature = "test-support"))]
    pub fn parse_args_for_test(&self, arguments: &str) -> Result<SubagentConfig, String> {
        self.parse_args(arguments)
    }

    /// Parse the tool arguments and create a SubagentConfig.
    fn parse_args(&self, arguments: &str) -> Result<SubagentConfig, String> {
        let args: serde_json::Value =
            serde_json::from_str(arguments).map_err(|e| format!("invalid JSON: {}", e))?;

        let task = args.get("task").and_then(|v| v.as_str()).map(String::from);

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

        // workflow_guards requires workflow — reject early rather than letting the
        // child process fail with an opaque CLI error.
        if workflow_guards && !workflow {
            return Err("workflow_guards requires workflow to also be true".to_string());
        }

        // Borrow-deserialize into the domain type so malformed by-value specs
        // fail here clearly and raw JSON never leaks into the launch pipeline.
        let workflow_spec = match args.get("workflow_spec") {
            Some(v) if !v.is_null() => {
                use serde::Deserialize;
                let spec = crate::domain::workflow::WorkflowSpec::deserialize(v)
                    .map_err(|e| format!("invalid workflow_spec: {}", e))?;
                Some(spec)
            }
            _ => None,
        };

        // #881: share set_model parsing so accepted forms cannot diverge;
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

        // Observer sub-agents have both mutation tools disabled (#966).
        let read_only = {
            let has = |name: &str| disable_tools.iter().any(|t| t == name);
            has("write") && has("edit")
        };

        Ok(SubagentConfig {
            task,
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

        {
            let entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            if entries.contains_key(session_name) {
                return Ok(ToolResult {
                    content: format!(
                        "Failed to spawn subagent: agent '{}' is already running",
                        session_name
                    ),
                    is_error: true,
                    image_blocks: vec![],
                });
            }
        }

        let socket_path = self
            .socket_dir
            .join(format!("quecto-agent-{session_name}.sock"));

        // A by-value workflow assignment is written to a file next to the
        // socket and forwarded as `--workflow-spec <path>`; the inline template
        // is too large for a bare CLI arg. The child runs it in Active mode
        // (binding) and deletes the file once read — see agent_tool_registry.
        let workflow_spec_path = if let Some(ref spec) = config.workflow_spec {
            let spec_json = serde_json::to_string(spec).map_err(|e| {
                DomainError::Tool(format!("failed to serialize workflow spec: {e}"))
            })?;
            if spec_json.len() > crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES {
                return Err(DomainError::Tool(format!(
                    "workflow spec too large: {} bytes (max {})",
                    spec_json.len(),
                    crate::domain::workflow::MAX_WORKFLOW_SPEC_BYTES
                )));
            }
            // Unique per spawn (pid + session) and created privately with
            // O_CREAT|O_EXCL + mode 0600 so a pre-planted symlink at the path
            // cannot be followed/overwritten and the contents are owner-only.
            let spec_path = self.socket_dir.join(format!(
                "quecto-wfspec-{session_name}-{}.json",
                std::process::id()
            ));
            write_private_new(&spec_path, spec_json.as_bytes())
                .map_err(|e| DomainError::Tool(format!("failed to write workflow spec: {e}")))?;
            Some(spec_path)
        } else {
            None
        };

        // Build the full child argument list (incl. `--model`, #881) via the
        // pure builder so the exact flag set is unit-testable.
        let effective_config =
            effective_config_path(config.config_path.as_ref(), inherited_runtime_config_path());
        let cli_args = super::spawn_launch_args::build_child_cli_args(
            &super::spawn_launch_args::ChildLaunchSpec {
                session_name,
                socket_path: &socket_path,
                config,
                effective_config: effective_config.as_deref(),
                parent_id: self.parent_id.as_deref(),
                restrict_to_workspace: self.restrict_to_workspace,
                workflow_spec_path: workflow_spec_path.as_deref(),
            },
        );

        let binary = super::spawn_binary::resolve_child_binary()?;
        let mut cmd = tokio::process::Command::new(&binary);
        cmd.args(&cli_args);

        if !self.base_dir.as_os_str().is_empty() {
            cmd.env("QUECTO_BASE_DIR", &self.base_dir);
        }

        // Detach child stdio — we interact via UDS, not stdout/stderr.
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());

        let mut child = cmd
            .spawn()
            .map_err(|e| DomainError::Tool(format!("failed to spawn subagent: {e}")))?;

        let pid = child.id().unwrap_or(0);

        // Wait for socket readiness while also observing premature child exit.
        if let Err(e) = self
            .wait_for_socket_or_child_exit(&socket_path, &mut child)
            .await
        {
            // Socket never became ready — kill the child if it is still alive and
            // report failure. If the child already exited, kill/wait are benign.
            let _ = child.kill().await;
            let _ = child.wait().await;
            return Err(e);
        }

        // Create exit signal channel for `await` support (#612).
        // The receiver is intentionally dropped — `watch` channels remain
        // functional after the initial receiver is dropped. Await callers
        // get their own receiver via `tx.subscribe()`. Do NOT switch to
        // `mpsc` or `oneshot` without updating the subscribe pattern.
        let (exit_tx, _exit_rx) = new_exit_signal_channel();

        // Register in shared registry BEFORE starting the monitor task,
        // so the monitor's update_entry calls find the entry (#522). The insert
        // also broadcasts the survivor set immediately so the TUI learns of the
        // new child at once instead of waiting for the next GetSubagents poll or
        // a terminal event — without this a child that begins a long first turn
        // stays invisible in the side panel until it finishes (#866).
        {
            let entry = initial_registry_entry(
                socket_path.clone(),
                pid,
                self.parent_id.clone(),
                config,
                Some(exit_tx.clone()),
            );
            register_and_broadcast(
                &self.registry,
                self.broadcast_tx.as_ref(),
                session_name,
                entry,
            );
        }

        // Start a persistent monitor task to track child events in real-time (#522).
        // Pass the notification sender so the monitor can auto-notify the parent (#523).
        let monitor_handle = super::subagent_monitor::spawn_monitor_task(
            session_name.to_string(),
            socket_path.clone(),
            self.registry.clone(),
            self.notify_tx.clone(),
            self.broadcast_tx.clone(),
            self.parent_id.clone(),
        );

        // Store the monitor handle so it can be aborted on shutdown.
        {
            let mut entries = self.registry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = entries.get_mut(session_name) {
                entry.monitor_handle = Some(std::sync::Arc::new(monitor_handle));
            }
        }

        // Spawn a background reaper task so the child process is always
        // cleaned up (no zombies) even if the parent never calls shutdown_all.
        // The reaper also aborts the monitor task to prevent leaks (#522),
        // and signals any waiting `await` calls with the exit status (#612).
        let reaper_registry = self.registry.clone();
        let reaper_name = session_name.to_string();
        let reaper_exit_tx = exit_tx;
        let reaper_broadcast = self.broadcast_tx.clone();
        tokio::spawn(async move {
            let status = child.wait().await;

            // Build the exit signal from the child's exit status (#612).
            let exit_signal = match status {
                Ok(exit_status) => {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        if let Some(signal) = exit_status.signal() {
                            ExitSignal {
                                exit_code: None,
                                signal: Some(signal),
                            }
                        } else {
                            ExitSignal {
                                exit_code: exit_status.code(),
                                signal: None,
                            }
                        }
                    }
                    #[cfg(not(unix))]
                    {
                        ExitSignal {
                            exit_code: exit_status.code(),
                            signal: None,
                        }
                    }
                }
                Err(_) => ExitSignal {
                    exit_code: None,
                    signal: None,
                },
            };

            // Signal any waiting `await` call before removing from registry.
            let _ = reaper_exit_tx.send(Some(exit_signal));

            // Cascade-remove the dead agent AND its descendants, then broadcast
            // the survivor set so every connected client (the TUI panel) drops
            // them promptly instead of leaving them lingering (#831). The reaper
            // is a detached task, so the send is best-effort (errors at debug).
            let crate::infrastructure::tools::subagent_cascade::CascadeOutcome { removed, event } =
                crate::infrastructure::tools::subagent_cascade::cascade_remove_and_state_changed(
                    &reaper_registry,
                    &reaper_name,
                );
            if let Some(event) = event {
                if let Some(tx) = &reaper_broadcast {
                    if let Err(e) = tx.send(event) {
                        tracing::debug!(
                            agent = %reaper_name,
                            error = %e,
                            "reaper: no subscribers for cascade state_changed broadcast"
                        );
                    }
                }
            }

            // Clean up the removed sub-tree. The dead agent itself was already
            // wait()ed, so we only abort its monitor (the aborted monitor will
            // NOT emit its EOF->Exited notification — that is why we announced the
            // exit above rather than relying on it, #831). For DESCENDANTS we also
            // SIGTERM their processes: when a parent exits its children are
            // orphaned and would otherwise leak as untracked processes that
            // `shutdown_all` can no longer reach (#831 security review). We do NOT
            // re-signal the dead agent's own pid (already reaped; avoids a
            // pid-reuse TOCTOU race).
            for (id, entry) in &removed {
                if id == &reaper_name {
                    if let Some(ref handle) = entry.monitor_handle {
                        handle.abort();
                    }
                    continue;
                }
                if let Some(ref tx) = entry.exit_signal_tx {
                    let _ = tx.send(Some(ExitSignal {
                        exit_code: None,
                        signal: Some(15), // SIGTERM
                    }));
                }
                crate::infrastructure::tools::subagent_cascade::terminate_removed_entry(entry);
            }
        });

        // If the caller provided an initial task, send it as the first prompt.
        // Fire-and-forget: the child acks the prompt internally, but we don't
        // read the response here. Use agent_cmd get_messages to check output.
        if let Some(ref task) = config.task {
            self.send_initial_prompt(&socket_path, task).await?;
        }

        Ok(ToolResult {
            content: format!(
                "Subagent '{}' is running. Use agent_cmd to interact.",
                session_name
            ),
            is_error: false,
            image_blocks: vec![],
        })
    }

    /// Poll until the UDS socket is connectable (up to 10s).
    async fn wait_for_socket(&self, path: &std::path::Path) -> Result<(), DomainError> {
        use tokio::time::Instant;

        let deadline = Instant::now() + Duration::from_secs(10);
        // Delay the first probe slightly — the child needs time to start up.
        let mut interval = tokio::time::interval_at(
            Instant::now() + Duration::from_millis(100),
            Duration::from_millis(100),
        );
        loop {
            interval.tick().await;
            // Just try to connect — no need for a separate exists() check.
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

    /// Poll until the UDS socket is connectable, but return a specific error if
    /// the child process exits before its socket becomes ready.
    async fn wait_for_socket_or_child_exit(
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

    /// Send the initial task as a UDS prompt after the socket is ready.
    /// Fire-and-forget: writes the prompt and closes the connection.
    async fn send_initial_prompt(
        &self,
        socket_path: &std::path::Path,
        task: &str,
    ) -> Result<(), DomainError> {
        use tokio::io::AsyncWriteExt;
        let mut stream = tokio::net::UnixStream::connect(socket_path)
            .await
            .map_err(|e| DomainError::Tool(format!("failed to connect to subagent: {e}")))?;
        let cmd = serde_json::json!({"type": "prompt", "message": task});
        let line = format!("{}\n", cmd);
        stream
            .write_all(line.as_bytes())
            .await
            .map_err(|e| DomainError::Tool(format!("failed to send prompt to subagent: {e}")))?;
        stream
            .shutdown()
            .await
            .map_err(|e| DomainError::Tool(format!("failed to shutdown stream: {e}")))?;
        Ok(())
    }
}

impl Tool for SpawnTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "spawn".into(),
            description: "Spawn a subagent as a background UDS-mode process. \
                Returns immediately and the child is auto-noted PASSIVELY: when it \
                completes, errors, or exits you automatically receive a one-line \
                completion note — non-blocking, entering your context at your NEXT turn \
                (no manual await needed). Multiple completions are deduped/coalesced \
                into a single note. The note is a summary only — use agent_cmd \
                get_messages (with count for the last N messages, or before to page older \
                history) to read the child's output. Blocking via agent_cmd command=await is \
                OPTIONAL: use it only when you \
                must wait synchronously (same turn) until the child reaches \
                idle/exited/timeout/error before continuing."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"Initial task to send to the subagent (optional — starts idle if omitted)"},"agent_id":{"type":"string","description":"Session name for the subagent (used to address it via agent_cmd)"},"system":{"type":"string","description":"System prompt for the subagent"},"config":{"type":"string","description":"Path to a config file to pass to the child agent via --config (optional)"},"model":{"type":"string","description":"Model for the child in provider/model form (e.g. 'openai/gpt-5.5'), same format as agent_cmd set_model. Forwarded to the child as --model at launch so its FIRST turn runs on this model. Precedence: explicit model > --config > built-in default. Invalid combinations are rejected with a clear error."},"effort":{"type":"string","description":"Reasoning effort for the child. Must be one of: none, low, medium, high, xhigh, max. Forwarded as --effort at launch. Precedence: explicit spawn effort > child forwarded agents.defaults.effort > inherited QUECTO_AGENTS_DEFAULTS_EFFORT > provider default."},"provider":{"type":"string","description":"Provider name for the child model (alternative to model; must be paired with model_id)"},"model_id":{"type":"string","description":"Model id for the child model (used with provider)"},"workflow":{"type":"boolean","description":"Start the child agent with --workflow (requires --mode uds, always enabled for spawned agents)"},"workflow_guards":{"type":"boolean","description":"Start the child agent with --workflow-guards (requires --workflow)"},"disable_tools":{"type":"array","items":{"type":"string"},"description":"Tool names to remove from the child's registry before its session starts (forwarded as --disable-tool per entry), e.g. [\"write\",\"edit\"]. The child model never sees the removed tools. Entries must be strings."},"read_only":{"type":"boolean","description":"Convenience that disables the 'write' and 'edit' tools in the child (equivalent to disable_tools:[\"write\",\"edit\"]); unions with any explicit disable_tools. Use to launch read-only children such as reviewers."},"workflow_spec":{"type":"object","description":"Assign a binding workflow to the child by value. Provide the full template inline: {\"template\":{\"id\":...,\"label\":...,\"description\":...,\"steps\":[{\"key\":...,\"label\":...,\"phase\":...}]}}. The child runs exactly this template in Active mode (no template selection) and it overrides the child's default template library.","properties":{"template":{"type":"object"}}}}}"#.into(),
        }
    }

    fn execute(
        &self,
        arguments: &str,
    ) -> Pin<Box<dyn Future<Output = Result<ToolResult, DomainError>> + Send + '_>> {
        let args = arguments.to_string();
        Box::pin(async move {
            match self.parse_args(&args) {
                Ok(config) => {
                    // Only spawn subprocess when base_dir is configured (CLI agent mode).
                    // Otherwise return a stub result (unit test / isolated mode).
                    if self.base_dir.as_os_str().is_empty() {
                        let session_name = config.agent_id.as_deref().unwrap_or("subagent");

                        // Stub registration uses the same entry builder as the
                        // real post-socket-ready path so status/parent/read_only
                        // and #1049 cannot drift (#866 broadcast still applies).
                        let stub_entry = initial_registry_entry(
                            PathBuf::from(format!("/stub/quecto-agent-{session_name}.sock")),
                            0,
                            self.parent_id.clone(),
                            &config,
                            None,
                        );
                        register_and_broadcast(
                            &self.registry,
                            self.broadcast_tx.as_ref(),
                            session_name,
                            stub_entry,
                        );

                        let msg = format!(
                            "Subagent '{}' is running. Use agent_cmd to interact.",
                            session_name,
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
