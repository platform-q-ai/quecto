//! The coordination board's callers inside a container and on the host:
//! [`SwarmContext`] and [`HostedStore`] reach the board through
//! [`SwarmBoard`], the Rust dispatcher over composition's handles (#2278).
//! Nothing here runs Python (#2282); the packaged Python board sources are
//! loaded only by tests, until #2283 deletes them.
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::application::swarm::dto::BoardLocation;
use crate::domain::error::DomainError;

pub use self::board::{SwarmBoard, SwarmBoardHandlesBuilder, SwarmBoardOpLogBuilder};

#[derive(Clone, Debug)]
pub struct SwarmContext {
    pub checkout: PathBuf,
    pub member: String,
    pub lifecycle: std::sync::Arc<dyn crate::application::swarm::ports::SwarmLifecycle>,
    /// The board this context calls: composition's handles (#2278).
    pub board: SwarmBoard,
}

impl SwarmContext {
    /// Explicit context supplied by the container launch adapter. Host-local
    /// reference scripts deliberately do not set this contract.
    pub fn discover(
        lifecycle: std::sync::Arc<dyn crate::application::swarm::ports::SwarmLifecycle>,
        board: SwarmBoard,
    ) -> Option<Self> {
        let checkout = Self::contracted_checkout()?;
        let member = std::env::var("QUECTO_SWARM_MEMBER")
            .unwrap_or_else(|_| format!("member-{}", std::process::id()));
        Some(Self {
            checkout,
            member,
            lifecycle,
            board,
        })
    }

    /// The checkout of the container contract this process was launched
    /// under, when it was: [`Self::discover`] finds a context exactly then.
    pub fn contracted_checkout() -> Option<PathBuf> {
        let checkout = std::env::var_os("QUECTO_SWARM_CHECKOUT")?;
        let protocol = std::env::var("QUECTO_SWARM_CONTAINER").ok()?;
        let host_namespace = std::env::var("QUECTO_SWARM_HOST_PID_NS").ok()?;
        match protocol == "isolated-pid-v1" && isolated_pid_namespace(&host_namespace) {
            true => Some(checkout.into()),
            false => None,
        }
    }

    pub fn database(&self) -> PathBuf {
        super::swarm_store_location::member_store_path(&self.checkout)
    }

    /// The packaged Python board, bound to this member (test support only:
    /// production runs no Python, #2282; #2283 deletes the sources).
    #[cfg(any(test, feature = "test-support"))]
    pub fn bootstrap(&self) -> String {
        bootstrap_source(&self.database(), &self.checkout, &self.member)
    }

    fn rpc(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        self.board.call(self.location(), &self.member, method, args)
    }

    /// The board file this context calls, in its checkout.
    fn location(&self) -> BoardLocation {
        BoardLocation {
            database: self.database(),
            checkout: self.checkout.clone(),
        }
    }

    /// One board call by name: the structured `swarm` ops' (#2279), and
    /// tests'. Blocking: call it off the async workers.
    pub(crate) fn call(&self, method: &str, args: Value) -> Result<Value, DomainError> {
        self.rpc(method, args)
    }

    /// How structured ops read a member's text and write their answers on
    /// this context's board (#2279): no board call, no file resolved, so
    /// it is read on the async worker (#2279 review L5).
    pub(crate) fn wire(&self) -> super::swarm_board_dispatch::BoardWire {
        self.board.wire()
    }

    /// Records `method` as refused with `kind` before it reached the board
    /// (#2279), `elapsed` after the op began.
    pub(crate) fn refused(
        &self,
        method: &str,
        kind: crate::domain::swarm::RefusalKind,
        arguments: super::swarm_board_dispatch::BindingFaults,
        elapsed: std::time::Duration,
    ) {
        self.board.refused(
            self.location(),
            &self.member,
            method,
            (kind, arguments),
            elapsed,
        );
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

    /// Writes the run's `swarm_run_summary` (#2313) when this member's
    /// harness is the one to: the coordinator's, once the run in
    /// `snapshot` settled; at most once per run, and only while the event
    /// log is on. Whether it wrote it.
    pub(crate) fn summarize_settled(&self, snapshot: &crate::domain::swarm::Snapshot) -> bool {
        match snapshot.summarized_by(&self.member) {
            true => self.board.summarize_run(&self.location(), &self.member),
            false => false,
        }
    }

    /// The full summary the harness reads for itself on this member's
    /// behalf (the post-call lifecycle's, a settlement's), recorded with
    /// role `host` (#2279 S15 final review). Blocking.
    pub(crate) fn host_summary(&self) -> Result<Value, DomainError> {
        self.board.call_as(
            self.location(),
            &self.member,
            "summary",
            json!([null]),
            super::swarm_board_dispatch::CallOrigin::Harness,
        )
    }
}

/// Where the coordination store of the container checked out at `checkout`
/// is, by the checkout's layout (its git directory, #2145). Members pin what
/// they find (`SwarmContext::database`); host reads follow this each time.
pub fn store_database(checkout: &Path) -> PathBuf {
    super::swarm_store_location::located(checkout)
}

#[cfg(any(test, feature = "test-support"))]
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

/// Host-side handle on a container's coordination store for the supervising
/// session (#1924): the store lives in the identity-mounted checkout, so the
/// session that launched the container reads it by path once the members'
/// sockets are gone. Membership-free reads only, plus the #1729 lost-harness
/// record made as the lost coordinator itself.
#[derive(Clone, Debug)]
pub struct HostedStore {
    checkout: PathBuf,
    board: SwarmBoard,
}

impl HostedStore {
    /// The store of the container checked out at `checkout`, reached
    /// through `board` (composition's handles, #2278).
    pub fn at(checkout: PathBuf, board: SwarmBoard) -> Self {
        Self { checkout, board }
    }

    /// The run the store holds, or `None` when no store exists there. The
    /// store's own bounded busy timeout absorbs contention from live members;
    /// a read that still fails is reported, and the caller retains the
    /// environment rather than guessing.
    pub fn hosted_run(
        &self,
    ) -> Result<Option<crate::domain::environment_retention::HostedSwarmRun>, DomainError> {
        let displaced = match self.contained()? {
            Found::Store { displaced } => displaced,
            Found::Nothing => return Ok(None),
        };
        let status = self
            .board
            .call(self.location(), "supervisor", "_status", json!([]))?;
        let run = decode_hosted_run(&status)?;
        // The current board holds no created run, but another place holds
        // a board a member may still be running on (#2206 round 3): the
        // box is not proven plain.
        match (run.created(), displaced) {
            (false, Some(displaced)) => Err(DomainError::Tool(format!(
                "swarm board {} holds no created run, but another board exists at {} where a member may still be running",
                self.database().display(),
                displaced.display()
            ))),
            _ => Ok(Some(run)),
        }
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
            Found::Store { .. } => {}
            Found::Nothing => {
                return Err(DomainError::Tool(format!(
                    "no swarm store at {}",
                    self.database().display()
                )));
            }
        }
        let value =
            self.board
                .call(self.location(), coordinator, "_lose_coordinator", json!([]))?;
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
    /// A store file, really inside the checkout — and a board at a place
    /// the layout no longer names, if one was also found (#2206 round 3).
    Store { displaced: Option<PathBuf> },
    /// No store file there (never created, or gone since).
    Nothing,
}

impl HostedStore {
    /// The board the host reads: where the checkout's layout places it.
    fn database(&self) -> PathBuf {
        store_database(&self.checkout)
    }

    /// The board file the host calls, in the container's checkout.
    fn location(&self) -> BoardLocation {
        BoardLocation {
            database: self.database(),
            checkout: self.checkout.clone(),
        }
    }

    /// The host opens the store only where it really is inside the checkout:
    /// a link or a `.git` pointer planted in the container is refused. (A
    /// linked worktree's git directory is outside its checkout, so the host
    /// cannot read that board and keeps the environment.)
    fn contained(&self) -> Result<Found, DomainError> {
        use super::swarm_store_location::BoardPresence;
        // "No board" only on every place's own "no such file" (#2206
        // round 2): a place that cannot be examined, a board only where
        // the layout no longer places it, or anything but a regular file
        // at the board's place is never taken for "no swarm".
        let (database, displaced) = match super::swarm_store_location::board_presence(
            &self.checkout,
        ) {
            BoardPresence::Absent => return Ok(Found::Nothing),
            BoardPresence::Current { board, displaced } => (board, displaced),
            BoardPresence::Displaced(displaced) => {
                return Err(DomainError::Tool(format!(
                    "swarm board {} is not where the checkout's layout now places it ({}); a member may still be running on it",
                    displaced.display(),
                    self.database().display()
                )));
            }
            BoardPresence::Unknown(reason) => {
                return Err(DomainError::Tool(format!(
                    "swarm board could not be looked for: {reason}"
                )));
            }
        };
        let store = std::fs::canonicalize(&database).map_err(|error| {
            DomainError::Tool(format!("swarm store {}: {error}", database.display()))
        })?;
        match std::fs::metadata(&store) {
            Ok(meta) if meta.is_file() => {}
            Ok(_) => {
                return Err(DomainError::Tool(format!(
                    "swarm store {} is not a regular file",
                    database.display()
                )));
            }
            Err(error) => {
                return Err(DomainError::Tool(format!(
                    "swarm store {}: {error}",
                    database.display()
                )));
            }
        }
        let checkout = std::fs::canonicalize(&self.checkout).map_err(|e| {
            DomainError::Tool(format!("swarm checkout {}: {e}", self.checkout.display()))
        })?;
        // (A link swapped in between this check and the open is not caught:
        // pinning the directory is #2147.)
        match store.starts_with(&checkout) {
            true => Ok(Found::Store { displaced }),
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

/// The board this process's `SwarmContext`s call (#2278): composition's,
/// bound once by the agent's admission, beside the process socket. A
/// context is discovered from this process's container contract, so its
/// board is this process's too.
static PROCESS_BOARD: std::sync::OnceLock<SwarmBoard> = std::sync::OnceLock::new();

/// Binds `board` as this process's board; the first binding stays.
/// Returns the board bound.
pub fn bind_process_board(board: SwarmBoard) -> &'static SwarmBoard {
    PROCESS_BOARD.get_or_init(|| board)
}

/// The board bound for this process, if admission bound one.
pub fn process_board() -> Option<&'static SwarmBoard> {
    PROCESS_BOARD.get()
}

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
        let engine = crate::domain::workflow::lock_engine(engine);
        engine.guards_enabled() || engine.active_template().is_some()
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

#[path = "swarm_bridge_board.rs"]
mod board;
#[path = "swarm_coordination.rs"]
mod coordination;
#[cfg(test)]
#[path = "swarm_participation_tests.rs"]
mod participation_tests;
