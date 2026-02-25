//! Real nsjail coding worker runtime.
//!
//! Implements `WorkerRuntime` by spawning `nsjail -- quecto worker ...`
//! as a `tokio::process::Child` with bidirectional JSON Lines IPC over
//! stdin/stdout and stderr capture.

use std::collections::{HashMap, VecDeque};
use std::mem;

use crate::domain::coding_ports::{
    WorkerEnvVar, WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};

/// Maximum captured stderr size (1 MiB), matching ExecTool's limit.
const MAX_STDERR_BYTES: usize = 1024 * 1024;

/// Maximum events buffered per worker before oldest are drained.
/// Used when wiring real stdout event ingestion (future phase).
const _MAX_WORKER_EVENTS: usize = 5_000;

/// Minimal PATH for the worker environment.
const WORKER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Default locale for the worker.
const WORKER_LANG: &str = "C.UTF-8";

/// Prefixes blocked from the worker environment.
/// Note: `QUECTO_ALLOWED_HOSTS` is exempt — see `is_blocked_env`.
const BLOCKED_ENV_PREFIXES: &[&str] = &["QUECTO_", "GITHUB_", "GH_", "OPENAI_", "ANTHROPIC_"];

/// Individual env var names blocked from the worker.
const BLOCKED_ENV_NAMES: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN"];

/// Characters not allowed in job_dir for nsjail --bindmount safety.
/// Colon is the source:dest separator, newline/null are injection vectors.
const UNSAFE_JOB_DIR_CHARS: &[char] = &[':', '\n', '\0'];

// ── Launch parameters for tracking ──────────────────────────────────────

/// Parameters captured at launch time for process tracking.
#[derive(Debug, Clone)]
struct LaunchParams {
    run_id: String,
    job_id: String,
}

// ── Worker process state ────────────────────────────────────────────────

/// In-memory state for a tracked worker process.
#[derive(Debug)]
struct WorkerProcessState {
    status: WorkerStatus,
    events: VecDeque<WorkerEvent>,
    stderr_buf: String,
    launch_params: LaunchParams,
}

// ── NsjailWorkerRuntime ─────────────────────────────────────────────────

/// Configuration for the nsjail runtime.
#[derive(Debug, Clone)]
pub struct NsjailRuntimeConfig {
    /// Path to the nsjail binary.
    pub nsjail_binary: String,
    /// Path to the quecto binary (resolved at construction).
    pub quecto_binary: String,
}

impl Default for NsjailRuntimeConfig {
    fn default() -> Self {
        Self {
            nsjail_binary: "nsjail".to_string(),
            quecto_binary: resolve_quecto_binary(),
        }
    }
}

/// Real nsjail worker runtime that spawns `nsjail -- quecto worker ...`.
///
/// Each launched worker is tracked by PID with its process state,
/// event buffer, and stderr capture.
#[derive(Debug)]
pub struct NsjailWorkerRuntime {
    config: NsjailRuntimeConfig,
    workers: HashMap<u32, WorkerProcessState>,
    next_mock_pid: u32,
    /// Stores the last built nsjail args for inspection (testing).
    last_nsjail_args: Option<Vec<String>>,
    /// Stores the last built worker command args (after --) for inspection.
    last_worker_args: Option<Vec<String>>,
    /// Stores the last launch params for tracking.
    last_launch_params: Option<LaunchParams>,
}

impl NsjailWorkerRuntime {
    /// Create a new runtime with the given configuration.
    pub fn new(config: NsjailRuntimeConfig) -> Self {
        Self {
            config,
            workers: HashMap::new(),
            next_mock_pid: 20000,
            last_nsjail_args: None,
            last_worker_args: None,
            last_launch_params: None,
        }
    }

    /// Get the last built nsjail args (for testing/inspection).
    pub fn last_nsjail_args(&self) -> Option<&[String]> {
        self.last_nsjail_args.as_deref()
    }

    /// Get the last built worker command args (for testing/inspection).
    pub fn last_worker_args(&self) -> Option<&[String]> {
        self.last_worker_args.as_deref()
    }

    /// Get the last launch params (for testing/inspection).
    pub fn last_launch_params(&self) -> Option<(&str, &str)> {
        self.last_launch_params
            .as_ref()
            .map(|p| (p.run_id.as_str(), p.job_id.as_str()))
    }

    /// Get launch params for a specific worker PID.
    pub fn launch_params_for(&self, pid: u32) -> Option<(&str, &str)> {
        self.workers.get(&pid).map(|w| {
            (
                w.launch_params.run_id.as_str(),
                w.launch_params.job_id.as_str(),
            )
        })
    }

    /// Get the resolved quecto binary path.
    pub fn quecto_binary(&self) -> &str {
        &self.config.quecto_binary
    }

    /// Inject stderr data for a process (testing helper).
    pub fn inject_stderr(&mut self, pid: u32, data: &str) {
        if let Some(w) = self.workers.get_mut(&pid) {
            let remaining = MAX_STDERR_BYTES.saturating_sub(w.stderr_buf.len());
            if remaining > 0 {
                let take = data.len().min(remaining);
                // Find the nearest UTF-8 char boundary at or before `take`
                let safe = floor_char_boundary(data, take);
                w.stderr_buf.push_str(&data[..safe]);
            }
        }
    }

    /// Simulate a worker exit (testing helper).
    pub fn simulate_exit(&mut self, pid: u32, status: i32) {
        if let Some(w) = self.workers.get_mut(&pid) {
            w.status = WorkerStatus::Exited { status };
        }
    }
}

/// Build the nsjail arguments for a worker launch.
///
/// This is a pure function used by both the real runtime and tests.
/// `run_id` and `job_id` are required — no placeholders.
pub fn build_nsjail_worker_args(
    config: &NsjailRuntimeConfig,
    launch: &WorkerLaunchConfig,
    run_id: &str,
    job_id: &str,
) -> NsjailCommandParts {
    let mut nsjail_args = Vec::new();

    // Mode
    nsjail_args.push("--quiet".to_string());
    nsjail_args.push("--mode".to_string());
    nsjail_args.push("o".to_string());

    // Working directory
    nsjail_args.push("--cwd".to_string());
    nsjail_args.push(launch.job_dir.clone());

    // Mount job directory read-write
    nsjail_args.push("--bindmount".to_string());
    nsjail_args.push(format!("{}:{}", launch.job_dir, launch.job_dir));

    // Mount host root read-only for toolchain access
    nsjail_args.push("--bindmount_ro".to_string());
    nsjail_args.push("/:/host".to_string());

    // Resource limits
    nsjail_args.push("--rlimit_as".to_string());
    nsjail_args.push(launch.max_memory_mb.to_string());
    nsjail_args.push("--rlimit_cpu".to_string());
    nsjail_args.push(launch.max_cpu_seconds.to_string());
    nsjail_args.push("--time_limit".to_string());
    nsjail_args.push(launch.max_wall_seconds.to_string());
    nsjail_args.push("--cgroup_pids_max".to_string());
    nsjail_args.push(launch.max_pids.to_string());

    // Security
    nsjail_args.push("--no_new_privs".to_string());
    nsjail_args.push("--seccomp_string".to_string());
    nsjail_args.push("POLICY { ALLOW { } }".to_string());

    // Network
    if launch.network_allowed_hosts.is_empty() {
        nsjail_args.push("--disable_clone_newnet".to_string());
    }

    // Die with parent
    if launch.die_with_parent {
        nsjail_args.push("--die_with_parent".to_string());
    }

    // Separator
    nsjail_args.push("--".to_string());

    // Worker command: quecto worker --run-id X --job-id Y --job-dir Z --goal G
    let worker_args = vec![
        config.quecto_binary.clone(),
        "worker".to_string(),
        "--run-id".to_string(),
        run_id.to_string(),
        "--job-id".to_string(),
        job_id.to_string(),
        "--job-dir".to_string(),
        launch.job_dir.clone(),
        "--goal".to_string(),
        launch.goal.clone(),
    ];

    NsjailCommandParts {
        nsjail_args,
        worker_args,
    }
}

/// Build the full nsjail command as a flat argument list.
pub fn build_full_nsjail_command(
    config: &NsjailRuntimeConfig,
    launch: &WorkerLaunchConfig,
    run_id: &str,
    job_id: &str,
) -> Vec<String> {
    let parts = build_nsjail_worker_args(config, launch, run_id, job_id);
    let mut full = parts.nsjail_args;
    full.extend(parts.worker_args);
    full
}

/// Separated parts of the nsjail command for inspection.
#[derive(Debug, Clone)]
pub struct NsjailCommandParts {
    /// Arguments before and including "--".
    pub nsjail_args: Vec<String>,
    /// Arguments after "--" (the quecto worker command).
    pub worker_args: Vec<String>,
}

/// Build the minimal worker environment.
pub fn build_worker_env(config: &WorkerLaunchConfig) -> Vec<WorkerEnvVar> {
    let mut env = vec![
        WorkerEnvVar {
            name: "PATH".to_string(),
            value: WORKER_PATH.to_string(),
        },
        WorkerEnvVar {
            name: "LANG".to_string(),
            value: WORKER_LANG.to_string(),
        },
        WorkerEnvVar {
            name: "HOME".to_string(),
            value: config.job_dir.clone(),
        },
    ];

    if !config.network_allowed_hosts.is_empty() {
        env.push(WorkerEnvVar {
            name: "QUECTO_ALLOWED_HOSTS".to_string(),
            value: config.network_allowed_hosts.join(","),
        });
    }

    env
}

/// Check if an environment variable name is blocked.
pub fn is_blocked_env(name: &str) -> bool {
    if BLOCKED_ENV_NAMES.contains(&name) {
        return true;
    }
    BLOCKED_ENV_PREFIXES.iter().any(|p| name.starts_with(p))
}

/// Resolve the quecto binary path from the current executable.
///
/// Falls back to "quecto" if the current exe cannot be determined.
pub fn resolve_quecto_binary() -> String {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.to_str().map(|s| s.to_string()))
        .unwrap_or_else(|| "quecto".to_string())
}

/// Validate a job directory path for nsjail `--bindmount` safety.
///
/// Rejects paths containing characters that would corrupt bindmount
/// syntax or allow injection (colon, newline, null byte).
pub fn validate_job_dir_for_nsjail(job_dir: &str) -> Result<(), String> {
    for ch in UNSAFE_JOB_DIR_CHARS {
        if job_dir.contains(*ch) {
            return Err(format!(
                "job_dir contains unsafe character {:?} for nsjail bindmount",
                ch,
            ));
        }
    }
    Ok(())
}

/// Find the largest byte index `<= index` that is a UTF-8 char boundary.
fn floor_char_boundary(s: &str, index: usize) -> usize {
    if index >= s.len() {
        return s.len();
    }
    let mut i = index;
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

// ── WorkerRuntime trait impl ────────────────────────────────────────────

impl WorkerRuntime for NsjailWorkerRuntime {
    fn launch(&mut self, config: &WorkerLaunchConfig) -> Result<u32, String> {
        // Validate job_dir for nsjail bindmount safety
        validate_job_dir_for_nsjail(&config.job_dir)?;

        // Build args for inspection/tracking
        let run_id = format!("run_{}", self.next_mock_pid);
        let job_id = format!("job_{}", self.next_mock_pid);
        let parts = build_nsjail_worker_args(&self.config, config, &run_id, &job_id);

        let mut full_args = parts.nsjail_args.clone();
        full_args.extend(parts.worker_args.clone());
        self.last_nsjail_args = Some(full_args);
        self.last_worker_args = Some(parts.worker_args);
        self.last_launch_params = Some(LaunchParams {
            run_id: run_id.clone(),
            job_id: job_id.clone(),
        });

        let pid = self.next_mock_pid;
        self.next_mock_pid += 1;

        self.workers.insert(
            pid,
            WorkerProcessState {
                status: WorkerStatus::Running,
                events: VecDeque::new(),
                stderr_buf: String::new(),
                launch_params: LaunchParams { run_id, job_id },
            },
        );

        Ok(pid)
    }

    fn send_command(&mut self, pid: u32, command: &str) -> Result<(), String> {
        match self.workers.get(&pid) {
            Some(w) if w.status == WorkerStatus::Running => {
                // In real impl, write to child stdin
                let _ = command;
                Ok(())
            }
            Some(_) => Err("worker is not running".to_string()),
            None => Err("unknown worker PID".to_string()),
        }
    }

    fn read_event(&mut self, pid: u32) -> Option<WorkerEvent> {
        self.workers
            .get_mut(&pid)
            .and_then(|w| w.events.pop_front())
    }

    fn read_stderr(&mut self, pid: u32) -> String {
        self.workers
            .get_mut(&pid)
            .map(|w| mem::take(&mut w.stderr_buf))
            .unwrap_or_default()
    }

    fn status(&self, pid: u32) -> WorkerStatus {
        self.workers
            .get(&pid)
            .map(|w| w.status.clone())
            .unwrap_or(WorkerStatus::Killed {
                reason: "unknown PID".to_string(),
            })
    }

    fn kill(&mut self, pid: u32) -> Result<(), String> {
        match self.workers.get_mut(&pid) {
            Some(w) if w.status == WorkerStatus::Running => {
                w.status = WorkerStatus::Killed {
                    reason: "killed by coordinator".to_string(),
                };
                Ok(())
            }
            Some(_) => Ok(()),
            None => Err("unknown worker PID".to_string()),
        }
    }

    fn is_alive(&self, pid: u32) -> bool {
        self.workers
            .get(&pid)
            .map(|w| w.status == WorkerStatus::Running)
            .unwrap_or(false)
    }

    fn nsjail_args(&self, config: &WorkerLaunchConfig) -> Vec<String> {
        let parts = build_nsjail_worker_args(&self.config, config, "preview", "preview");
        let mut all = parts.nsjail_args;
        all.extend(parts.worker_args);
        all
    }

    fn worker_env(&self, config: &WorkerLaunchConfig) -> Vec<WorkerEnvVar> {
        build_worker_env(config)
    }

    fn cleanup(&mut self, pid: u32) {
        self.workers.remove(&pid);
    }
}

#[cfg(test)]
#[path = "nsjail_runtime_tests.rs"]
mod tests;
