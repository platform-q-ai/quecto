// Spawn tool: spawns a child quecto agent process as a UDS-mode background agent.
//
// The child runs `quecto agent --mode uds --persist` and the parent interacts
// with it via the companion `agent_cmd` tool (#421).

use std::collections::HashMap;
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::domain::error::DomainError;
use crate::domain::subagent::{SubagentConfig, validate_agent_id};
use crate::domain::tool::{Tool, ToolDefinition, ToolResult};

pub use super::subagent_registry::{SubagentEntry, SubagentRegistry};

use super::subagent_registry::{ExitSignal, NotificationTx, new_exit_signal_channel};

/// Validate a config file path supplied via the spawn tool's JSON input.
///
/// Rejects paths that contain `..` components to prevent path-traversal attacks
/// (e.g. `../../../../etc/shadow`).  Absolute paths and relative paths without
/// traversal are accepted — the config file may legitimately live anywhere the
/// user chooses, but the LLM must not be able to escape to arbitrary system paths
/// via traversal sequences.
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

/// Write `data` to `path`, creating it privately: `O_CREAT|O_EXCL` (so a
/// pre-planted symlink at the path is rejected rather than followed) with
/// owner-only `0600` permissions. A stale file left by a crashed prior spawn is
/// removed and recreated once (the retry still uses `O_EXCL`). Falls back to a
/// plain write on non-unix platforms.
#[cfg(unix)]
fn write_private_new(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    fn create_excl(path: &std::path::Path) -> std::io::Result<std::fs::File> {
        std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(path)
    }
    let mut file = match create_excl(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            let _ = std::fs::remove_file(path);
            create_excl(path)?
        }
        Err(e) => return Err(e),
    };
    file.write_all(data)
}

#[cfg(not(unix))]
fn write_private_new(path: &std::path::Path, data: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, data)
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

        // Optional by-value workflow assignment. Deserialize straight into the
        // typed domain `WorkflowSpec` (borrowing the JSON value — no clone) so a
        // malformed spec is rejected here with a clear error rather than crashing
        // the child, and the rest of the pipeline carries a domain type, not raw
        // JSON.
        let workflow_spec = match args.get("workflow_spec") {
            Some(v) if !v.is_null() => {
                use serde::Deserialize;
                let spec = crate::domain::workflow::WorkflowSpec::deserialize(v)
                    .map_err(|e| format!("invalid workflow_spec: {}", e))?;
                Some(spec)
            }
            _ => None,
        };

        if let Some(ref id) = agent_id {
            super::subagent_registry::validate_agent_id_format(id)?;
            if !self.allowed_agents.is_empty() {
                validate_agent_id(id, &self.allowed_agents).map_err(|e| e.to_string())?;
            }
        }

        Ok(SubagentConfig {
            task,
            agent_id,
            restrict_to_workspace: self.restrict_to_workspace,
            system,
            config_path,
            workflow,
            workflow_guards,
            workflow_spec,
        })
    }

    /// Launch a child quecto agent in UDS mode and register it in the registry.
    async fn launch_uds_agent(&self, config: &SubagentConfig) -> Result<ToolResult, DomainError> {
        let binary = std::env::current_exe()
            .map_err(|e| DomainError::Tool(format!("cannot find quecto binary: {e}")))?;

        let session_name = config.agent_id.as_deref().unwrap_or("subagent");

        // Reject if this agent_id is already running.
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

        // Deterministic socket path so the parent can address children by name.
        let socket_path = self
            .socket_dir
            .join(format!("quecto-agent-{session_name}.sock"));

        let mut cmd = tokio::process::Command::new(&binary);
        cmd.arg("agent")
            .arg("--mode")
            .arg("uds")
            .arg("-s")
            .arg(session_name)
            .arg("--socket")
            .arg(&socket_path)
            .arg("--persist");

        if let Some(ref system) = config.system {
            cmd.arg("--system").arg(system);
        }

        // Forward --config if the caller specified a custom config path. In
        // managed runtime pods, inherit the active runtime config for spawned
        // subagents so they use the same tool isolation defaults as the parent.
        if let Some(cfg_path) =
            effective_config_path(config.config_path.as_ref(), inherited_runtime_config_path())
        {
            cmd.arg("--config").arg(cfg_path);
        }

        // Tell the child who its parent is, so the child's OWN emitted events
        // carry the correct parent_id (PRD Stage B) — not just the copy the
        // parent's monitor re-stamps when forwarding.
        if let Some(parent_id) = &self.parent_id {
            cmd.arg("--parent-id").arg(parent_id);
        }

        // Forward --workflow / --workflow-guards when requested.
        if config.workflow {
            cmd.arg("--workflow");
        }
        if config.workflow_guards {
            cmd.arg("--workflow-guards");
        }

        // A by-value workflow assignment is written to a file next to the
        // socket and forwarded as `--workflow-spec <path>`; the inline template
        // is too large for a bare CLI arg. The child runs it in Active mode
        // (binding) and deletes the file once read — see agent_tool_registry.
        if let Some(ref spec) = config.workflow_spec {
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
            cmd.arg("--workflow-spec").arg(&spec_path);
        }

        // Propagate --no-sandbox so child agents inherit the same workspace
        // restriction posture as the parent.
        if !self.restrict_to_workspace {
            cmd.arg("--no-sandbox");
        }

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

        // Wait for socket readiness.
        if let Err(e) = self.wait_for_socket(&socket_path).await {
            // Socket never became ready — kill the child and report failure.
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
        // so the monitor's update_entry calls find the entry (#522).
        {
            let mut entry = SubagentEntry::new(socket_path.clone(), pid);
            entry.exit_signal_tx = Some(exit_tx.clone());
            // Stamp the child's parent as THIS agent's own id (#820 panel tree):
            // without it every entry's parent_id stayed None, so grandchildren
            // could never nest under their real parent in the sub-agent panel.
            entry.parent_id = self.parent_id.clone();
            self.registry
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert(session_name.to_string(), entry);
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

            // Abort the monitor task so it doesn't leak (#522). The aborted
            // monitor will NOT emit its EOF->Exited notification, so we must
            // announce the exit ourselves below rather than relying on it (#831).
            {
                let entries = reaper_registry.lock().unwrap_or_else(|e| e.into_inner());
                if let Some(entry) = entries.get(&reaper_name) {
                    if let Some(ref handle) = entry.monitor_handle {
                        handle.abort();
                    }
                }
            }

            // Cascade-remove the dead agent AND its descendants, then broadcast
            // the survivor set so every connected client (the TUI panel) drops
            // them promptly instead of leaving them lingering (#831). The reaper
            // is a detached task, so the send is best-effort (errors at debug).
            if let Some(event) =
                crate::infrastructure::tools::subagent_monitor::cascade_remove_and_state_changed(
                    &reaper_registry,
                    &reaper_name,
                )
            {
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
        });

        // If the caller provided an initial task, send it as the first prompt.
        // Fire-and-forget: the child acks the prompt internally, but we don't
        // read the response here. Use agent_cmd get_messages_tail to check output.
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
                get_messages_tail or get_messages to read the child's full output. \
                Blocking via agent_cmd command=await is OPTIONAL: use it only when you \
                must wait synchronously (same turn) until the child reaches \
                idle/exited/timeout/error before continuing."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"Initial task to send to the subagent (optional — starts idle if omitted)"},"agent_id":{"type":"string","description":"Session name for the subagent (used to address it via agent_cmd)"},"system":{"type":"string","description":"System prompt for the subagent"},"config":{"type":"string","description":"Path to a config file to pass to the child agent via --config (optional)"},"workflow":{"type":"boolean","description":"Start the child agent with --workflow (requires --mode uds, always enabled for spawned agents)"},"workflow_guards":{"type":"boolean","description":"Start the child agent with --workflow-guards (requires --workflow)"},"workflow_spec":{"type":"object","description":"Assign a binding workflow to the child by value. Provide the full template inline: {\"template\":{\"id\":...,\"label\":...,\"description\":...,\"steps\":[{\"key\":...,\"label\":...,\"phase\":...}]}}. The child runs exactly this template in Active mode (no template selection) and it overrides the child's default template library.","properties":{"template":{"type":"object"}}}}}"#.into(),
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

                        // Register in stub mode too so BDD tests can verify registry.
                        let mut stub_entry = SubagentEntry::new(
                            PathBuf::from(format!("/stub/quecto-agent-{session_name}.sock")),
                            0,
                        );
                        // Mirror the real path: stamp the child's parent as this
                        // agent's own id so the panel tree nests correctly.
                        stub_entry.parent_id = self.parent_id.clone();
                        self.registry
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(session_name.to_string(), stub_entry);

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

/// Send SIGTERM to all tracked subagent processes and clear the registry.
/// Also aborts all monitor tasks (#522).
pub fn shutdown_all(registry: &SubagentRegistry) {
    let mut entries = registry.lock().unwrap_or_else(|e| e.into_inner());
    for (name, entry) in entries.iter() {
        // Abort monitor task if running (#522).
        if let Some(ref handle) = entry.monitor_handle {
            handle.abort();
            tracing::info!(agent = %name, "aborted monitor task");
        }
        if entry.pid != 0 {
            // Use kill(1) rather than libc::kill to avoid adding libc as a dependency.
            let _ = std::process::Command::new("kill")
                .arg("-TERM")
                .arg(entry.pid.to_string())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
            tracing::info!(agent = %name, pid = entry.pid, "sent SIGTERM to subagent");
        }
    }
    entries.clear();
}

#[cfg(test)]
#[path = "spawn_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "spawn_cov_tests.rs"]
mod cov_tests;
