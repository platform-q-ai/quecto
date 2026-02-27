//! Infrastructure implementation of the CoordinatorSpawner port.
//!
//! Spawns the coordinator as a detached `quecto agent` child process with
//! a coordinator-specific system prompt, long timeout, and named session.
//! Writes the child PID to `coordinator/pid` for liveness checks.

use std::path::PathBuf;
use std::sync::Arc;

use crate::domain::coding_ipc::{CoordinatorIpc, CoordinatorSpawner, SpawnResult};

/// Default maximum timeout for the coordinator process (24 hours).
const DEFAULT_MAX_TIMEOUT_SECS: u64 = 86400;

/// Default session name for the coordinator process.
const DEFAULT_SESSION_NAME: &str = "coordinator";

/// Configuration for the coordinator process spawner.
#[derive(Debug, Clone)]
pub struct CoordinatorSpawnConfig {
    /// Base directory for the coordinator process (QUECTO_BASE_DIR).
    pub base_dir: PathBuf,
    /// Session name for persistence across restarts.
    pub session: String,
    /// Maximum wall-clock timeout in seconds.
    pub max_timeout_secs: u64,
    /// System prompt for the coordinator agent.
    pub system_prompt: Option<String>,
}

impl CoordinatorSpawnConfig {
    /// Create a new config with default settings.
    pub fn new(base_dir: PathBuf) -> Self {
        Self {
            base_dir,
            session: DEFAULT_SESSION_NAME.to_string(),
            max_timeout_secs: DEFAULT_MAX_TIMEOUT_SECS,
            system_prompt: None,
        }
    }

    /// Set the session name.
    pub fn with_session(mut self, session: &str) -> Self {
        self.session = session.to_string();
        self
    }

    /// Set the maximum timeout.
    pub fn with_max_timeout(mut self, secs: u64) -> Self {
        self.max_timeout_secs = secs;
        self
    }

    /// Set the system prompt.
    pub fn with_system_prompt(mut self, prompt: &str) -> Self {
        self.system_prompt = Some(prompt.to_string());
        self
    }
}

/// Spawns the coordinator as a real `quecto agent` child process.
///
/// On `ensure_alive()`:
/// 1. Checks if the coordinator is alive via `CoordinatorIpc::is_coordinator_alive()`.
/// 2. If alive, returns the existing PID from the pid file.
/// 3. If not alive, resolves the current executable, spawns a child `quecto agent`
///    with coordinator-specific flags, records the PID, and returns.
pub struct CoordinatorProcessSpawner {
    ipc: Arc<dyn CoordinatorIpc>,
    config: CoordinatorSpawnConfig,
}

impl std::fmt::Debug for CoordinatorProcessSpawner {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CoordinatorProcessSpawner")
            .field("session", &self.config.session)
            .field("max_timeout_secs", &self.config.max_timeout_secs)
            .finish()
    }
}

impl CoordinatorProcessSpawner {
    pub fn new(ipc: Arc<dyn CoordinatorIpc>, config: CoordinatorSpawnConfig) -> Self {
        Self { ipc, config }
    }

    /// Session name getter (for BDD assertions).
    pub fn session_name(&self) -> &str {
        &self.config.session
    }

    /// Max timeout getter (for BDD assertions).
    pub fn max_timeout_secs(&self) -> u64 {
        self.config.max_timeout_secs
    }

    /// Spawn the coordinator child process. Returns the child PID.
    fn spawn_coordinator(&self) -> Result<u32, String> {
        let exe = std::env::current_exe().map_err(|e| format!("current_exe: {e}"))?;

        let mut cmd = std::process::Command::new(exe);
        cmd.arg("agent")
            .arg("-m")
            .arg("You are the coordinator. Process inbox commands and manage coding jobs.")
            .arg("-s")
            .arg(&self.config.session)
            .arg("--max-time")
            .arg(self.config.max_timeout_secs.to_string());

        if let Some(ref prompt) = self.config.system_prompt {
            cmd.arg("--system").arg(prompt);
        }

        // Set QUECTO_BASE_DIR so the child uses the same config/workspace.
        cmd.env("QUECTO_BASE_DIR", &self.config.base_dir);

        // Detach stdout/stderr — communication is via file-based IPC only.
        cmd.stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null());

        let child = cmd.spawn().map_err(|e| format!("spawn failed: {e}"))?;
        let pid = child.id();

        // Record the PID for liveness checks.
        self.ipc
            .write_pid(pid)
            .map_err(|e| format!("write_pid: {e}"))?;

        Ok(pid)
    }
}

impl CoordinatorSpawner for CoordinatorProcessSpawner {
    fn ensure_alive(&self) -> Result<SpawnResult, String> {
        // Check if the coordinator is already alive.
        if self.ipc.is_coordinator_alive() {
            let pid = self
                .ipc
                .read_pid()
                .map_err(|e| format!("read_pid: {e}"))?
                .ok_or_else(|| "coordinator alive but no PID file".to_string())?;
            return Ok(SpawnResult {
                pid,
                spawned: false,
            });
        }

        // Not alive — spawn a new coordinator.
        let pid = self.spawn_coordinator()?;
        Ok(SpawnResult { pid, spawned: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::coding_ipc::*;
    use std::sync::Mutex;

    /// Test mock IPC that simulates alive/dead coordinator states.
    #[derive(Debug)]
    struct TestMockIpc {
        alive: bool,
        pid: Mutex<Option<u32>>,
        write_pid_calls: Mutex<Vec<u32>>,
    }

    impl TestMockIpc {
        fn alive(pid: u32) -> Self {
            Self {
                alive: true,
                pid: Mutex::new(Some(pid)),
                write_pid_calls: Mutex::new(vec![]),
            }
        }

        fn dead() -> Self {
            Self {
                alive: false,
                pid: Mutex::new(None),
                write_pid_calls: Mutex::new(vec![]),
            }
        }
    }

    impl CoordinatorIpc for TestMockIpc {
        fn write_command(&self, _cmd: &CoordinatorIpcCommand) -> Result<(), String> {
            Ok(())
        }
        fn read_pending_commands(&self) -> Result<Vec<CoordinatorIpcCommand>, String> {
            Ok(vec![])
        }
        fn acknowledge_command(&self, _command_id: &str) -> Result<(), String> {
            Ok(())
        }
        fn write_response(&self, _resp: &CoordinatorIpcResponse) -> Result<(), String> {
            Ok(())
        }
        fn read_response(
            &self,
            _command_id: &str,
        ) -> Result<Option<CoordinatorIpcResponse>, String> {
            Ok(None)
        }
        fn write_notification(&self, _notif: &CoordinatorNotification) -> Result<(), String> {
            Ok(())
        }
        fn read_notifications(&self) -> Result<Vec<CoordinatorNotification>, String> {
            Ok(vec![])
        }
        fn acknowledge_notification(&self, _filename: &str) -> Result<(), String> {
            Ok(())
        }
        fn write_state(&self, _state: &CoordinatorState) -> Result<(), String> {
            Ok(())
        }
        fn read_state(&self) -> Result<Option<CoordinatorState>, String> {
            Ok(None)
        }
        fn write_pid(&self, pid: u32) -> Result<(), String> {
            *self.pid.lock().unwrap() = Some(pid);
            self.write_pid_calls.lock().unwrap().push(pid);
            Ok(())
        }
        fn read_pid(&self) -> Result<Option<u32>, String> {
            Ok(*self.pid.lock().unwrap())
        }
        fn is_coordinator_alive(&self) -> bool {
            self.alive
        }
    }

    #[test]
    fn test_config_defaults() {
        let config = CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q"));
        assert_eq!(config.session, "coordinator");
        assert_eq!(config.max_timeout_secs, 86400);
        assert!(config.system_prompt.is_none());
    }

    #[test]
    fn test_config_builder() {
        let config = CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q"))
            .with_session("my-coordinator")
            .with_max_timeout(3600)
            .with_system_prompt("You are a coordinator.");
        assert_eq!(config.session, "my-coordinator");
        assert_eq!(config.max_timeout_secs, 3600);
        assert_eq!(
            config.system_prompt.as_deref(),
            Some("You are a coordinator.")
        );
    }

    #[test]
    fn test_ensure_alive_already_alive() {
        let ipc = Arc::new(TestMockIpc::alive(42));
        let spawner = CoordinatorProcessSpawner::new(
            ipc.clone(),
            CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q")),
        );
        let result = spawner.ensure_alive().expect("should succeed");
        assert_eq!(result.pid, 42);
        assert!(!result.spawned);
        assert!(ipc.write_pid_calls.lock().unwrap().is_empty());
    }

    #[test]
    fn test_session_name_getter() {
        let ipc = Arc::new(TestMockIpc::dead());
        let spawner = CoordinatorProcessSpawner::new(
            ipc,
            CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q")).with_session("custom"),
        );
        assert_eq!(spawner.session_name(), "custom");
    }

    #[test]
    fn test_max_timeout_getter() {
        let ipc = Arc::new(TestMockIpc::dead());
        let spawner = CoordinatorProcessSpawner::new(
            ipc,
            CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q")).with_max_timeout(7200),
        );
        assert_eq!(spawner.max_timeout_secs(), 7200);
    }

    #[test]
    fn test_debug_format() {
        let ipc = Arc::new(TestMockIpc::dead());
        let spawner = CoordinatorProcessSpawner::new(
            ipc,
            CoordinatorSpawnConfig::new(PathBuf::from("/tmp/q")),
        );
        let debug = format!("{spawner:?}");
        assert!(debug.contains("CoordinatorProcessSpawner"));
        assert!(debug.contains("coordinator"));
    }

    // Note: We cannot test spawn_coordinator() in unit tests because it
    // actually tries to spawn a child process. The BDD tests use mock
    // spawners instead. The real spawn path is tested in e2e tests.
}
