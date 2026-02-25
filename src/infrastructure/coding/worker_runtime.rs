//! nsjail coding worker runtime implementation.
//!
//! Manages the lifecycle of coding worker processes inside nsjail containers.
//! Each worker communicates via JSON Lines over stdin/stdout.

use std::collections::{HashMap, VecDeque};

use crate::domain::coding_ports::{
    WorkerEnvVar, WorkerEvent, WorkerLaunchConfig, WorkerRuntime, WorkerStatus,
};

/// Minimal PATH and locale variables for the worker environment.
const WORKER_PATH: &str = "/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin";
const WORKER_LANG: &str = "C.UTF-8";

/// Prefixes that are stripped from the worker's environment.
const BLOCKED_ENV_PREFIXES: &[&str] = &["QUECTO_", "GITHUB_", "GH_", "OPENAI_", "ANTHROPIC_"];

/// Blocked individual env var names.
const BLOCKED_ENV_NAMES: &[&str] = &["GITHUB_TOKEN", "GH_TOKEN"];

/// State of a tracked worker process.
#[derive(Debug)]
struct WorkerState {
    status: WorkerStatus,
    events: VecDeque<WorkerEvent>,
    stderr: String,
    env: Vec<WorkerEnvVar>,
    nsjail_args: Vec<String>,
    commands_received: Vec<String>,
}

/// Mock worker runtime for testing. Simulates nsjail process management.
///
/// In production, this would spawn actual nsjail processes. For BDD and unit
/// testing, it tracks process state in memory.
#[derive(Debug)]
pub struct MockWorkerRuntime {
    workers: HashMap<u32, WorkerState>,
    next_pid: u32,
}

impl MockWorkerRuntime {
    pub fn new() -> Self {
        Self {
            workers: HashMap::new(),
            next_pid: 10000,
        }
    }

    /// Simulate the worker emitting an event on stdout.
    pub fn inject_event(&mut self, pid: u32, event: WorkerEvent) {
        if let Some(w) = self.workers.get_mut(&pid) {
            w.events.push_back(event);
        }
    }

    /// Simulate the worker writing to stderr.
    pub fn inject_stderr(&mut self, pid: u32, output: &str) {
        if let Some(w) = self.workers.get_mut(&pid) {
            w.stderr.push_str(output);
        }
    }

    /// Simulate the worker exiting with a status code.
    pub fn simulate_exit(&mut self, pid: u32, status: i32) {
        if let Some(w) = self.workers.get_mut(&pid) {
            w.status = WorkerStatus::Exited { status };
        }
    }

    /// Simulate the worker being killed.
    pub fn simulate_kill(&mut self, pid: u32, reason: &str) {
        if let Some(w) = self.workers.get_mut(&pid) {
            w.status = WorkerStatus::Killed {
                reason: reason.to_string(),
            };
        }
    }

    /// Get the environment variables for a running worker.
    pub fn worker_env_for(&self, pid: u32) -> Vec<WorkerEnvVar> {
        self.workers
            .get(&pid)
            .map(|w| w.env.clone())
            .unwrap_or_default()
    }

    /// Get the nsjail arguments for a running worker.
    pub fn nsjail_args_for(&self, pid: u32) -> Vec<String> {
        self.workers
            .get(&pid)
            .map(|w| w.nsjail_args.clone())
            .unwrap_or_default()
    }

    /// Get the number of currently running workers.
    pub fn running_count(&self) -> usize {
        self.workers
            .values()
            .filter(|w| w.status == WorkerStatus::Running)
            .count()
    }

    /// Build nsjail arguments for the given configuration.
    fn build_nsjail_args(config: &WorkerLaunchConfig) -> Vec<String> {
        let mut args = Vec::new();
        args.push("--mode".to_string());
        args.push("o".to_string());

        // Mount job directory read-write
        args.push("--bindmount".to_string());
        args.push(format!("{}:{}", config.job_dir, config.job_dir));

        // Mount host root read-only
        args.push("--bindmount_ro".to_string());
        args.push("/:/host".to_string());

        // Resource limits
        args.push("--rlimit_as".to_string());
        args.push(config.max_memory_mb.to_string());
        args.push("--time_limit".to_string());
        args.push(config.max_cpu_seconds.to_string());
        args.push("--max_cpus".to_string());
        args.push(config.max_wall_seconds.to_string());
        args.push("--cgroup_pids_max".to_string());
        args.push(config.max_pids.to_string());

        // Security
        args.push("--no_new_privs".to_string());
        args.push("--seccomp_string".to_string());
        args.push("POLICY { ALLOW { } }".to_string());

        // Network
        if config.network_allowed_hosts.is_empty() {
            args.push("--disable_clone_newnet".to_string());
        }

        // Die with parent
        if config.die_with_parent {
            args.push("--die_with_parent".to_string());
        }

        args
    }

    /// Build the minimal worker environment.
    fn build_worker_env(config: &WorkerLaunchConfig) -> Vec<WorkerEnvVar> {
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

        // Add allowed network hosts as env var if present
        if !config.network_allowed_hosts.is_empty() {
            env.push(WorkerEnvVar {
                name: "QUECTO_ALLOWED_HOSTS".to_string(),
                value: config.network_allowed_hosts.join(","),
            });
        }

        env
    }
}

impl Default for MockWorkerRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl WorkerRuntime for MockWorkerRuntime {
    fn launch(&mut self, config: &WorkerLaunchConfig) -> Result<u32, String> {
        let pid = self.next_pid;
        self.next_pid += 1;

        let nsjail_args = Self::build_nsjail_args(config);
        let env = Self::build_worker_env(config);

        self.workers.insert(
            pid,
            WorkerState {
                status: WorkerStatus::Running,
                events: VecDeque::new(),
                stderr: String::new(),
                env,
                nsjail_args,
                commands_received: Vec::new(),
            },
        );
        Ok(pid)
    }

    fn send_command(&mut self, pid: u32, command: &str) -> Result<(), String> {
        match self.workers.get_mut(&pid) {
            Some(w) if w.status == WorkerStatus::Running => {
                w.commands_received.push(command.to_string());
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
                let out = w.stderr.clone();
                w.stderr.clear();
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
            Some(_) => Ok(()), // Already dead
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
        Self::build_nsjail_args(config)
    }

    fn worker_env(&self, config: &WorkerLaunchConfig) -> Vec<WorkerEnvVar> {
        Self::build_worker_env(config)
    }

    fn cleanup(&mut self, pid: u32) {
        self.workers.remove(&pid);
    }
}

/// Check if an environment variable name is blocked from the worker.
pub fn is_blocked_env(name: &str) -> bool {
    if BLOCKED_ENV_NAMES.contains(&name) {
        return true;
    }
    BLOCKED_ENV_PREFIXES.iter().any(|p| name.starts_with(p))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coding_event::EventEnvelope;

    fn default_config() -> WorkerLaunchConfig {
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

    #[test]
    fn test_launch_and_status() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        assert_eq!(rt.status(pid), WorkerStatus::Running);
        assert!(rt.is_alive(pid));
    }

    #[test]
    fn test_send_command() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        assert!(rt.send_command(pid, r#"{"type":"ping"}"#).is_ok());
    }

    #[test]
    fn test_read_event() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        assert!(rt.read_event(pid).is_none());

        let env = EventEnvelope {
            v: "1.0".to_string(),
            ts: "2026-01-01T00:00:00Z".to_string(),
            run_id: "r1".to_string(),
            job_id: "j1".to_string(),
            source: crate::domain::coding_event::EventSource::Worker,
            event_type: "job.status".to_string(),
            seq: 1,
            payload: serde_json::json!({"state": "running"}),
        };
        rt.inject_event(pid, WorkerEvent::Valid(env));
        assert!(matches!(rt.read_event(pid), Some(WorkerEvent::Valid(_))));
    }

    #[test]
    fn test_malformed_event() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        rt.inject_event(
            pid,
            WorkerEvent::Malformed {
                raw: "not json".into(),
            },
        );
        assert!(matches!(
            rt.read_event(pid),
            Some(WorkerEvent::Malformed { .. })
        ));
    }

    #[test]
    fn test_stderr() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        rt.inject_stderr(pid, "warning: something");
        assert_eq!(rt.read_stderr(pid), "warning: something");
        assert_eq!(rt.read_stderr(pid), ""); // cleared after read
    }

    #[test]
    fn test_exit_and_kill() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        rt.simulate_exit(pid, 0);
        assert_eq!(rt.status(pid), WorkerStatus::Exited { status: 0 });
        assert!(!rt.is_alive(pid));

        let pid2 = rt.launch(&default_config()).unwrap();
        assert!(rt.kill(pid2).is_ok());
        assert!(!rt.is_alive(pid2));
    }

    #[test]
    fn test_nsjail_args() {
        let rt = MockWorkerRuntime::new();
        let args = rt.nsjail_args(&default_config());
        assert!(args.contains(&"--no_new_privs".to_string()));
        assert!(args.contains(&"--seccomp_string".to_string()));
        assert!(args.contains(&"--rlimit_as".to_string()));
        assert!(args.contains(&"512".to_string()));
        assert!(args.contains(&"--cgroup_pids_max".to_string()));
        assert!(args.contains(&"128".to_string()));
        assert!(args.contains(&"--disable_clone_newnet".to_string()));
        assert!(args.contains(&"--die_with_parent".to_string()));
    }

    #[test]
    fn test_nsjail_args_with_network() {
        let rt = MockWorkerRuntime::new();
        let mut config = default_config();
        config.network_allowed_hosts = vec!["github.com".into()];
        let args = rt.nsjail_args(&config);
        assert!(!args.contains(&"--disable_clone_newnet".to_string()));
    }

    #[test]
    fn test_worker_env_minimal() {
        let rt = MockWorkerRuntime::new();
        let env = rt.worker_env(&default_config());
        let names: Vec<&str> = env.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"PATH"));
        assert!(names.contains(&"LANG"));
        assert!(names.contains(&"HOME"));
        assert!(env.len() <= 4, "env should be minimal");
    }

    #[test]
    fn test_blocked_env() {
        assert!(is_blocked_env("QUECTO_SECRET_KEY"));
        assert!(is_blocked_env("QUECTO_API_KEY"));
        assert!(is_blocked_env("GITHUB_TOKEN"));
        assert!(is_blocked_env("GH_TOKEN"));
        assert!(is_blocked_env("OPENAI_API_KEY"));
        assert!(is_blocked_env("ANTHROPIC_API_KEY"));
        assert!(!is_blocked_env("PATH"));
        assert!(!is_blocked_env("HOME"));
        assert!(!is_blocked_env("LANG"));
    }

    #[test]
    fn test_running_count() {
        let mut rt = MockWorkerRuntime::new();
        assert_eq!(rt.running_count(), 0);
        let p1 = rt.launch(&default_config()).unwrap();
        let p2 = rt.launch(&default_config()).unwrap();
        assert_eq!(rt.running_count(), 2);
        rt.simulate_exit(p1, 0);
        assert_eq!(rt.running_count(), 1);
        rt.kill(p2).unwrap();
        assert_eq!(rt.running_count(), 0);
    }

    #[test]
    fn test_cleanup() {
        let mut rt = MockWorkerRuntime::new();
        let pid = rt.launch(&default_config()).unwrap();
        rt.cleanup(pid);
        assert!(!rt.is_alive(pid));
    }

    #[test]
    fn test_mount_table_in_args() {
        let rt = MockWorkerRuntime::new();
        let args = rt.nsjail_args(&default_config());
        // Job dir should be in bindmount (rw)
        assert!(args.iter().any(|a| a.contains("/tmp/jobs/job_001/repo")));
        // Host root should be in bindmount_ro
        assert!(args.contains(&"--bindmount_ro".to_string()));
        assert!(args.iter().any(|a| a.contains("/:/host")));
    }
}
