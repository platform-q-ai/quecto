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

use std::path::{Path, PathBuf};

use crate::application::environments::dto::{
    AssetOutcome, AssetState, ContainerAsset, ContainerAssetCatalogue, ContainerConfigDocument,
    ContainerConfigEntry, ContainerRuntimeTarget, DiagnosableContainerConfig,
    PersistedContainerConfig, PreflightCheck,
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

// ─── Container-config discovery (#2024 S4c) ─────────────────────────────────

/// The raw effective container-config set of the launching agent, read the
/// way a launch reads it: the global file with the checkout's trusted
/// overlay merged in, each entry marked with the layer that declared it,
/// plus whether an overlay was withheld and the layer diagnostics. The
/// adapter reports the set as configured; the listing use case owns the
/// rules over it (default visibility, ordering). `Err` carries why no set
/// could be read (no configuration composed, an invalid file).
pub trait ContainerConfigRoster: Send + Sync {
    fn roster(&self) -> Result<ContainerConfigRosterReport, String>;

    /// An opaque token that changes whenever the roster could have
    /// changed (a configuration layer or the trust record was written)
    /// and is otherwise stable — cheap enough to ask for on every render,
    /// so a presenter can cache the rendered roster against it instead of
    /// reading the configuration each time.
    fn revision(&self) -> String;
}

/// What the roster port reports before the listing rules are applied.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ContainerConfigRosterReport {
    pub configs: Vec<ContainerConfigEntry>,
    pub overlay_withheld: bool,
    pub diagnostics: Vec<String>,
}

// ─── Standard container (#2024 S4e) ─────────────────────────────────────────

/// The embedded standard bundle (Containerfile, runtime scripts) and its
/// materialisation below a project. The catalogue is what this binary
/// carries; `observe` compares a destination with the embedded bytes;
/// `materialise` writes a missing asset (with its mode) and never
/// replaces an existing file, so an operator's edit survives a re-init.
/// The use case owns where the bundle goes and what a difference means.
pub trait ContainerAssetStore: Send + Sync {
    fn catalogue(&self) -> ContainerAssetCatalogue;

    /// What the destination of `asset` below `dir` holds; `Err` when it
    /// cannot be judged or written through (a symbolic link in the
    /// file's place or in any directory between `root` and it), so a
    /// dry run and a status see exactly what a materialise would refuse.
    fn observe(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetState, String>;

    /// Write `asset` below `dir`, which lies below `root` (the project):
    /// no directory between `root` (exclusive) and the asset may be a
    /// symbolic link, so a swapped directory cannot redirect the write
    /// outside the project.
    fn materialise(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String>;

    /// As `materialise`, but a regular file holding other bytes is
    /// replaced whole (temporary file, rename) with the embedded ones and
    /// its mode: `init --refresh`, the way back from an edit or an older
    /// bundle. The same symbolic-link refusals apply.
    fn refresh(
        &self,
        root: &Path,
        dir: &Path,
        asset: &ContainerAsset,
    ) -> Result<AssetOutcome, String>;
}

/// The repository a checkout came from: its `origin` remote URL, `None`
/// when the directory is not a git checkout or has no `origin`. `Err`
/// carries why it could not be asked (git missing).
pub trait WorkspaceOrigin: Send + Sync {
    fn origin(&self, checkout: &Path) -> Result<Option<String>, String>;

    /// The root of the working tree `checkout` lies in (`git rev-parse
    /// --show-toplevel`), `None` when it is not inside a checkout.
    /// `Err` carries why it could not be asked (git missing).
    fn toplevel(&self, checkout: &Path) -> Result<Option<PathBuf>, String>;
}

/// Records one `container_configs.<name>` entry in the project's
/// repo-local overlay through the configuration capability's one safe
/// write path (composition maps this port onto it): validated as a layer
/// and as the merge, trust recorded for exactly the bytes written, an
/// untrusted overlay refused in that capability's own words.
pub trait ContainerConfigPersistence: Send + Sync {
    /// Whether a persist would be accepted as things stand (an overlay
    /// location exists, the overlay's current content is trusted or
    /// absent): the refusal a persist would give, before anything else
    /// is written. Nothing is written.
    fn check(&self) -> Result<(), String>;

    fn persist(
        &self,
        name: &str,
        entry: &ContainerConfigDocument,
    ) -> Result<PersistedContainerConfig, String>;

    /// The overlay file a persist would write, when the run has one.
    fn location(&self) -> Option<PathBuf>;

    /// The entry `container_configs.<name>` as the project's own overlay
    /// currently declares it (`None` when the overlay has none), so a
    /// re-init can keep what it wrote before. Read from the applied
    /// overlay only — never a global entry of the same name; `Err` when
    /// the overlay cannot be read.
    fn existing(&self, name: &str) -> Result<Option<ContainerConfigDocument>, String>;
}
