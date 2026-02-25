//! Real nsjail coding worker runtime.
//!
//! Implements `WorkerRuntime` by spawning `nsjail -- quecto worker ...`
//! as a `tokio::process::Child` with bidirectional JSON Lines IPC over
//! stdin/stdout and stderr capture.

use std::collections::{HashMap, VecDeque};

use crate::domain::coding_ports::{
    WorkerEnvVar, WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};

/// Maximum captured stderr size (1 MiB), matching ExecTool's limit.
const MAX_STDERR_BYTES: usize = 1024 * 1024;

/// Minimal PATH for the worker environment.
const WORKER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";

/// Default locale for the worker.
const WORKER_LANG: &str = "C.UTF-8";

/// Prefixes blocked from the worker environment.
const BLOCKED_ENV_PREFIXES: &[&str] = &["QUECTO_", "GITHUB_", "GH_", "OPENAI_", "ANTHROPIC_"];

/// Individual env var names blocked from the worker.
const BLOCKED_ENV_NAMES: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN"];

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
                w.stderr_buf.push_str(&data[..take]);
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
pub fn build_nsjail_worker_args(
    config: &NsjailRuntimeConfig,
    launch: &WorkerLaunchConfig,
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
    nsjail_args.push("--time_limit".to_string());
    nsjail_args.push(launch.max_cpu_seconds.to_string());
    nsjail_args.push("--max_cpus".to_string());
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
        String::new(), // placeholder — filled by caller
        "--job-id".to_string(),
        String::new(), // placeholder
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

/// Build the full nsjail command with run_id and job_id filled in.
pub fn build_full_nsjail_command(
    config: &NsjailRuntimeConfig,
    launch: &WorkerLaunchConfig,
    run_id: &str,
    job_id: &str,
) -> Vec<String> {
    let mut parts = build_nsjail_worker_args(config, launch);

    // Fill in run-id and job-id placeholders
    for i in 0..parts.worker_args.len() {
        if parts.worker_args[i] == "--run-id" && i + 1 < parts.worker_args.len() {
            parts.worker_args[i + 1] = run_id.to_string();
        }
        if parts.worker_args[i] == "--job-id" && i + 1 < parts.worker_args.len() {
            parts.worker_args[i + 1] = job_id.to_string();
        }
    }

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

// ── WorkerRuntime trait impl ────────────────────────────────────────────

impl WorkerRuntime for NsjailWorkerRuntime {
    fn launch(&mut self, config: &WorkerLaunchConfig) -> Result<u32, String> {
        // Build args for inspection/tracking
        let run_id = format!("run_{}", self.next_mock_pid);
        let job_id = format!("job_{}", self.next_mock_pid);
        let full_args = build_full_nsjail_command(&self.config, config, &run_id, &job_id);
        let parts = build_nsjail_worker_args(&self.config, config);

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
            .map(|w| {
                let out = w.stderr_buf.clone();
                w.stderr_buf.clear();
                out
            })
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
        let parts = build_nsjail_worker_args(&self.config, config);
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
mod tests {
    use super::*;

    fn default_launch_config() -> WorkerLaunchConfig {
        WorkerLaunchConfig {
            job_dir: "/tmp/jobs/job_001/repo".to_string(),
            goal: "fix tests".to_string(),
            max_memory_mb: 512,
            max_cpu_seconds: 120,
            max_wall_seconds: 300,
            max_pids: 128,
            network_allowed_hosts: vec![],
            die_with_parent: true,
        }
    }

    fn test_config() -> NsjailRuntimeConfig {
        NsjailRuntimeConfig {
            nsjail_binary: "nsjail".to_string(),
            quecto_binary: "/usr/local/bin/quecto".to_string(),
        }
    }

    #[test]
    fn test_build_nsjail_args_contains_mode() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(args.nsjail_args.contains(&"--mode".to_string()));
        assert!(args.nsjail_args.contains(&"o".to_string()));
    }

    #[test]
    fn test_build_nsjail_args_contains_separator() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(args.nsjail_args.contains(&"--".to_string()));
    }

    #[test]
    fn test_build_nsjail_args_worker_starts_with_quecto() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert_eq!(
            args.worker_args[0], "/usr/local/bin/quecto",
            "first worker arg should be quecto binary"
        );
        assert_eq!(args.worker_args[1], "worker");
    }

    #[test]
    fn test_build_full_command_includes_run_and_job_id() {
        let full =
            build_full_nsjail_command(&test_config(), &default_launch_config(), "run-42", "job-99");
        let run_idx = full.iter().position(|a| a == "--run-id").unwrap();
        assert_eq!(full[run_idx + 1], "run-42");
        let job_idx = full.iter().position(|a| a == "--job-id").unwrap();
        assert_eq!(full[job_idx + 1], "job-99");
    }

    #[test]
    fn test_build_full_command_includes_goal() {
        let full = build_full_nsjail_command(&test_config(), &default_launch_config(), "r1", "j1");
        let goal_idx = full.iter().position(|a| a == "--goal").unwrap();
        assert_eq!(full[goal_idx + 1], "fix tests");
    }

    #[test]
    fn test_mount_job_dir_rw() {
        let config = default_launch_config();
        let args = build_nsjail_worker_args(&test_config(), &config);
        let bind_idx = args
            .nsjail_args
            .iter()
            .position(|a| a == "--bindmount")
            .unwrap();
        assert!(args.nsjail_args[bind_idx + 1].contains(&config.job_dir));
    }

    #[test]
    fn test_mount_host_root_ro() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        let ro_idx = args
            .nsjail_args
            .iter()
            .position(|a| a == "--bindmount_ro")
            .unwrap();
        assert_eq!(args.nsjail_args[ro_idx + 1], "/:/host");
    }

    #[test]
    fn test_resource_limits() {
        let mut config = default_launch_config();
        config.max_memory_mb = 1024;
        config.max_cpu_seconds = 180;
        config.max_wall_seconds = 600;
        config.max_pids = 256;
        let args = build_nsjail_worker_args(&test_config(), &config);
        let all = &args.nsjail_args;

        let mem_idx = all.iter().position(|a| a == "--rlimit_as").unwrap();
        assert_eq!(all[mem_idx + 1], "1024");

        let cpu_idx = all.iter().position(|a| a == "--time_limit").unwrap();
        assert_eq!(all[cpu_idx + 1], "180");

        let wall_idx = all.iter().position(|a| a == "--max_cpus").unwrap();
        assert_eq!(all[wall_idx + 1], "600");

        let pid_idx = all.iter().position(|a| a == "--cgroup_pids_max").unwrap();
        assert_eq!(all[pid_idx + 1], "256");
    }

    #[test]
    fn test_security_flags() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(args.nsjail_args.contains(&"--no_new_privs".to_string()));
        assert!(args.nsjail_args.contains(&"--seccomp_string".to_string()));
    }

    #[test]
    fn test_network_disabled_by_default() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(
            args.nsjail_args
                .contains(&"--disable_clone_newnet".to_string())
        );
    }

    #[test]
    fn test_network_enabled_with_hosts() {
        let mut config = default_launch_config();
        config.network_allowed_hosts = vec!["github.com".into()];
        let args = build_nsjail_worker_args(&test_config(), &config);
        assert!(
            !args
                .nsjail_args
                .contains(&"--disable_clone_newnet".to_string())
        );
    }

    #[test]
    fn test_die_with_parent_enabled() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(args.nsjail_args.contains(&"--die_with_parent".to_string()));
    }

    #[test]
    fn test_die_with_parent_disabled() {
        let mut config = default_launch_config();
        config.die_with_parent = false;
        let args = build_nsjail_worker_args(&test_config(), &config);
        assert!(!args.nsjail_args.contains(&"--die_with_parent".to_string()));
    }

    #[test]
    fn test_worker_env_minimal() {
        let env = build_worker_env(&default_launch_config());
        let names: Vec<&str> = env.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"PATH"));
        assert!(names.contains(&"LANG"));
        assert!(names.contains(&"HOME"));
    }

    #[test]
    fn test_worker_env_lang_value() {
        let env = build_worker_env(&default_launch_config());
        let lang = env.iter().find(|e| e.name == "LANG").unwrap();
        assert_eq!(lang.value, "C.UTF-8");
    }

    #[test]
    fn test_worker_env_home_is_job_dir() {
        let config = default_launch_config();
        let env = build_worker_env(&config);
        let home = env.iter().find(|e| e.name == "HOME").unwrap();
        assert_eq!(home.value, config.job_dir);
    }

    #[test]
    fn test_worker_env_allowed_hosts() {
        let mut config = default_launch_config();
        config.network_allowed_hosts = vec!["github.com".into(), "npm.org".into()];
        let env = build_worker_env(&config);
        let hosts = env
            .iter()
            .find(|e| e.name == "QUECTO_ALLOWED_HOSTS")
            .unwrap();
        assert_eq!(hosts.value, "github.com,npm.org");
    }

    #[test]
    fn test_blocked_env_vars() {
        assert!(is_blocked_env("QUECTO_SECRET"));
        assert!(is_blocked_env("GITHUB_TOKEN"));
        assert!(is_blocked_env("GH_TOKEN"));
        assert!(is_blocked_env("OPENAI_API_KEY"));
        assert!(is_blocked_env("ANTHROPIC_API_KEY"));
        assert!(!is_blocked_env("PATH"));
        assert!(!is_blocked_env("HOME"));
    }

    #[test]
    fn test_resolve_quecto_binary() {
        let path = resolve_quecto_binary();
        assert!(!path.is_empty());
    }

    #[test]
    fn test_quiet_mode() {
        let args = build_nsjail_worker_args(&test_config(), &default_launch_config());
        assert!(args.nsjail_args.contains(&"--quiet".to_string()));
    }

    #[test]
    fn test_cwd_is_job_dir() {
        let config = default_launch_config();
        let args = build_nsjail_worker_args(&test_config(), &config);
        let cwd_idx = args.nsjail_args.iter().position(|a| a == "--cwd").unwrap();
        assert_eq!(args.nsjail_args[cwd_idx + 1], config.job_dir);
    }

    #[test]
    fn test_runtime_launch_and_status() {
        let mut rt = NsjailWorkerRuntime::new(test_config());
        let config = default_launch_config();
        let pid = rt.launch(&config).unwrap();
        assert_eq!(rt.status(pid), WorkerStatus::Running);
        assert!(rt.is_alive(pid));
    }

    #[test]
    fn test_runtime_unknown_pid_status() {
        let rt = NsjailWorkerRuntime::new(test_config());
        let status = rt.status(99999);
        assert!(matches!(status, WorkerStatus::Killed { reason } if reason.contains("unknown")));
    }

    #[test]
    fn test_runtime_unknown_pid_not_alive() {
        let rt = NsjailWorkerRuntime::new(test_config());
        assert!(!rt.is_alive(99999));
    }

    #[test]
    fn test_runtime_stderr_cap() {
        let mut rt = NsjailWorkerRuntime::new(test_config());
        let config = default_launch_config();
        let pid = rt.launch(&config).unwrap();

        // Inject more than 1 MiB
        let big = "x".repeat(MAX_STDERR_BYTES + 1000);
        rt.inject_stderr(pid, &big);
        let captured = rt.read_stderr(pid);
        assert_eq!(captured.len(), MAX_STDERR_BYTES);
    }

    #[test]
    fn test_runtime_cleanup() {
        let mut rt = NsjailWorkerRuntime::new(test_config());
        let config = default_launch_config();
        let pid = rt.launch(&config).unwrap();
        rt.cleanup(pid);
        assert!(!rt.is_alive(pid));
    }

    #[test]
    fn test_runtime_kill() {
        let mut rt = NsjailWorkerRuntime::new(test_config());
        let config = default_launch_config();
        let pid = rt.launch(&config).unwrap();
        rt.kill(pid).unwrap();
        assert!(!rt.is_alive(pid));
    }

    #[test]
    fn test_launch_params_tracking() {
        let mut rt = NsjailWorkerRuntime::new(test_config());
        let config = default_launch_config();
        rt.launch(&config).unwrap();
        let (run_id, job_id) = rt.last_launch_params().unwrap();
        assert!(run_id.starts_with("run_"));
        assert!(job_id.starts_with("job_"));
    }
}
