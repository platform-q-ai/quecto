//! Trusted packaged Python loading and the SQLite lifecycle adapter.
//! No helper source is imported from the shared checkout or user site packages.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::domain::error::DomainError;

#[derive(Clone, Debug)]
pub struct SwarmContext {
    pub checkout: PathBuf,
    pub member: String,
    pub lifecycle: std::sync::Arc<dyn crate::domain::swarm::SwarmLifecycle>,
}

impl SwarmContext {
    /// Explicit context supplied by the container launch adapter. Host-local
    /// reference scripts deliberately do not set this contract.
    pub fn discover(
        lifecycle: std::sync::Arc<dyn crate::domain::swarm::SwarmLifecycle>,
    ) -> Option<Self> {
        let checkout = std::env::var_os("QUECTO_SWARM_CHECKOUT")?;
        let protocol = std::env::var("QUECTO_SWARM_CONTAINER").ok()?;
        if protocol != "isolated-pid-v1" {
            return None;
        }
        let host_namespace = std::env::var("QUECTO_SWARM_HOST_PID_NS").ok()?;
        if !isolated_pid_namespace(&host_namespace) {
            return None;
        }
        let member = std::env::var("QUECTO_SWARM_MEMBER")
            .unwrap_or_else(|_| format!("member-{}", std::process::id()));
        Some(Self {
            checkout: checkout.into(),
            member,
            lifecycle,
        })
    }

    pub fn database(&self) -> PathBuf {
        self.checkout.join(".quecto/swarm.sqlite")
    }

    pub fn bootstrap(&self) -> String {
        let mut source = String::from("import sys, types, json\n");
        for (name, body) in [
            ("swarm_policy", include_str!("../../domain/swarm_policy.py")),
            (
                "swarm_use_cases",
                include_str!("../../application/swarm_use_cases.py"),
            ),
            (
                "swarm_repository",
                include_str!("swarm_helpers/swarm_repository.py"),
            ),
            ("swarm_store", include_str!("swarm_helpers/swarm_store.py")),
            ("swarm_tasks", include_str!("swarm_helpers/swarm_tasks.py")),
            ("swarm", include_str!("swarm_helpers/swarm.py")),
        ] {
            source.push_str(&format!(
                "_m=types.ModuleType({name:?}); sys.modules[{name:?}]=_m; exec(compile({}, {name:?}, 'exec'), _m.__dict__)\n",
                serde_json::to_string(body).expect("source serializes")
            ));
        }
        source.push_str(&format!(
            "import swarm\nswarm.board=swarm.Workbench({}, {}, {})\n",
            json!(self.database().to_string_lossy()),
            json!(self.checkout.to_string_lossy()),
            json!(self.member),
        ));
        source
    }

    fn rpc(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        let source = format!(
            "{}\ntry:\n print(json.dumps({{'ok':getattr(swarm.board,{}) (*json.loads({}))}}))\nexcept Exception as e:\n print(json.dumps({{'error':str(e)}}))\n",
            self.bootstrap(),
            json!(method),
            json!(args.to_string())
        );
        let output = std::process::Command::new("python3")
            .args(["-I", "-c", &source])
            .env_clear()
            .env("PATH", "/usr/local/bin:/usr/bin:/bin")
            .output()
            .map_err(|e| DomainError::Tool(format!("swarm coordination interpreter: {e}")))?;
        if !output.status.success() {
            return Err(DomainError::Tool(format!(
                "swarm coordination failed: {}",
                String::from_utf8_lossy(&output.stderr)
            )));
        }
        let value: Value = serde_json::from_slice(&output.stdout)
            .map_err(|e| DomainError::Tool(format!("invalid swarm coordination response: {e}")))?;
        if let Some(error) = value.get("error") {
            return Err(DomainError::Tool(format!("swarm: {error}")));
        }
        Ok(value["ok"].clone())
    }

    #[cfg(any(test, feature = "test-support"))]
    pub(crate) fn call(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        self.rpc(method, args)
    }

    pub fn accept_wake(&self, generation: u64) -> Result<bool, DomainError> {
        self.rpc("_accept_wake", json!([generation]))?
            .as_bool()
            .ok_or_else(|| DomainError::Tool("invalid wake receipt".into()))
    }

    pub fn pause(&self, reason: &str) -> Result<Value, DomainError> {
        self.rpc("pause", json!([reason]))
    }

    pub fn resume(&self) -> Result<Value, DomainError> {
        self.rpc("resume", json!([]))
    }

    pub fn control_status(&self) -> Result<Value, DomainError> {
        self.rpc("_control_status", json!([]))
    }

    pub fn usage_report(&self) -> Result<Value, DomainError> {
        self.rpc("usage_report", json!([]))
    }
    pub fn usage_budget(
        &self,
        limit: Option<u64>,
        strict_unknown: bool,
    ) -> Result<Value, DomainError> {
        self.rpc("usage_budget", json!([limit, strict_unknown]))
    }

    pub fn events(&self, after: u64, limit: u32) -> Result<Value, DomainError> {
        self.rpc("events", json!([after, limit]))
    }

    pub fn summary(&self) -> Result<Value, DomainError> {
        self.summary_since(None)
    }

    pub fn summary_since(&self, since: Option<u64>) -> Result<Value, DomainError> {
        self.rpc("summary", json!([since]))
    }
}

/// Kernel process start time disambiguates recycled PIDs. Missing procfs is
/// unknown, never proof that a worker has stopped modifying the checkout.
pub fn process_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(") ")?
        .1
        .split_whitespace()
        .nth(19)
        .map(str::to_owned)
}

pub fn process_confirmed_dead(pid: u32, start: &str) -> bool {
    match process_start(pid) {
        Some(current) => current != start,
        None => {
            !Path::new(&format!("/proc/{pid}")).exists() && Path::new("/proc/self/stat").exists()
        }
    }
}

static PROCESS_SOCKET: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

pub fn set_process_socket(socket: PathBuf) {
    let _ = PROCESS_SOCKET.set(socket);
}

pub fn process_socket() -> Option<&'static Path> {
    PROCESS_SOCKET.get().map(PathBuf::as_path)
}

fn isolated_pid_namespace(host: &str) -> bool {
    host.starts_with("pid:[")
        && host.ends_with(']')
        && std::fs::read_link("/proc/self/ns/pid")
            .is_ok_and(|current| current.to_string_lossy() != host)
}

#[cfg(test)]
mod context_tests {
    #[test]
    fn host_pid_namespace_cannot_confer_container_availability() {
        let current = std::fs::read_link("/proc/self/ns/pid").unwrap();
        assert!(!super::isolated_pid_namespace(&current.to_string_lossy()));
        assert!(!super::isolated_pid_namespace("not a namespace identity"));
    }
}

#[path = "swarm_coordination.rs"]
mod coordination;
