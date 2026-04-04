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
    /// Whether network passthrough should be inherited.
    network_passthrough: bool,
    /// Base directory for the child agent process.
    base_dir: PathBuf,
    /// Directory for UDS sockets (e.g. `$XDG_RUNTIME_DIR` or temp).
    socket_dir: PathBuf,
    /// Shared registry of spawned subagents.
    registry: SubagentRegistry,
    /// Optional notification sender for parent LLM auto-notify (#523).
    notify_tx: Option<NotificationTx>,
}

impl SpawnTool {
    pub fn new(allowed_agents: Vec<String>, restrict_to_workspace: bool) -> Self {
        Self {
            allowed_agents,
            restrict_to_workspace,
            network_passthrough: false,
            base_dir: PathBuf::new(),
            socket_dir: PathBuf::new(),
            registry: Arc::new(Mutex::new(HashMap::new())),
            notify_tx: None,
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
            network_passthrough: false,
            base_dir,
            socket_dir: PathBuf::new(),
            registry: Arc::new(Mutex::new(HashMap::new())),
            notify_tx: None,
        }
    }

    /// Set the directory for child agent UDS sockets.
    pub fn with_socket_dir(mut self, socket_dir: PathBuf) -> Self {
        self.socket_dir = socket_dir;
        self
    }

    /// Enable or disable network passthrough for spawned child agents.
    pub fn with_network(mut self, network_passthrough: bool) -> Self {
        self.network_passthrough = network_passthrough;
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
            .map(|s| validate_config_path(s))
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
            return Err(
                "workflow_guards requires workflow to also be true".to_string(),
            );
        }

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

        // Forward --config if the caller specified a custom config path.
        if let Some(ref cfg_path) = config.config_path {
            cmd.arg("--config").arg(cfg_path);
        }

        // Forward --workflow / --workflow-guards when requested.
        if config.workflow {
            cmd.arg("--workflow");
        }
        if config.workflow_guards {
            cmd.arg("--workflow-guards");
        }

        // Propagate --no-sandbox so child agents inherit the same workspace
        // restriction posture as the parent.
        if !self.restrict_to_workspace {
            cmd.arg("--no-sandbox");
        }

        // Propagate --network so child agents inherit the same network access
        // posture as the parent.
        if self.network_passthrough {
            cmd.arg("--network");
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

            // Abort the monitor task and remove from registry when the child exits.
            let mut entries = reaper_registry.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(entry) = entries.get(&reaper_name) {
                if let Some(ref handle) = entry.monitor_handle {
                    handle.abort();
                }
            }
            entries.remove(&reaper_name);
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
                Returns immediately. Use agent_cmd to send commands, check status, \
                read results, steer, or abort the subagent."
                .into(),
            parameters_schema: r#"{"type":"object","properties":{"task":{"type":"string","description":"Initial task to send to the subagent (optional — starts idle if omitted)"},"agent_id":{"type":"string","description":"Session name for the subagent (used to address it via agent_cmd)"},"system":{"type":"string","description":"System prompt for the subagent"},"config":{"type":"string","description":"Path to a config file to pass to the child agent via --config (optional)"},"workflow":{"type":"boolean","description":"Start the child agent with --workflow (requires --mode uds, always enabled for spawned agents)"},"workflow_guards":{"type":"boolean","description":"Start the child agent with --workflow-guards (requires --workflow)"}}}"#.into(),
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
                        self.registry
                            .lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .insert(
                                session_name.to_string(),
                                SubagentEntry::new(
                                    PathBuf::from(format!(
                                        "/stub/quecto-agent-{session_name}.sock"
                                    )),
                                    0,
                                ),
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
mod tests {
    use super::*;

    fn test_tool() -> SpawnTool {
        SpawnTool::new(
            vec!["news-bot".to_string(), "weather-bot".to_string()],
            true,
        )
    }

    #[test]
    fn test_definition() {
        let tool = test_tool();
        let def = tool.definition();
        assert_eq!(def.name, "spawn");
        assert!(!def.description.is_empty());
        assert!(def.description.contains("agent_cmd"));
    }

    #[test]
    fn test_definition_task_not_required() {
        let tool = test_tool();
        let def = tool.definition();
        let schema: serde_json::Value = serde_json::from_str(&def.parameters_schema).unwrap();
        // No "required" array — task is optional
        assert!(
            schema.get("required").is_none(),
            "task should not be required in schema"
        );
    }

    #[test]
    fn test_parse_valid_task() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":"Summarize news"}"#).unwrap();
        assert_eq!(config.task.as_deref(), Some("Summarize news"));
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_parse_without_task() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"agent_id":"news-bot"}"#).unwrap();
        assert!(config.task.is_none());
        assert_eq!(config.agent_id.as_deref(), Some("news-bot"));
    }

    #[test]
    fn test_parse_empty_object() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{}"#).unwrap();
        assert!(config.task.is_none());
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_parse_with_agent_id() {
        let tool = test_tool();
        let config = tool
            .parse_args(r#"{"task":"Get weather","agent_id":"weather-bot"}"#)
            .unwrap();
        assert_eq!(config.agent_id.as_deref(), Some("weather-bot"));
    }

    #[test]
    fn test_parse_disallowed_agent() {
        let tool = test_tool();
        let result = tool.parse_args(r#"{"task":"Evil task","agent_id":"evil-bot"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not allowed"));
    }

    #[test]
    fn test_parse_empty_allowlist_permits_any() {
        let tool = SpawnTool::new(vec![], true);
        let config = tool
            .parse_args(r#"{"task":"Do stuff","agent_id":"any-bot"}"#)
            .unwrap();
        assert_eq!(config.agent_id.as_deref(), Some("any-bot"));
    }

    #[test]
    fn test_parse_with_system_prompt() {
        let tool = test_tool();
        let config = tool
            .parse_args(r#"{"task":"Summarize","system":"You are a summarizer"}"#)
            .unwrap();
        assert_eq!(config.system.as_deref(), Some("You are a summarizer"));
    }

    #[test]
    fn test_parse_rejects_invalid_agent_id_format() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.parse_args(r#"{"task":"Do stuff","agent_id":"../escape"}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_with_base_dir_sets_fields() {
        let base = PathBuf::from("/tmp/quecto-test");
        let tool = SpawnTool::with_base_dir(vec!["bot-a".to_string()], false, base.clone());
        assert_eq!(tool.base_dir, base);
        assert_eq!(tool.allowed_agents, vec!["bot-a".to_string()]);
        assert!(!tool.restrict_to_workspace);
    }

    #[test]
    fn test_new_sets_empty_base_dir() {
        let tool = SpawnTool::new(vec![], false);
        assert!(tool.base_dir.as_os_str().is_empty());
    }

    #[test]
    fn test_with_registry_shares_state() {
        let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
        let tool = SpawnTool::new(vec![], true).with_registry(registry.clone());
        registry.lock().unwrap().insert(
            "test".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 123),
        );
        assert!(tool.registry.lock().unwrap().contains_key("test"));
    }
    #[test]
    fn test_validate_agent_id_format_empty_string() {
        let result = super::super::subagent_registry::validate_agent_id_format("");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("1-64 characters"));
    }

    #[test]
    fn test_validate_agent_id_format_max_length_64() {
        let id = "a".repeat(64);
        let result = super::super::subagent_registry::validate_agent_id_format(&id);
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_too_long_65() {
        let id = "a".repeat(65);
        let result = super::super::subagent_registry::validate_agent_id_format(&id);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("1-64 characters"));
    }

    #[test]
    fn test_validate_agent_id_format_all_valid_chars() {
        assert!(super::super::subagent_registry::validate_agent_id_format("abcXYZ019_-").is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_single_char() {
        use super::super::subagent_registry::validate_agent_id_format;
        assert!(validate_agent_id_format("a").is_ok());
        assert!(validate_agent_id_format("Z").is_ok());
        assert!(validate_agent_id_format("0").is_ok());
        assert!(validate_agent_id_format("_").is_ok());
        assert!(validate_agent_id_format("-").is_ok());
    }

    #[test]
    fn test_validate_agent_id_format_invalid_dot() {
        let result = super::super::subagent_registry::validate_agent_id_format("hello.world");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_validate_agent_id_format_invalid_space() {
        let result = super::super::subagent_registry::validate_agent_id_format("hello world");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("[a-zA-Z0-9_-]"));
    }

    #[test]
    fn test_validate_agent_id_format_invalid_slash() {
        let result = super::super::subagent_registry::validate_agent_id_format("a/b");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_agent_id_format_invalid_unicode() {
        let result = super::super::subagent_registry::validate_agent_id_format("böt");
        assert!(result.is_err());
    }
    #[tokio::test]
    async fn test_execute_stub_mode_success() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool
            .execute(r#"{"task":"Do something useful"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("agent_cmd"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_no_task() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.execute(r#"{"agent_id":"idle-worker"}"#).await.unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("agent_cmd"));
        assert!(result.content.contains("idle-worker"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_registers_in_registry() {
        let tool = SpawnTool::new(vec![], true);
        let _result = tool
            .execute(r#"{"task":"work","agent_id":"my-bot"}"#)
            .await
            .unwrap();
        assert!(tool.registry.lock().unwrap().contains_key("my-bot"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_default_agent_id() {
        let tool = SpawnTool::new(vec![], true);
        let _result = tool.execute(r#"{"task":"work"}"#).await.unwrap();
        assert!(tool.registry.lock().unwrap().contains_key("subagent"));
    }

    #[tokio::test]
    async fn test_execute_stub_mode_with_agent_id() {
        let tool = SpawnTool::new(vec!["my-bot".to_string()], true);
        let result = tool
            .execute(r#"{"task":"fetch data","agent_id":"my-bot"}"#)
            .await
            .unwrap();
        assert!(!result.is_error);
        assert!(result.content.contains("my-bot"));
    }
    #[tokio::test]
    async fn test_execute_invalid_json() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.execute("not valid json").await.unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("Failed to spawn subagent"));
        assert!(result.content.contains("invalid JSON"));
    }

    #[tokio::test]
    async fn test_execute_disallowed_agent_returns_error() {
        let tool = SpawnTool::new(vec!["allowed-bot".to_string()], true);
        let result = tool
            .execute(r#"{"task":"evil","agent_id":"not-allowed"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("not allowed"));
    }

    #[tokio::test]
    async fn test_execute_invalid_agent_id_format_returns_error() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool
            .execute(r#"{"task":"test","agent_id":"bad id!"}"#)
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.content.contains("[a-zA-Z0-9_-]"));
    }
    #[test]
    fn test_parse_args_invalid_json_garbage() {
        let tool = test_tool();
        let result = tool.parse_args("{garbage}}}");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("invalid JSON"));
    }

    #[test]
    fn test_parse_args_task_not_string() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":42}"#).unwrap();
        // task is not a string, so it's None
        assert!(config.task.is_none());
    }

    #[test]
    fn test_parse_args_task_null() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":null}"#).unwrap();
        assert!(config.task.is_none());
    }

    #[test]
    fn test_parse_args_system_not_string_ignored() {
        let tool = test_tool();
        let config = tool.parse_args(r#"{"task":"work","system":123}"#).unwrap();
        assert!(config.system.is_none());
    }

    #[test]
    fn test_parse_args_agent_id_not_string_ignored() {
        let tool = test_tool();
        let config = tool
            .parse_args(r#"{"task":"work","agent_id":999}"#)
            .unwrap();
        assert!(config.agent_id.is_none());
    }

    #[test]
    fn test_parse_args_restrict_to_workspace_inherited() {
        let tool_true = SpawnTool::new(vec![], true);
        let tool_false = SpawnTool::new(vec![], false);
        let cfg_t = tool_true.parse_args(r#"{"task":"a"}"#).unwrap();
        let cfg_f = tool_false.parse_args(r#"{"task":"a"}"#).unwrap();
        assert!(cfg_t.restrict_to_workspace);
        assert!(!cfg_f.restrict_to_workspace);
    }
    #[test]
    fn test_shutdown_all_clears_registry() {
        let registry: SubagentRegistry = Arc::new(Mutex::new(HashMap::new()));
        registry.lock().unwrap().insert(
            "bot".to_string(),
            SubagentEntry::new(PathBuf::from("/tmp/test.sock"), 0),
        );
        assert!(!registry.lock().unwrap().is_empty());
        shutdown_all(&registry);
        assert!(registry.lock().unwrap().is_empty());
    }
    #[test]
    fn test_debug_trait() {
        let tool = SpawnTool::new(vec!["bot".to_string()], true);
        let debug_str = format!("{:?}", tool);
        assert!(debug_str.contains("SpawnTool"));
        assert!(debug_str.contains("bot"));
        assert!(debug_str.contains("restrict_to_workspace: true"));
    }

    #[test]
    fn test_debug_with_base_dir() {
        let tool = SpawnTool::with_base_dir(vec![], false, PathBuf::from("/some/path"));
        let debug_str = format!("{:?}", tool);
        assert!(debug_str.contains("/some/path"));
    }

    // --- config_path validation ---

    #[test]
    fn test_parse_config_path_valid_absolute() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","config":"/home/user/.quecto/config.json"}"#)
            .unwrap();
        assert_eq!(
            cfg.config_path,
            Some(PathBuf::from("/home/user/.quecto/config.json"))
        );
    }

    #[test]
    fn test_parse_config_path_valid_relative() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","config":"configs/custom.json"}"#)
            .unwrap();
        assert_eq!(cfg.config_path, Some(PathBuf::from("configs/custom.json")));
    }

    #[test]
    fn test_parse_config_path_traversal_rejected() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.parse_args(r#"{"task":"work","config":"../../etc/shadow"}"#);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains(".."), "expected '..' in error, got: {err}");
        assert!(err.contains("not allowed"), "expected 'not allowed' in error, got: {err}");
    }

    #[test]
    fn test_parse_config_path_traversal_absolute_rejected() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.parse_args(r#"{"task":"work","config":"/safe/../etc/shadow"}"#);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_config_path_absent_is_none() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
        assert!(cfg.config_path.is_none());
    }

    #[test]
    fn test_parse_config_path_non_string_ignored() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool.parse_args(r#"{"task":"work","config":123}"#).unwrap();
        assert!(cfg.config_path.is_none());
    }

    // --- workflow / workflow_guards validation ---

    #[test]
    fn test_parse_workflow_true() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","workflow":true}"#)
            .unwrap();
        assert!(cfg.workflow);
        assert!(!cfg.workflow_guards);
    }

    #[test]
    fn test_parse_workflow_false_by_default() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool.parse_args(r#"{"task":"work"}"#).unwrap();
        assert!(!cfg.workflow);
        assert!(!cfg.workflow_guards);
    }

    #[test]
    fn test_parse_workflow_guards_requires_workflow() {
        let tool = SpawnTool::new(vec![], true);
        let result = tool.parse_args(r#"{"task":"work","workflow_guards":true}"#);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("workflow_guards requires workflow"));
    }

    #[test]
    fn test_parse_workflow_guards_with_workflow_ok() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","workflow":true,"workflow_guards":true}"#)
            .unwrap();
        assert!(cfg.workflow);
        assert!(cfg.workflow_guards);
    }

    #[test]
    fn test_parse_workflow_non_bool_ignored() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","workflow":"yes"}"#)
            .unwrap();
        assert!(!cfg.workflow);
    }

    #[test]
    fn test_parse_workflow_guards_non_bool_ignored() {
        let tool = SpawnTool::new(vec![], true);
        let cfg = tool
            .parse_args(r#"{"task":"work","workflow_guards":1}"#)
            .unwrap();
        assert!(!cfg.workflow_guards);
    }

    // --- validate_config_path unit tests ---

    #[test]
    fn test_validate_config_path_clean_absolute() {
        let result = validate_config_path("/home/user/.quecto/config.json");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_path_clean_relative() {
        let result = validate_config_path("configs/custom.json");
        assert!(result.is_ok());
    }

    #[test]
    fn test_validate_config_path_dotdot_relative() {
        let result = validate_config_path("../../etc/shadow");
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.contains(".."));
        assert!(err.contains("not allowed"));
    }

    #[test]
    fn test_validate_config_path_dotdot_embedded() {
        let result = validate_config_path("/safe/path/../etc/passwd");
        assert!(result.is_err());
    }

    #[test]
    fn test_validate_config_path_single_dot_ok() {
        // A single "." (current dir) is fine — it's not traversal.
        let result = validate_config_path("./config.json");
        assert!(result.is_ok());
    }
}
