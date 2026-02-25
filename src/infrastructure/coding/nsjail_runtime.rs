//! Real nsjail coding worker runtime.
//!
//! Implements `WorkerRuntime` by spawning `nsjail -- quecto worker ...`
//! as a child process with bidirectional JSON Lines IPC over
//! stdin/stdout and stderr capture.
//!
//! Supports two modes:
//! - **Mock mode** (default): Assigns synthetic PIDs for command-construction
//!   tests. No real process is spawned.
//! - **Real mode** (`command_override` set): Spawns a real child process using
//!   `std::process::Command`, pipes stdin/stdout/stderr, and tracks the OS PID.

use std::collections::{HashMap, VecDeque};
use std::io::{BufRead, BufReader, Read, Write};
use std::mem;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;

use crate::domain::coding_event::EventEnvelope;
use crate::domain::coding_ports::{
    WorkerEnvVar, WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};

/// Maximum captured stderr size (1 MiB), matching ExecTool's limit.
const MAX_STDERR_BYTES: usize = 1024 * 1024;

/// Maximum events buffered per worker before oldest are dropped.
const MAX_WORKER_EVENTS: usize = 5_000;

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
struct WorkerProcessState {
    status: WorkerStatus,
    events: VecDeque<WorkerEvent>,
    stderr_buf: String,
    launch_params: LaunchParams,
    /// Real child process (only present in real spawn mode).
    child: Option<Child>,
    /// Shared event buffer populated by the stdout reader thread.
    shared_events: Option<Arc<Mutex<VecDeque<WorkerEvent>>>>,
    /// Shared stderr buffer populated by the stderr reader thread.
    shared_stderr: Option<Arc<Mutex<String>>>,
}

impl std::fmt::Debug for WorkerProcessState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("WorkerProcessState")
            .field("status", &self.status)
            .field("events_len", &self.events.len())
            .field("stderr_len", &self.stderr_buf.len())
            .field("has_child", &self.child.is_some())
            .finish()
    }
}

// ── NsjailWorkerRuntime ─────────────────────────────────────────────────

/// Configuration for the nsjail runtime.
#[derive(Debug, Clone)]
pub struct NsjailRuntimeConfig {
    /// Path to the nsjail binary.
    pub nsjail_binary: String,
    /// Path to the quecto binary (resolved at construction).
    pub quecto_binary: String,
    /// When set, spawn this command directly instead of nsjail.
    /// Used for testing real process spawn without nsjail capabilities.
    /// The command receives the worker launch config via arguments.
    pub command_override: Option<Vec<String>>,
}

impl Default for NsjailRuntimeConfig {
    fn default() -> Self {
        Self {
            nsjail_binary: "nsjail".to_string(),
            quecto_binary: resolve_quecto_binary(),
            command_override: None,
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

impl Drop for NsjailWorkerRuntime {
    fn drop(&mut self) {
        for w in self.workers.values_mut() {
            if let Some(ref mut child) = w.child {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
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

        if let Some(cmd_override) = &self.config.command_override {
            return self.launch_real(config, cmd_override.clone());
        }
        self.launch_mock(config)
    }

    fn send_command(&mut self, pid: u32, command: &str) -> Result<(), String> {
        let w = self
            .workers
            .get_mut(&pid)
            .ok_or_else(|| "unknown worker PID".to_string())?;
        if w.status != WorkerStatus::Running {
            return Err("worker is not running".to_string());
        }
        if let Some(ref mut child) = w.child {
            if let Some(ref mut stdin) = child.stdin {
                stdin
                    .write_all(command.as_bytes())
                    .map_err(|e| format!("stdin write: {e}"))?;
                stdin
                    .write_all(b"\n")
                    .map_err(|e| format!("stdin newline: {e}"))?;
                stdin.flush().map_err(|e| format!("stdin flush: {e}"))?;
            }
        }
        Ok(())
    }

    fn read_event(&mut self, pid: u32) -> Option<WorkerEvent> {
        let w = self.workers.get_mut(&pid)?;
        // First drain any locally buffered events (mock mode)
        if let Some(ev) = w.events.pop_front() {
            return Some(ev);
        }
        // Drain from shared buffer (real spawn mode)
        if let Some(ref shared) = w.shared_events {
            if let Ok(mut q) = shared.lock() {
                return q.pop_front();
            }
        }
        None
    }

    fn read_stderr(&mut self, pid: u32) -> String {
        let w = match self.workers.get_mut(&pid) {
            Some(w) => w,
            None => return String::new(),
        };
        // Drain from shared stderr buffer (real spawn mode), respecting cap
        if let Some(ref shared) = w.shared_stderr {
            if let Ok(mut b) = shared.lock() {
                let remaining = MAX_STDERR_BYTES.saturating_sub(w.stderr_buf.len());
                if remaining > 0 && !b.is_empty() {
                    let safe = floor_char_boundary(&b, remaining);
                    w.stderr_buf.push_str(&b[..safe]);
                }
                b.clear();
            }
        }
        mem::take(&mut w.stderr_buf)
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
        let w = self
            .workers
            .get_mut(&pid)
            .ok_or_else(|| "unknown worker PID".to_string())?;
        if w.status != WorkerStatus::Running {
            return Ok(());
        }
        if let Some(ref mut child) = w.child {
            let _ = child.kill();
            let _ = child.wait();
        }
        w.status = WorkerStatus::Killed {
            reason: "killed by coordinator".to_string(),
        };
        Ok(())
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
        if let Some(mut w) = self.workers.remove(&pid) {
            if let Some(ref mut child) = w.child {
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }
}

impl NsjailWorkerRuntime {
    /// Mock launch: assigns a synthetic PID without spawning a process.
    fn launch_mock(&mut self, config: &WorkerLaunchConfig) -> Result<u32, String> {
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
                child: None,
                shared_events: None,
                shared_stderr: None,
            },
        );

        Ok(pid)
    }

    /// Real launch: spawns a child process with piped I/O.
    fn launch_real(
        &mut self,
        config: &WorkerLaunchConfig,
        cmd_override: Vec<String>,
    ) -> Result<u32, String> {
        let (program, args) = cmd_override
            .split_first()
            .ok_or_else(|| "command_override is empty".to_string())?;

        let worker_env = build_worker_env(config);
        let mut cmd = Command::new(program);
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env_clear();
        for var in &worker_env {
            cmd.env(&var.name, &var.value);
        }
        let mut child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;

        let pid = child.id();
        let run_id = format!("run_{pid}");
        let job_id = format!("job_{pid}");

        // Build args for inspection
        let parts = build_nsjail_worker_args(&self.config, config, &run_id, &job_id);
        let mut full_args = parts.nsjail_args.clone();
        full_args.extend(parts.worker_args.clone());
        self.last_nsjail_args = Some(full_args);
        self.last_worker_args = Some(parts.worker_args);
        self.last_launch_params = Some(LaunchParams {
            run_id: run_id.clone(),
            job_id: job_id.clone(),
        });

        // Spawn background stdout reader thread
        let shared_events = Arc::new(Mutex::new(VecDeque::<WorkerEvent>::new()));
        if let Some(stdout) = child.stdout.take() {
            let events = Arc::clone(&shared_events);
            thread::spawn(move || {
                read_stdout_lines(stdout, &events);
            });
        }

        // Spawn background stderr reader thread
        let shared_stderr = Arc::new(Mutex::new(String::new()));
        if let Some(stderr) = child.stderr.take() {
            let stderr_buf = Arc::clone(&shared_stderr);
            thread::spawn(move || {
                read_stderr_to_buffer(stderr, &stderr_buf);
            });
        }

        self.workers.insert(
            pid,
            WorkerProcessState {
                status: WorkerStatus::Running,
                events: VecDeque::new(),
                stderr_buf: String::new(),
                launch_params: LaunchParams { run_id, job_id },
                child: Some(child),
                shared_events: Some(shared_events),
                shared_stderr: Some(shared_stderr),
            },
        );

        Ok(pid)
    }

    /// Poll a real child process and update its status if exited.
    pub fn poll_status(&mut self, pid: u32) {
        if let Some(w) = self.workers.get_mut(&pid) {
            if w.status != WorkerStatus::Running {
                return;
            }
            if let Some(ref mut child) = w.child {
                match child.try_wait() {
                    Ok(Some(exit_status)) => {
                        let code = exit_status.code().unwrap_or(-1);
                        w.status = WorkerStatus::Exited { status: code };
                    }
                    Ok(None) => {} // still running
                    Err(_) => {}
                }
            }
        }
    }

    /// Wait for a real child process to exit (blocking).
    pub fn wait_for_exit(&mut self, pid: u32) {
        if let Some(w) = self.workers.get_mut(&pid) {
            if let Some(ref mut child) = w.child {
                match child.wait() {
                    Ok(exit_status) => {
                        let code = exit_status.code().unwrap_or(-1);
                        w.status = WorkerStatus::Exited { status: code };
                    }
                    Err(_) => {
                        w.status = WorkerStatus::Killed {
                            reason: "wait failed".to_string(),
                        };
                    }
                }
            }
        }
    }
}

/// Background thread: read lines from child stdout, parse as events.
fn read_stdout_lines(stdout: std::process::ChildStdout, events: &Mutex<VecDeque<WorkerEvent>>) {
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        let trimmed = line.trim().to_string();
        if trimmed.is_empty() {
            continue;
        }
        let event = match serde_json::from_str::<EventEnvelope>(&trimmed) {
            Ok(envelope) => WorkerEvent::Valid(envelope),
            Err(_) => WorkerEvent::Malformed { raw: trimmed },
        };
        if let Ok(mut q) = events.lock() {
            if q.len() >= MAX_WORKER_EVENTS {
                q.pop_front(); // drop oldest to stay within cap
            }
            q.push_back(event);
        }
    }
}

/// Background thread: read stderr and accumulate into a shared buffer (capped).
fn read_stderr_to_buffer(mut stderr: std::process::ChildStderr, buf: &Mutex<String>) {
    let mut tmp = [0u8; 8192];
    loop {
        // Check cap before reading to avoid unnecessary I/O
        let at_cap = buf
            .lock()
            .map(|b| b.len() >= MAX_STDERR_BYTES)
            .unwrap_or(true);
        if at_cap {
            // Cap reached — drain remaining stderr to prevent pipe blocking
            loop {
                match stderr.read(&mut tmp) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => continue,
                }
            }
        }
        match stderr.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&tmp[..n]);
                if let Ok(mut b) = buf.lock() {
                    let remaining = MAX_STDERR_BYTES.saturating_sub(b.len());
                    if remaining > 0 {
                        let safe = floor_char_boundary(&chunk, remaining);
                        b.push_str(&chunk[..safe]);
                    }
                }
            }
            Err(_) => break,
        }
    }
}

#[cfg(test)]
#[path = "nsjail_runtime_tests.rs"]
mod tests;
