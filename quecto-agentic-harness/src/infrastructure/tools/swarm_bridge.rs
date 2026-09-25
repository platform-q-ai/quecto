//! Trusted packaged Python loading and the SQLite lifecycle adapter.
//! No helper source is imported from the shared checkout or user site packages.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::domain::error::DomainError;

#[derive(Clone, Debug)]
pub struct SwarmContext {
    pub checkout: PathBuf,
    pub member: String,
    pub lifecycle: std::sync::Arc<dyn crate::application::swarm::ports::SwarmLifecycle>,
}

impl SwarmContext {
    /// Explicit context supplied by the container launch adapter. Host-local
    /// reference scripts deliberately do not set this contract.
    pub fn discover(
        lifecycle: std::sync::Arc<dyn crate::application::swarm::ports::SwarmLifecycle>,
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
        super::swarm_store_location::member_store_path(&self.checkout)
    }

    pub fn bootstrap(&self) -> String {
        bootstrap_source(&self.database(), &self.checkout, &self.member)
    }

    fn rpc(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        store_rpc(&self.database(), &self.checkout, &self.member, method, args)
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

/// Where the coordination store of the container checked out at `checkout`
/// is, by the checkout's layout (its git directory, #2145). Members pin what
/// they find (`SwarmContext::database`); host reads follow this each time.
pub fn store_database(checkout: &Path) -> PathBuf {
    super::swarm_store_location::located(checkout)
}

fn bootstrap_source(database: &Path, checkout: &Path, member: &str) -> String {
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
        json!(database.to_string_lossy()),
        json!(checkout.to_string_lossy()),
        json!(member),
    ));
    source
}

/// One board call against the store at `checkout`, acting as `member`, over
/// the persistent interpreter for that pair (see `swarm_board_worker`).
/// Shared by in-swarm contexts and the supervising session's host-side
/// handle (#1924).
fn store_rpc(
    database: &Path,
    checkout: &Path,
    member: &str,
    method: &str,
    args: Value,
) -> Result<Value, DomainError> {
    super::swarm_board_worker::call(
        &super::swarm_board_worker::Board {
            checkout,
            database,
            member,
        },
        &bootstrap_source(database, checkout, member),
        method,
        args,
    )
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

    /// The run the store holds, or `None` when no store exists there. The
    /// store's own bounded busy timeout absorbs contention from live members;
    /// a read that still fails is reported, and the caller retains the
    /// environment rather than guessing.
    pub fn hosted_run(
        &self,
    ) -> Result<Option<crate::domain::environment_retention::HostedSwarmRun>, DomainError> {
        match self.contained()? {
            Found::Store => {}
            Found::Nothing => return Ok(None),
        }
        let status = store_rpc(
            &self.database(),
            &self.checkout,
            "supervisor",
            "_status",
            json!([]),
        )?;
        decode_hosted_run(&status).map(Some)
    }

    /// Record the coordinator's loss in one store operation: a run that has
    /// already ended (or that the store's expiry check ends first) is left
    /// alone; otherwise the coordinator is quarantined exactly as an in-swarm
    /// reconcile would quarantine a vanished harness (paused holding
    /// `failed`). Returns the run as it stands afterwards.
    pub fn record_lost_coordinator(
        &self,
        coordinator: &str,
    ) -> Result<crate::domain::environment_retention::CoordinatorLoss, DomainError> {
        match self.contained()? {
            Found::Store => {}
            Found::Nothing => {
                return Err(DomainError::Tool(format!(
                    "no swarm store at {}",
                    self.database().display()
                )));
            }
        }
        let value = store_rpc(
            &self.database(),
            &self.checkout,
            coordinator,
            "_lose_coordinator",
            json!([]),
        )?;
        Ok(crate::domain::environment_retention::CoordinatorLoss {
            run: decode_hosted_run(&value)?,
            lost: value["lost"]
                .as_bool()
                .ok_or_else(|| DomainError::Tool("swarm loss receipt carries no verdict".into()))?,
        })
    }
}

/// What the host finds where a container's store should be.
enum Found {
    /// A store file, really inside the checkout.
    Store,
    /// No store file there (never created, or gone since).
    Nothing,
}

impl HostedStore {
    /// The board the host reads: where the checkout's layout places it.
    fn database(&self) -> PathBuf {
        store_database(&self.checkout)
    }

    /// The host opens the store only where it really is inside the checkout:
    /// a link or a `.git` pointer planted in the container is refused. (A
    /// linked worktree's git directory is outside its checkout, so the host
    /// cannot read that board and keeps the environment.)
    fn contained(&self) -> Result<Found, DomainError> {
        let database = self.database();
        let store = match std::fs::canonicalize(&database) {
            Ok(store) if store.is_file() => store,
            Ok(_) => return Ok(Found::Nothing),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Found::Nothing);
            }
            Err(error) => {
                return Err(DomainError::Tool(format!(
                    "swarm store {}: {error}",
                    database.display()
                )));
            }
        };
        let checkout = std::fs::canonicalize(&self.checkout).map_err(|e| {
            DomainError::Tool(format!("swarm checkout {}: {e}", self.checkout.display()))
        })?;
        // (A link swapped in between this check and the open is not caught:
        // pinning the directory is #2147.)
        match store.starts_with(&checkout) {
            true => Ok(Found::Store),
            false => Err(DomainError::Tool(format!(
                "swarm store {} is outside its checkout {}",
                store.display(),
                checkout.display()
            ))),
        }
    }
}

fn decode_hosted_run(
    status: &Value,
) -> Result<crate::domain::environment_retention::HostedSwarmRun, DomainError> {
    let deadline = status["deadline"]
        .as_f64()
        .ok_or_else(|| DomainError::Tool("swarm status carries no deadline".into()))?;
    let run_status = status["status"]
        .as_str()
        .ok_or_else(|| DomainError::Tool("swarm status carries no status".into()))?;
    let coordinator = status["coordinator"]
        .as_str()
        .ok_or_else(|| DomainError::Tool("swarm status carries no coordinator".into()))?;
    let outcome = status["outcome"]
        .as_str()
        .map(coordination::decode_status)
        .transpose()?;
    let id = status["id"]
        .as_str()
        .filter(|id| !id.is_empty())
        .ok_or_else(|| DomainError::Tool("swarm status carries no run id".into()))?;
    Ok(crate::domain::environment_retention::HostedSwarmRun {
        id: id.to_owned(),
        status: coordination::decode_status(run_status)?,
        outcome,
        coordinator: coordinator.to_owned(),
        deadline,
    })
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
