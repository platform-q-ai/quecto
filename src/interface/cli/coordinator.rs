//! `quecto coordinator` subcommand — LLM-driven coordinator agent.
//!
//! Runs a long-lived agent process that manages coding jobs autonomously.
//! The coordinator has its own LLM provider, tool registry (with `coding_job`
//! in inline mode + `recall`), and a heartbeat loop that polls the IPC inbox
//! for commands from the main agent.
//!
//! Architecture: this is a mini-gateway — same pattern as `Gateway::run()`
//! but without Telegram channels. The heartbeat reads pending IPC commands,
//! converts them to agent messages, runs the agent loop, and writes
//! responses back to the IPC outbox.
//!
//! Security: the coordinator's tool registry is restricted to `coding_job`
//! and `recall` — no exec/fs/spawn tools. This prevents privilege
//! escalation if the coordinator's LLM is tricked via prompt injection.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::heartbeat;
use crate::domain::agent::AgentLoop;
use crate::domain::coding_ipc::{CoordinatorIpc, CoordinatorIpcCommand};
use crate::domain::workspace::HeartbeatTaskSource;
use crate::infrastructure::coding::coordinator_ipc::FileCoordinatorIpc;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

use crate::interface::shared::{SharedLifecycleDriver, build_coding_lifecycle};

// ── Parsed arguments ────────────────────────────────────────────────────

/// Parsed coordinator command-line arguments.
#[derive(Debug, Clone)]
pub struct CoordinatorArgs {
    /// Path to the IPC directory (coordinator/).
    pub ipc_dir: String,
    /// How often to poll the inbox (heartbeat interval), in seconds.
    pub heartbeat_interval_secs: u64,
}

/// Default heartbeat interval in seconds.
const DEFAULT_HEARTBEAT_INTERVAL_SECS: u64 = 60;

/// Minimum allowed heartbeat interval in seconds.
const MIN_HEARTBEAT_INTERVAL_SECS: u64 = 10;

/// Parse coordinator flags from a slice of CLI arguments.
pub fn parse_coordinator_args(args: &[String]) -> Result<CoordinatorArgs, String> {
    let mut ipc_dir: Option<String> = None;
    let mut heartbeat_interval_secs: Option<u64> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--ipc-dir" => {
                ipc_dir = Some(require_next(args, &mut i, "--ipc-dir")?);
            }
            "--heartbeat-interval" => {
                let val = require_next(args, &mut i, "--heartbeat-interval")?;
                let n: u64 = val
                    .parse()
                    .map_err(|_| format!("--heartbeat-interval must be a number, got '{val}'"))?;
                if n < MIN_HEARTBEAT_INTERVAL_SECS {
                    return Err(format!(
                        "--heartbeat-interval must be >= {MIN_HEARTBEAT_INTERVAL_SECS}, got {n}"
                    ));
                }
                heartbeat_interval_secs = Some(n);
            }
            // Legacy flag — accept but convert to heartbeat interval.
            "--poll-interval-ms" => {
                let val = require_next(args, &mut i, "--poll-interval-ms")?;
                let ms: u64 = val
                    .parse()
                    .map_err(|_| format!("--poll-interval-ms must be a number, got '{val}'"))?;
                // Convert ms to seconds, enforce minimum.
                let secs = (ms / 1000).max(MIN_HEARTBEAT_INTERVAL_SECS);
                heartbeat_interval_secs = Some(secs);
            }
            "--help" | "-h" => {
                return Err("coordinator: see documentation for usage".to_string());
            }
            other if other.starts_with("--") || other.starts_with('-') => {
                return Err(format!("unknown flag '{other}'"));
            }
            _ => {
                i += 1;
                continue;
            }
        }
    }

    let ipc_dir = ipc_dir.ok_or("missing required flag --ipc-dir")?;

    Ok(CoordinatorArgs {
        ipc_dir,
        heartbeat_interval_secs: heartbeat_interval_secs.unwrap_or(DEFAULT_HEARTBEAT_INTERVAL_SECS),
    })
}

fn require_next(args: &[String], i: &mut usize, flag: &str) -> Result<String, String> {
    *i += 1;
    if *i < args.len() {
        let val = args[*i].clone();
        *i += 1;
        Ok(val)
    } else {
        Err(format!("{flag} requires a value"))
    }
}

// ── IPC heartbeat source ────────────────────────────────────────────────
// NOTE: A coordinator system prompt will be injected as `Message::system()`
// once `heartbeat::dispatch_task()` supports system-message injection.
// Until then, the coordinator's role context is implicit in the task
// descriptions ("Process coordinator command ...") and tool definitions.

/// A `HeartbeatTaskSource` that reads pending IPC commands from the
/// coordinator inbox and formats them as heartbeat task messages.
///
/// On each `read_heartbeat_md()` call, this source:
/// 1. Reads pending commands from the inbox
/// 2. Formats non-shutdown commands as heartbeat task lines (one per command)
/// 3. Acknowledges inbox files (removes from inbox so they aren't re-read)
/// 4. Stores command_ids in `pending_command_ids` for the coordinator loop
///    to write real responses to the outbox after the LLM finishes
///
/// The outbox response is NOT written here — that would cause the gateway's
/// `poll_response()` to return a premature `{"status":"processing"}` before
/// the coordinator LLM has actually processed the command.
struct InboxHeartbeatSource {
    ipc: Arc<dyn CoordinatorIpc>,
    /// Command IDs from the last `read_heartbeat_md()` call.
    /// The coordinator loop reads these after `execute_heartbeat_tick()`
    /// to map agent results back to IPC responses.
    /// Order matches the task lines (and thus the `HeartbeatTaskResult` vec).
    pending_command_ids: std::sync::Mutex<Vec<String>>,
}

impl InboxHeartbeatSource {
    fn new(ipc: Arc<dyn CoordinatorIpc>) -> Self {
        Self {
            ipc,
            pending_command_ids: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Take the pending command IDs from the last tick (drains the vec).
    fn take_pending_command_ids(&self) -> Vec<String> {
        self.pending_command_ids
            .lock()
            .map(|mut ids| std::mem::take(&mut *ids))
            .unwrap_or_default()
    }
}

impl HeartbeatTaskSource for InboxHeartbeatSource {
    fn read_heartbeat_md(
        &self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = Result<Option<String>, crate::domain::error::DomainError>,
                > + Send
                + '_,
        >,
    > {
        Box::pin(async move {
            let commands = self.ipc.read_pending_commands().map_err(|e| {
                crate::domain::error::DomainError::Other(format!("read inbox: {e}"))
            })?;

            if commands.is_empty() {
                // No work — return None so the agent loop is NOT invoked.
                // This avoids ~1440 unnecessary LLM API calls per day.
                return Ok(None);
            }

            // Filter out shutdown commands — they are handled deterministically
            // in run_coordinator() before reaching the LLM.
            let non_shutdown: Vec<_> = commands.iter().filter(|c| c.action != "shutdown").collect();

            if non_shutdown.is_empty() {
                // Only shutdown commands — don't invoke LLM.
                return Ok(None);
            }

            // Store command IDs for post-processing (outbox response writing).
            let command_ids: Vec<String> =
                non_shutdown.iter().map(|c| c.command_id.clone()).collect();
            if let Ok(mut pending) = self.pending_command_ids.lock() {
                *pending = command_ids;
            }

            // Format each command as a heartbeat task line.
            let mut content = String::new();
            for cmd in &non_shutdown {
                let payload_str =
                    serde_json::to_string(&cmd.payload).unwrap_or_else(|_| "{}".to_string());
                content.push_str(&format!(
                    "- Process coordinator command (id={}): action={}, payload={}\n",
                    cmd.command_id, cmd.action, payload_str,
                ));
            }

            // Acknowledge inbox files — remove from inbox so they aren't
            // re-read on the next heartbeat tick. Do NOT write to outbox here;
            // the outbox response is written by the coordinator loop after
            // the LLM finishes processing.
            for cmd in &non_shutdown {
                if let Err(e) = self.ipc.acknowledge_command(&cmd.command_id) {
                    tracing::warn!(command_id = %cmd.command_id, "failed to ack: {e}");
                }
            }

            Ok(Some(content))
        })
    }
}

// ── Coordinator agent loop ──────────────────────────────────────────────

/// Handle the `quecto coordinator` subcommand.
///
/// Builds a full LLM-driven agent with restricted tools (`coding_job` +
/// `recall` only), then runs a heartbeat loop that polls the IPC inbox
/// and feeds commands to the agent. This is the production entry point
/// called from `cli/mod.rs`.
pub fn cmd_coordinator(
    ctx: &super::CliContext,
    args: &[String],
    stdout: &mut String,
    stderr: &mut String,
) -> i32 {
    let coord_args = match parse_coordinator_args(args) {
        Ok(a) => a,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    let base_dir = ctx.base_dir();
    let workspace = base_dir.join("workspace");

    // Initialize IPC.
    let ipc_dir = PathBuf::from(&coord_args.ipc_dir);
    let ipc = match FileCoordinatorIpc::new(&ipc_dir) {
        Ok(ipc) => Arc::new(ipc) as Arc<dyn CoordinatorIpc>,
        Err(e) => {
            stderr.push_str(&format!("coordinator: failed to init IPC: {e}\n"));
            return 1;
        }
    };

    // Write PID for liveness checks.
    let pid = std::process::id();
    if let Err(e) = ipc.write_pid(pid) {
        stderr.push_str(&format!("coordinator: failed to write PID: {e}\n"));
        return 1;
    }

    // Load config once — used for provider, registry settings, and agent config.
    let config = match load_config(&base_dir) {
        Ok(c) => c,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    // Build LLM provider.
    let provider = match super::agent::build_agent_provider(&config, &base_dir) {
        Ok(p) => p,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    // Build restricted tool registry: coding_job + recall only.
    // No exec/fs_read/fs_write/spawn — the coordinator should not have
    // arbitrary shell or filesystem access (security reviewer finding).
    let mut registry = ToolRegistryImpl::new();
    let lifecycle_driver = build_coding_lifecycle(&mut registry, &workspace, &base_dir);

    let session_key = "coordinator:main".to_string();
    let spill_store = Arc::new(FileContextSpillStore::new(base_dir.clone()));
    registry.register(Arc::new(RecallTool::new(
        spill_store.clone(),
        session_key.clone(),
    )));

    // Build agent with system prompt injected as the first message.
    let agent = AgentLoopImpl::new(AgentLoopConfig {
        provider,
        tool_registry: Box::new(registry),
        model: config.agents.defaults.model.clone(),
        max_tokens: config.agents.defaults.max_tokens,
        temperature: config.agents.defaults.temperature,
        spill_store: Some(spill_store),
        session_key,
        context_collapse_after_turns: config.agents.defaults.context_collapse_after_turns,
        max_context_tokens: config.agents.defaults.max_context_tokens,
    })
    .with_max_tool_iterations(config.agents.defaults.max_tool_iterations);

    let agent: Arc<dyn AgentLoop> = Arc::new(agent);

    stdout.push_str(&format!(
        "coordinator: ready (ipc_dir={}, heartbeat={}s, pid={})\n",
        coord_args.ipc_dir, coord_args.heartbeat_interval_secs, pid,
    ));

    // Run the coordinator event loop on a single-threaded runtime.
    // The coordinator processes inbox commands sequentially — no need for
    // a multi-threaded scheduler (performance reviewer finding).
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            stderr.push_str(&format!("coordinator: failed to create runtime: {e}\n"));
            return 1;
        }
    };

    rt.block_on(run_coordinator(CoordinatorRunContext {
        agent,
        ipc,
        lifecycle_driver,
        workspace,
        heartbeat_interval_secs: coord_args.heartbeat_interval_secs,
    }))
}

/// Load config from the base directory.
fn load_config(
    base_dir: &std::path::Path,
) -> Result<crate::infrastructure::config::Config, String> {
    let config_path = base_dir.join("config.json");
    if !config_path.exists() {
        return Err(format!(
            "config not found at {}\nrun 'quecto onboard' first",
            config_path.display()
        ));
    }
    let env_overrides: std::collections::HashMap<String, String> = std::env::vars()
        .filter(|(k, _)| k.starts_with("QUECTO_"))
        .collect();
    crate::infrastructure::config::Config::load_with_env(
        config_path.to_str().unwrap_or(""),
        &env_overrides,
    )
    .map_err(|e| format!("failed to load config: {e}"))
}

/// Handle shutdown commands deterministically (no LLM needed).
///
/// Acknowledges each shutdown command, writes a success response, and
/// returns `true` if at least one shutdown command was found.
fn handle_shutdown_commands(ipc: &dyn CoordinatorIpc) -> bool {
    let commands = match ipc.read_pending_commands() {
        Ok(cmds) => cmds,
        Err(_) => return false,
    };

    let shutdown_cmds: Vec<&CoordinatorIpcCommand> =
        commands.iter().filter(|c| c.action == "shutdown").collect();

    if shutdown_cmds.is_empty() {
        return false;
    }

    for cmd in &shutdown_cmds {
        let response = crate::domain::coding_ipc::CoordinatorIpcResponse {
            command_id: cmd.command_id.clone(),
            ok: true,
            body: Some(serde_json::json!({"status": "shutdown_acknowledged"})),
            error: None,
        };
        let _ = ipc.write_response(&response);
        let _ = ipc.acknowledge_command(&cmd.command_id);
    }

    true
}

/// Query active job count from the lifecycle driver.
fn active_job_count(driver: &SharedLifecycleDriver) -> u32 {
    match driver.lock() {
        Ok(guard) => {
            let list = guard
                .coordinator()
                .list(&crate::domain::coding_command::ListRequest { state_filter: None });
            list.jobs.iter().filter(|j| !j.state.is_terminal()).count() as u32
        }
        Err(_) => 0,
    }
}

/// Process inbox commands and write real responses to the outbox.
///
/// Runs `execute_heartbeat_tick()` on the inbox source, then maps each
/// result back to its command_id and writes the LLM response to the outbox.
async fn process_inbox_tick(
    inbox_source: &InboxHeartbeatSource,
    agent: &dyn crate::domain::agent::AgentLoop,
    ipc: &dyn CoordinatorIpc,
    timeout: std::time::Duration,
) {
    match heartbeat::execute_heartbeat_tick(inbox_source, agent, timeout).await {
        Ok(results) => {
            let command_ids = inbox_source.take_pending_command_ids();
            for (i, result) in results.iter().enumerate() {
                tracing::info!(
                    task = result.message.as_str(),
                    "coordinator heartbeat task completed"
                );
                if let Some(cmd_id) = command_ids.get(i) {
                    let resp = crate::domain::coding_ipc::CoordinatorIpcResponse {
                        command_id: cmd_id.clone(),
                        ok: true,
                        body: Some(serde_json::json!({
                            "status": "completed",
                            "response": result.response,
                        })),
                        error: None,
                    };
                    if let Err(e) = ipc.write_response(&resp) {
                        tracing::warn!(command_id = %cmd_id, "failed to write response: {e}");
                    }
                }
            }
        }
        Err(e) => {
            let command_ids = inbox_source.take_pending_command_ids();
            for cmd_id in &command_ids {
                let resp = crate::domain::coding_ipc::CoordinatorIpcResponse {
                    command_id: cmd_id.clone(),
                    ok: false,
                    body: None,
                    error: Some(format!("heartbeat tick failed: {e}")),
                };
                let _ = ipc.write_response(&resp);
            }
            tracing::error!(error = %e, "coordinator heartbeat tick failed");
        }
    }
}

/// Bundled arguments for `run_coordinator()` to stay within clippy's
/// argument limit.
struct CoordinatorRunContext {
    agent: Arc<dyn AgentLoop>,
    ipc: Arc<dyn CoordinatorIpc>,
    lifecycle_driver: SharedLifecycleDriver,
    workspace: PathBuf,
    heartbeat_interval_secs: u64,
}

/// Run the coordinator's async event loop until shutdown.
///
/// The loop runs a heartbeat that polls the IPC inbox and feeds commands
/// to the agent. On SIGINT/SIGTERM or a `shutdown` IPC command, the loop
/// exits gracefully.
async fn run_coordinator(ctx: CoordinatorRunContext) -> i32 {
    let CoordinatorRunContext {
        agent,
        ipc,
        lifecycle_driver,
        workspace,
        heartbeat_interval_secs,
    } = ctx;
    // Initialize tracing.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let interval = std::time::Duration::from_secs(heartbeat_interval_secs);
    let inbox_source = InboxHeartbeatSource::new(ipc.clone());
    // Timeout per heartbeat task: interval minus 10s margin, clamped to [30s, 300s].
    let timeout_secs = heartbeat_interval_secs.saturating_sub(10).clamp(30, 300);
    let timeout = std::time::Duration::from_secs(timeout_secs);

    tracing::info!(
        heartbeat_secs = heartbeat_interval_secs,
        timeout_secs = timeout_secs,
        "coordinator agent started"
    );

    // Also read workspace HEARTBEAT.md for any standing coordinator tasks.
    let workspace_source =
        crate::infrastructure::persistence::workspace_store::FileHeartbeatTaskSource::new(
            &workspace,
        );

    // Run heartbeat + signal handler concurrently.
    // NOTE: The system prompt is not injected as a `Message::system()` because
    // `execute_heartbeat_tick()` creates a fresh message vec per task. The
    // coordinator's role context is implicit in the task descriptions and
    // tool definitions. A future PR can add system-message injection to
    // `dispatch_task()` if richer context is needed.
    tokio::select! {
        _ = async {
            loop {
                // Check for shutdown commands deterministically (no LLM).
                if handle_shutdown_commands(&*ipc) {
                    tracing::info!("coordinator: shutdown command received via IPC");
                    return;
                }

                // Process inbox commands via the agent.
                tracing::debug!("coordinator heartbeat tick");

                process_inbox_tick(&inbox_source, &*agent, &*ipc, timeout).await;

                // Also run workspace heartbeat tasks if any.
                match heartbeat::execute_heartbeat_tick(&workspace_source, &*agent, timeout).await {
                    Ok(results) => {
                        for result in &results {
                            tracing::info!(
                                task = result.message.as_str(),
                                via_spawn = result.dispatched_via_spawn,
                                "coordinator workspace heartbeat task completed"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "coordinator workspace heartbeat failed");
                    }
                }

                // Write state snapshot with real active job count.
                let jobs = active_job_count(&lifecycle_driver);
                let state = crate::domain::coding_ipc::CoordinatorState {
                    alive: true,
                    active_jobs: jobs,
                    last_heartbeat: chrono::Utc::now().to_rfc3339(),
                    job_summary: serde_json::json!({}),
                };
                let _ = ipc.write_state(&state);

                tokio::time::sleep(interval).await;
            }
        } => {}
        _ = tokio::signal::ctrl_c() => {
            tracing::info!("coordinator: shutdown signal received");
        }
    }

    tracing::info!("coordinator agent stopped");
    0
}

/// Help text for the coordinator subcommand.
pub fn coordinator_help_text() -> &'static str {
    "  coordinator Run the coordinator agent (internal)\n"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_args_basic() {
        let args = vec!["--ipc-dir".into(), "/tmp/coord".into()];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.ipc_dir, "/tmp/coord");
        assert_eq!(
            parsed.heartbeat_interval_secs,
            DEFAULT_HEARTBEAT_INTERVAL_SECS
        );
    }

    #[test]
    fn test_parse_args_with_heartbeat_interval() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--heartbeat-interval".into(),
            "30".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.ipc_dir, "/tmp/coord");
        assert_eq!(parsed.heartbeat_interval_secs, 30);
    }

    #[test]
    fn test_parse_args_legacy_poll_interval() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--poll-interval-ms".into(),
            "60000".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.heartbeat_interval_secs, 60);
    }

    #[test]
    fn test_parse_args_legacy_poll_interval_minimum() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--poll-interval-ms".into(),
            "100".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        // 100ms / 1000 = 0, clamped to MIN_HEARTBEAT_INTERVAL_SECS (10)
        assert_eq!(parsed.heartbeat_interval_secs, MIN_HEARTBEAT_INTERVAL_SECS);
    }

    #[test]
    fn test_parse_args_heartbeat_interval_too_low() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--heartbeat-interval".into(),
            "5".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("must be >= 10"));
    }

    #[test]
    fn test_parse_args_heartbeat_interval_zero() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--heartbeat-interval".into(),
            "0".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("must be >= 10"));
    }

    #[test]
    fn test_parse_args_missing_ipc_dir() {
        let args = vec!["--heartbeat-interval".into(), "30".into()];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("missing required flag --ipc-dir"));
    }

    #[test]
    fn test_parse_args_unknown_flag() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--unknown-flag".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("unknown flag"));
    }

    #[test]
    fn test_parse_args_invalid_heartbeat_interval() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--heartbeat-interval".into(),
            "abc".into(),
        ];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("must be a number"));
    }

    #[test]
    fn test_parse_args_ipc_dir_missing_value() {
        let args = vec!["--ipc-dir".into()];
        let err = parse_coordinator_args(&args).unwrap_err();
        assert!(err.contains("requires a value"));
    }

    #[test]
    fn test_parse_args_heartbeat_at_minimum() {
        let args = vec![
            "--ipc-dir".into(),
            "/tmp/coord".into(),
            "--heartbeat-interval".into(),
            "10".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.heartbeat_interval_secs, 10);
    }
}
