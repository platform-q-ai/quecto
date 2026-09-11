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
        store_database(&self.checkout)
    }

    pub fn bootstrap(&self) -> String {
        bootstrap_source(&self.checkout, &self.member)
    }

    fn rpc(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        store_rpc(&self.checkout, &self.member, method, args)
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

    /// The coordinator's own resume: refused by the store (#1729).
    pub fn resume(&self) -> Result<Value, DomainError> {
        self.rpc("resume", json!([]))
    }

    /// Supervisor resume from outside the swarm (#1729).
    pub fn resume_external(&self) -> Result<Value, DomainError> {
        self.rpc("_resume_external", json!([]))
    }

    /// Supervisor close: the held outcome becomes terminal (#1729).
    pub fn close(&self) -> Result<Value, DomainError> {
        self.rpc("_close", json!([]))
    }

    /// Supervisor deadline extension in seconds (#1729).
    pub fn extend_deadline(&self, seconds: u64) -> Result<Value, DomainError> {
        self.rpc("_extend_deadline", json!([seconds]))
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

    /// Whether a run has been created in this container, readable before
    /// joining (no membership needed): a container without a store is at its
    /// bootstrap placeholder.
    pub fn run_created(&self) -> Result<bool, DomainError> {
        if !self.database().exists() {
            return Ok(false);
        }
        let status = self.rpc("_status", json!([]))?;
        let deadline = status["deadline"]
            .as_f64()
            .ok_or_else(|| DomainError::Tool("swarm status carries no deadline".into()))?;
        Ok(crate::domain::swarm::participates(deadline))
    }

    pub fn summary_since(&self, since: Option<u64>) -> Result<Value, DomainError> {
        self.rpc("summary", json!([since]))
    }
}

/// The coordination store every member of a container shares, by checkout.
pub fn store_database(checkout: &Path) -> PathBuf {
    checkout.join(".quecto/swarm.sqlite")
}

fn bootstrap_source(checkout: &Path, member: &str) -> String {
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
        json!(store_database(checkout).to_string_lossy()),
        json!(checkout.to_string_lossy()),
        json!(member),
    ));
    source
}

/// One isolated interpreter call against the store at `checkout`, acting as
/// `member`. Shared by in-swarm contexts and the supervising session's
/// host-side handle (#1924).
fn store_rpc(
    checkout: &Path,
    member: &str,
    method: &str,
    args: Value,
) -> Result<Value, DomainError> {
    let source = format!(
        "{}\ntry:\n print(json.dumps({{'ok':getattr(swarm.board,{}) (*json.loads({}))}}))\nexcept Exception as e:\n print(json.dumps({{'error':str(e)}}))\n",
        bootstrap_source(checkout, member),
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

/// Host-side handle on a container's coordination store for the supervising
/// session (#1924): the store lives in the identity-mounted checkout, so the
/// session that launched the container reads it by path once the members'
/// sockets are gone. Membership-free reads only, plus the #1729 lost-harness
/// record made as the lost coordinator itself.
#[derive(Clone, Debug)]
pub struct HostedStore {
    checkout: PathBuf,
}

impl HostedStore {
    pub fn at(checkout: PathBuf) -> Self {
        Self { checkout }
    }

    /// The run the store holds, or `None` when no store exists there. Live
    /// members may hold the store's short immediate transactions, so a read
    /// that fails is retried a few times before it is reported unreadable.
    pub fn hosted_run(
        &self,
    ) -> Result<Option<crate::domain::environment_finalization::HostedSwarmRun>, DomainError> {
        if !store_database(&self.checkout).is_file() {
            return Ok(None);
        }
        const ATTEMPTS: u32 = 4;
        let mut attempt = 1;
        let status = loop {
            match store_rpc(&self.checkout, "supervisor", "_status", json!([])) {
                Ok(status) => break status,
                Err(error) if attempt < ATTEMPTS => {
                    tracing::debug!(%error, attempt, "hosted swarm store read failed; retrying");
                    std::thread::sleep(std::time::Duration::from_millis(250 * u64::from(attempt)));
                    attempt += 1;
                }
                Err(error) => return Err(error),
            }
        };
        let deadline = status["deadline"]
            .as_f64()
            .ok_or_else(|| DomainError::Tool("swarm status carries no deadline".into()))?;
        let run_status = status["status"]
            .as_str()
            .ok_or_else(|| DomainError::Tool("swarm status carries no status".into()))?;
        let coordinator = status["coordinator"]
            .as_str()
            .ok_or_else(|| DomainError::Tool("swarm status carries no coordinator".into()))?;
        Ok(Some(
            crate::domain::environment_finalization::HostedSwarmRun {
                status: coordination::decode_status(run_status)?,
                coordinator: coordinator.to_owned(),
                deadline,
            },
        ))
    }

    /// Quarantine `coordinator` exactly as an in-swarm reconcile would when
    /// its harness vanished: the run ends as a pause holding `failed`.
    pub fn record_lost_coordinator(&self, coordinator: &str) -> Result<(), DomainError> {
        store_rpc(
            &self.checkout,
            coordinator,
            "_quarantine",
            json!([coordinator]),
        )
        .map(|_| ())
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

/// A composition's workflow engine slot (#1715): filled once the workflow
/// runtime is built, read by the swarm tool before creating a run.
pub type WorkflowEngineSlot = std::sync::Arc<
    std::sync::OnceLock<std::sync::Arc<std::sync::Mutex<crate::domain::workflow::WorkflowEngine>>>,
>;

/// Whether the composition is running a workflow right now: guards on, or a
/// template selected/bound. A merely available, idle engine is not engaged.
pub fn workflow_engaged(slot: &WorkflowEngineSlot) -> bool {
    slot.get().is_some_and(|engine| {
        engine
            .lock()
            .map(|engine| engine.guards_enabled() || engine.active_template().is_some())
            .unwrap_or(true)
    })
}

/// Whether this process takes part in a swarm (#1715). One shared handle is
/// created per process and injected into the spawn, workflow and swarm
/// tools; creating a run (or a supervisor tick seeing one) flips it, so an
/// ordinary container that becomes a swarm after composition is seen by every
/// tool at once. Hooks run once on the transition into participation (the
/// workflow selector nudge is switched off there). Tests inject a fixed answer.
#[derive(Clone)]
pub enum Participation {
    Shared(std::sync::Arc<SharedParticipation>),
    Fixed(bool),
}

type ParticipationHook = Box<dyn Fn() + Send + Sync>;

#[derive(Default)]
pub struct SharedParticipation {
    flag: std::sync::atomic::AtomicBool,
    hooks: std::sync::Mutex<Vec<ParticipationHook>>,
}

impl std::fmt::Debug for Participation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Participation({})", self.participating())
    }
}

impl Participation {
    pub fn shared() -> Self {
        Self::Shared(std::sync::Arc::default())
    }

    /// No swarm at all (host-local runtimes and isolated consumers).
    pub fn none() -> Self {
        Self::Fixed(false)
    }

    pub fn participating(&self) -> bool {
        match self {
            Self::Shared(shared) => shared.flag.load(std::sync::atomic::Ordering::SeqCst),
            Self::Fixed(value) => *value,
        }
    }

    /// Run `hook` once when this process becomes a swarm participant (at once
    /// if it already is). A fixed answer never transitions.
    pub fn on_participation(&self, hook: impl Fn() + Send + Sync + 'static) {
        let Self::Shared(shared) = self else {
            return;
        };
        if shared.flag.load(std::sync::atomic::Ordering::SeqCst) {
            hook();
            return;
        }
        shared
            .hooks
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(Box::new(hook));
    }

    /// Record participation; a fixed answer never changes. Participation is
    /// never revoked: a created run stays a swarm through every later status.
    pub fn set(&self, participating: bool) {
        let Self::Shared(shared) = self else {
            return;
        };
        if !participating || shared.flag.swap(true, std::sync::atomic::Ordering::SeqCst) {
            return;
        }
        let hooks = std::mem::take(
            &mut *shared
                .hooks
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
        for hook in hooks {
            hook();
        }
    }
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
#[cfg(test)]
#[path = "swarm_participation_tests.rs"]
mod participation_tests;
