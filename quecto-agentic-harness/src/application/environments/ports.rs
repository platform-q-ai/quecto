//! Capability-local effect ports of the environments capability (#1939).
//!
//! Each port is owned by a use case here; infrastructure implements them and
//! composition wires concrete instances. Signatures name only domain and
//! application types: no socket, process, runtime or wire vocabulary crosses
//! this boundary. The environment registry itself is the pure domain
//! aggregate (`EnvironmentRegistry`): its exclusive kill and inspect claims
//! are the transitions these use cases drive.
use std::future::Future;
use std::pin::Pin;

use std::path::Path;

use crate::application::environments::dto::{
    ContainerRuntimeTarget, DiagnosableContainerConfig, EnvironmentLiveness, EnvironmentStateDir,
    PreflightCheck, RuntimeContainer,
};
use crate::domain::environment_registry::EnvironmentRecord;
use crate::domain::environment_retention::{CoordinatorLoss, HostedSwarmRun, SwarmRunObservation};

pub type PortFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>;

/// The retained script argv of an environment, run against its runtime id.
/// Whether and when each runs is the use case's decision; the adapter owns
/// the invocation mechanics (argv exec, environment variable, bounds).
pub trait EnvironmentProcessCommands: Send + Sync {
    /// Run the retained `inspect` once: the parsed metadata object on
    /// success, an actionable error otherwise.
    fn run_retained_inspect<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<serde_json::Value, String>>;

    /// Run the retained `kill` once. `Ok` means the script reported success
    /// (the environment is gone); `Err` carries the script's own account.
    fn run_retained_kill<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>>;

    /// Run the retained `cleanup` once (best effort by contract).
    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, ()>;
}

/// What the supervising session can learn about — and record on — the swarm
/// run an environment hosts (#1924).
pub trait HostedSwarmRunObservation: Send + Sync {
    /// Observe the swarm run `record` hosts, when it advertises a
    /// coordination store this session can reach.
    fn observe_hosted_swarm_run<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, SwarmRunObservation>;

    /// Record the coordinator's harness as lost on the hosted run in ONE
    /// store operation: a run that has already ended (including one the
    /// store's own expiry check ends first) is left alone; any other run is
    /// paused holding `failed` (the #1729 lost-harness rule). Returns the run
    /// as it stands afterwards.
    fn record_lost_coordinator<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
        hosted: &'a HostedSwarmRun,
    ) -> PortFuture<'a, Result<CoordinatorLoss, String>>;
}

/// How one member's shutdown was settled by the subagent capability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberShutdownResult {
    /// Acknowledged the protocol and its exit was observed; no signal.
    Graceful,
    /// The locally owned handle's fallback produced the exit.
    Fallback,
    /// Already exited (or already compensated) when the shutdown reached it.
    AlreadyExited,
    /// Asked, but this session holds no process for it and no exit was
    /// observed within the bound: its row was compensated unobserved. The
    /// retained environment kill is what ends it.
    Unobserved,
    /// Another termination of the same member was already in flight and
    /// completed; this shutdown joined it.
    Joined,
}

impl MemberShutdownResult {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Graceful => "graceful",
            Self::Fallback => "fallback",
            Self::AlreadyExited => "already-exited",
            Self::Unobserved => "unobserved",
            Self::Joined => "joined",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledMember {
    /// The member's agent UUID as recorded on the environment.
    pub member: String,
    pub result: MemberShutdownResult,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnsettledMember {
    pub member: String,
    pub detail: String,
}

/// What asking every member of an environment to shut down established.
/// A member is either settled (its row's terminal effects ran, or were
/// joined) or reported unsettled with any claim on it lifted: nothing is
/// silently dropped, and the caller decides whether the environment's own
/// kill may proceed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct MemberShutdownReport {
    pub settled: Vec<SettledMember>,
    pub unsettled: Vec<UnsettledMember>,
}

impl MemberShutdownReport {
    pub fn all_settled(&self) -> bool {
        self.unsettled.is_empty()
    }
}

/// Shuts the members of an environment down through the subagent teardown
/// capability: protocol shutdown over each member's own edge (a container
/// coordinator's harness settles its in-container descendants itself), the
/// owned-handle fallback only for handles this session owns, and each row's
/// exactly-once compensation. Never a signal to a process this session does
/// not own, never the environment's own kill.
pub trait EnvironmentMemberShutdown: Send + Sync {
    fn shutdown_members<'a>(
        &'a self,
        members: &'a [String],
    ) -> PortFuture<'a, MemberShutdownReport>;
}

// ─── Container-runtime diagnosis (#2024 S4b) ─────────────────────────────────

/// The container config a diagnosis targets, resolved the way a launch
/// resolves it: the effective configuration of the working directory
/// (its trusted overlay merged over the global file) or an explicit file,
/// the named entry or the labelled default. Implemented by infrastructure
/// over the launch policy's selection; composition binds the checkout.
pub trait ContainerConfigLookup: Send + Sync {
    fn lookup(&self, target: &ContainerRuntimeTarget)
    -> Result<DiagnosableContainerConfig, String>;
}

/// The create script's own preflight, run without creating an
/// environment: which binaries exist, whether the image is present,
/// whether the repository is reachable, whether the state dir is
/// writable. One list of checks serves the create and the doctor, so the
/// adapter asks the script rather than reimplementing it. `Err` carries
/// why no checks could be obtained (the script refuses the mode, is
/// missing, or reported nothing).
pub trait ContainerRuntimePreflight: Send + Sync {
    fn preflight(&self, config: &DiagnosableContainerConfig)
    -> Result<Vec<PreflightCheck>, String>;
}

// ─── Durable environments (#2024 S4d) ────────────────────────────────────────

/// The durable registry of one base directory: every session sharing the
/// directory allocates its refs and writes its records here, so a `C*` ref
/// is unique across sessions and outlives the harness that minted it.
/// Members are never stored: they belong to the session that launched
/// them. Every operation is synchronous — a record write is small and must
/// have landed before the launch that committed it returns.
pub trait EnvironmentRegistryStore: Send + Sync {
    /// The next never-reused ref number, allocated under an exclusive hold
    /// so two sessions never receive the same one.
    fn allocate_ref(&self) -> Result<u64, String>;
    /// Every record on file, in ref order.
    fn load(&self) -> Result<Vec<EnvironmentRecord>, String>;
    /// Write `record` under its ref, replacing what was there.
    fn record(&self, record: &EnvironmentRecord) -> Result<(), String>;
    /// Remove the record under `environment_ref` (a rolled-back create).
    fn forget(&self, environment_ref: &str) -> Result<(), String>;
}

/// The runtime reality behind a record: its container's liveness through
/// the retained `inspect` argv, and its retained `cleanup`. Synchronous
/// (bounded scripts) so a startup restore and the CLI collector can ask
/// without a runtime.
pub trait EnvironmentProcess: Send + Sync {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness;
    /// Run the retained `cleanup` once; `Err` carries the script's account.
    fn cleanup(&self, record: &EnvironmentRecord) -> Result<(), String>;
}

/// The host's inventory of environments outside any registry, through a
/// container config's own scripts (the harness knows no runtime): every
/// environment the runtime knows (`inspect --list`), the state
/// directories under a state root, and the removal of one environment by
/// id (the config's `cleanup`, which takes the container and the state
/// dir down together).
pub trait ContainerRuntimeInventory: Send + Sync {
    fn containers(
        &self,
        config: &DiagnosableContainerConfig,
    ) -> Result<Vec<RuntimeContainer>, String>;
    fn environment_dirs(&self, root: &Path) -> Result<Vec<EnvironmentStateDir>, String>;
    fn remove(
        &self,
        config: &DiagnosableContainerConfig,
        environment_id: &str,
    ) -> Result<(), String>;
}
