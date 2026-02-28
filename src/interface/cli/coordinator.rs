//! `quecto coordinator` subcommand — LLM-driven coordinator agent.
//!
//! Runs a long-lived agent process that manages coding jobs autonomously.
//! The coordinator has its own LLM provider, tool registry (with `coding_job`
//! in inline mode), and a heartbeat loop that polls the IPC inbox for
//! commands from the main agent.
//!
//! Architecture: this is a mini-gateway — same pattern as `Gateway::run()`
//! but without Telegram channels. The heartbeat reads pending IPC commands,
//! converts them to agent messages, runs the agent loop, and writes
//! responses back to the IPC outbox.

use std::path::PathBuf;
use std::sync::Arc;

use crate::application::agent_loop::{AgentLoopConfig, AgentLoopImpl};
use crate::application::heartbeat;
use crate::domain::agent::AgentLoop;
use crate::domain::coding_ipc::CoordinatorIpc;
use crate::domain::workspace::HeartbeatTaskSource;
use crate::infrastructure::coding::coordinator_ipc::FileCoordinatorIpc;
use crate::infrastructure::persistence::context_spill::FileContextSpillStore;
use crate::infrastructure::security::sandbox::Sandbox;
use crate::infrastructure::tools::recall::RecallTool;
use crate::infrastructure::tools::registry::ToolRegistryImpl;

use crate::interface::shared::build_coding_lifecycle;

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
                heartbeat_interval_secs = Some(n);
            }
            // Legacy flag — accept but convert to heartbeat interval.
            "--poll-interval-ms" => {
                let val = require_next(args, &mut i, "--poll-interval-ms")?;
                let ms: u64 = val
                    .parse()
                    .map_err(|_| format!("--poll-interval-ms must be a number, got '{val}'"))?;
                // Convert ms to seconds, minimum 1s.
                heartbeat_interval_secs = Some((ms / 1000).max(1));
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

// ── Coordinator system prompt ───────────────────────────────────────────

/// System prompt that defines the coordinator agent's role.
const COORDINATOR_SYSTEM_PROMPT: &str = "\
You are a coding job coordinator. You manage parallel coding workers for \
long-running coding sessions. You have the `coding_job` tool available to \
create repos, run jobs, check status, cancel, and cleanup.

When you receive a command from the main agent via your heartbeat inbox, \
execute it using the coding_job tool and return the result. You can reason \
about stuck workers, retry failed jobs, and make autonomous decisions about \
the coding workflow.

On each heartbeat tick you should:
1. Check for pending commands in the inbox and process them.
2. Check the status of all running jobs and handle any that are stuck or failed.
3. Report results back via the outbox.

You are a long-lived process — you run continuously until shutdown is requested.";

// ── IPC heartbeat source ────────────────────────────────────────────────

/// A `HeartbeatTaskSource` that reads pending IPC commands from the
/// coordinator inbox and formats them as heartbeat task messages.
///
/// Each pending command becomes a heartbeat task message that the agent
/// processes through its LLM loop. The agent uses the `coding_job` tool
/// to execute the command and the response is captured.
///
/// The system prompt is prepended to each task so the agent has context
/// about its role on every invocation.
struct InboxHeartbeatSource {
    ipc: Arc<dyn CoordinatorIpc>,
    system_context: String,
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
                // Return a status check task even when inbox is empty so
                // the coordinator proactively monitors running jobs.
                return Ok(Some(format!(
                    "- {context} Check status of all running coding jobs and report any issues.\n",
                    context = self.system_context
                )));
            }

            // Format each command as a heartbeat task line.
            let mut content = String::new();
            for cmd in &commands {
                let payload_str =
                    serde_json::to_string(&cmd.payload).unwrap_or_else(|_| "{}".to_string());
                content.push_str(&format!(
                    "- {context} Process coordinator command (id={}): action={}, payload={}\n",
                    cmd.command_id,
                    cmd.action,
                    payload_str,
                    context = self.system_context
                ));

                // Write a placeholder response immediately so the main agent
                // doesn't time out waiting. The agent will update it after
                // processing.
                let ack_response = crate::domain::coding_ipc::CoordinatorIpcResponse {
                    command_id: cmd.command_id.clone(),
                    ok: true,
                    body: Some(serde_json::json!({"status": "processing"})),
                    error: None,
                };
                if let Err(e) = self.ipc.write_response(&ack_response) {
                    tracing::warn!(command_id = %cmd.command_id, "failed to ack: {e}");
                }
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
/// Builds a full LLM-driven agent with `coding_job` tool (inline mode),
/// then runs a heartbeat loop that polls the IPC inbox and feeds commands
/// to the agent. This is the production entry point called from `cli/mod.rs`.
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

    // Build LLM provider.
    let provider = match super::agent::build_agent_provider(
        &match load_config(&base_dir) {
            Ok(c) => c,
            Err(e) => {
                stderr.push_str(&format!("coordinator: {e}\n"));
                return 1;
            }
        },
        &base_dir,
    ) {
        Ok(p) => p,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    let config = match load_config(&base_dir) {
        Ok(c) => c,
        Err(e) => {
            stderr.push_str(&format!("coordinator: {e}\n"));
            return 1;
        }
    };

    // Build tool registry with coding_job in inline mode.
    let sandbox = Sandbox::new(Some(workspace.clone()), true);
    let exec_settings = ToolRegistryImpl::exec_registry_settings_from_config(&config);
    let mut registry = ToolRegistryImpl::with_core_tools_and_exec_settings(
        workspace.clone(),
        sandbox,
        exec_settings,
    );
    let _lifecycle_driver = build_coding_lifecycle(&mut registry, &workspace, &base_dir);

    let session_key = "coordinator:main".to_string();
    let spill_store = Arc::new(FileContextSpillStore::new(base_dir.clone()));
    registry.register(Arc::new(RecallTool::new(
        spill_store.clone(),
        session_key.clone(),
    )));

    // Build agent.
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

    // Run the coordinator event loop.
    let rt = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            stderr.push_str(&format!("coordinator: failed to create runtime: {e}\n"));
            return 1;
        }
    };

    rt.block_on(run_coordinator(
        agent,
        ipc,
        workspace,
        coord_args.heartbeat_interval_secs,
    ))
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

/// Run the coordinator's async event loop until shutdown.
///
/// The loop runs a heartbeat that polls the IPC inbox and feeds commands
/// to the agent. On SIGINT/SIGTERM, the loop exits gracefully.
async fn run_coordinator(
    agent: Arc<dyn AgentLoop>,
    ipc: Arc<dyn CoordinatorIpc>,
    workspace: PathBuf,
    heartbeat_interval_secs: u64,
) -> i32 {
    // Initialize tracing.
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .try_init();

    let interval = std::time::Duration::from_secs(heartbeat_interval_secs);
    let inbox_source = InboxHeartbeatSource {
        ipc: ipc.clone(),
        system_context: COORDINATOR_SYSTEM_PROMPT.to_string(),
    };
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
    tokio::select! {
        _ = async {
            loop {
                // Process inbox commands via the agent.
                tracing::debug!("coordinator heartbeat tick");
                match heartbeat::execute_heartbeat_tick(&inbox_source, &*agent, timeout).await {
                    Ok(results) => {
                        for result in &results {
                            tracing::info!(
                                task = result.message.as_str(),
                                "coordinator heartbeat task completed"
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "coordinator heartbeat tick failed");
                    }
                }

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

                // Write state snapshot.
                let state = crate::domain::coding_ipc::CoordinatorState {
                    alive: true,
                    active_jobs: 0, // TODO: query from lifecycle driver
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
            "5000".into(),
        ];
        let parsed = parse_coordinator_args(&args).unwrap();
        assert_eq!(parsed.heartbeat_interval_secs, 5);
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
        // 100ms / 1000 = 0, clamped to 1
        assert_eq!(parsed.heartbeat_interval_secs, 1);
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
    fn test_system_prompt_is_non_empty() {
        assert!(!COORDINATOR_SYSTEM_PROMPT.is_empty());
        assert!(COORDINATOR_SYSTEM_PROMPT.contains("coding job coordinator"));
    }
}
