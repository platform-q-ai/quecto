//! Trusted packaged Python loading and the SQLite lifecycle adapter.
//! No helper source is imported from the shared checkout or user site packages.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::domain::error::DomainError;

#[derive(Clone, Debug)]
pub struct SwarmContext {
    pub checkout: PathBuf,
    pub member: String,
}

impl SwarmContext {
    /// Explicit context supplied by the container launch adapter. Host-local
    /// reference scripts deliberately do not set this contract.
    pub fn discover() -> Option<Self> {
        let checkout = std::env::var_os("QUECTO_SWARM_CHECKOUT")?;
        let runtime = std::env::var("QUECTO_CONTAINER_RUNTIME").ok()?;
        if !matches!(runtime.as_str(), "docker" | "podman") {
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
        })
    }

    pub fn database(&self) -> PathBuf {
        self.checkout.join(".quecto/swarm.sqlite")
    }

    pub fn bootstrap(&self) -> String {
        let mut source = String::from("import sys, types, json\n");
        for (name, body) in [
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

    pub fn call(&self, method: &str, args: Value) -> Result<Value, DomainError> {
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

    pub fn summary(&self) -> Result<Value, DomainError> {
        self.call("summary", json!([]))
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
