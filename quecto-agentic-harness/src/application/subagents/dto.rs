//! Boundary request/result models for the subagent teardown capability.
use std::fmt;

use crate::domain::ids::AgentUuid;
use crate::domain::subagent_teardown::{
    DelegatedAgentIdentity, RoutingDepth, ShutdownReason, TerminationRouteError,
};

/// Independent inbound events that all converge on one common shutdown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ShutdownTrigger {
    /// A `shutdown` protocol command on an authorized connection.
    ProtocolCommand,
    /// The authenticated launch-bound parent connection closed.
    ParentConnectionClosed,
    /// No parent bound its control connection within the bind deadline.
    ParentNeverBound,
    /// SIGTERM/SIGINT delivered by the operating system.
    TerminationSignal,
    /// The last client of a top-level harness whose lifetime ends with it
    /// disconnected (#1938).
    LastClientDisconnected,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrepareShutdownRequest {
    pub reason: ShutdownReason,
    pub trigger: ShutdownTrigger,
}

/// Opaque proof that one caller holds the admitted shutdown. Every
/// participant gets its own token (one admission, one holder each), so a
/// holder's release can never be mistaken for another's. Only the
/// transaction that minted it can read it.
///
/// A token is a plain value: it has no drop behaviour. A holder that loses
/// it (or `mem::forget`s it) without releasing or executing leaks the freeze
/// until another trigger executes; keeping the token reachable is the
/// caller's responsibility (the UDS controller guards its own).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct ShutdownToken {
    admission: u64,
    holder: u64,
}

impl ShutdownToken {
    pub(super) const fn mint(admission: u64, holder: u64) -> Self {
        Self { admission, holder }
    }

    pub(super) const fn admission(&self) -> u64 {
        self.admission
    }

    pub(super) const fn holder(&self) -> u64 {
        self.holder
    }

    /// Presenter tests need a token to shape an ACK; it never reaches a wire.
    #[cfg(test)]
    pub(crate) const fn for_presenter_tests() -> Self {
        Self {
            admission: 0,
            holder: 0,
        }
    }
}

impl fmt::Debug for ShutdownToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ShutdownToken(..)")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedShutdown {
    pub token: ShutdownToken,
    /// `true` when an earlier trigger already admitted the shutdown and this
    /// caller joined it instead of admitting a second one.
    pub joined: bool,
    /// Reason recorded by the admitting trigger; later joiners inherit it.
    pub reason: ShutdownReason,
}

/// What releasing one holder's admission did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The last holder left: the freeze was lifted and nothing ran.
    Released,
    /// Other holders still hold the admission; it stays frozen.
    StillHeld,
    /// This holder had already released; nothing changed.
    AlreadyReleased,
    /// Execution already began or finished; a release changes nothing.
    ExecutionUnderway,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PersistenceOutcome {
    Persisted,
    Failed(String),
}

/// Result of the executed common shutdown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ShutdownOutcome {
    pub reason: ShutdownReason,
    /// Every trigger that admitted or joined this shutdown, first one first.
    pub triggers: Vec<ShutdownTrigger>,
    pub turn_cancelled: bool,
    pub children_shut_down: Vec<AgentUuid>,
    pub children_failed: Vec<(AgentUuid, String)>,
    pub persistence: PersistenceOutcome,
    pub exit_signalled: bool,
}

/// Terminal error vocabulary of the two-phase shutdown transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HarnessShutdownError {
    /// Execute or release without any admitted shutdown.
    NotPrepared,
    /// The token does not belong to the current admission or names no
    /// holder of it.
    UnknownToken,
    /// The token's holder already released it; a released holder can neither
    /// execute nor release again.
    TokenReleased,
    /// The harness already terminated: nothing further can be admitted.
    AlreadyTerminated,
    /// The lifecycle repository refused a transition it must never refuse
    /// for an admitted shutdown.
    LifecycleViolation(String),
    /// The spawned teardown run was dropped before it completed (its runtime
    /// went away); the admission and its progress are intact and `Execute`
    /// resumes at the first incomplete step.
    ExecutionInterrupted,
}

impl fmt::Display for HarnessShutdownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotPrepared => f.write_str("no shutdown has been prepared"),
            Self::UnknownToken => f.write_str("shutdown token is not the admitted one"),
            Self::TokenReleased => f.write_str("shutdown token was already released"),
            Self::AlreadyTerminated => f.write_str("harness already terminated"),
            Self::LifecycleViolation(detail) => write!(f, "lifecycle violation: {detail}"),
            Self::ExecutionInterrupted => f.write_str("shutdown execution was interrupted"),
        }
    }
}

impl std::error::Error for HarnessShutdownError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminateDelegatedAgentRequest {
    pub target: DelegatedAgentIdentity,
    pub remaining_depth: RoutingDepth,
}

/// The single edge this harness acted on; it never shuts itself down here.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminationRouted {
    /// The target was a direct child and self shutdown was invoked on it.
    /// `result` is how its end was observed by this harness — the owner —
    /// once the conclusion ports are composed; `None` when this harness
    /// only routed the edge and observed nothing (a bare route).
    ShutdownRequested {
        child: DelegatedAgentIdentity,
        result: Option<TerminationResult>,
    },
    /// The command was forwarded one hop; this harness stays alive.
    /// `result` is the outcome the downstream owner relayed, when any.
    Forwarded {
        via: DelegatedAgentIdentity,
        remaining_depth: RoutingDepth,
        result: Option<TerminationResult>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminateDelegatedAgentError {
    /// Routing policy refused: no edge was touched.
    Rejected(TerminationRouteError),
    /// The routing port failed to reach the resolved direct child; nothing
    /// was dispatched to it.
    ChildUnreachable { child: AgentUuid, detail: String },
    /// The receiving harness is itself frozen or terminated.
    NotAccepting,
    /// The target is not routable because this harness already observed
    /// its end (its terminal effects were claimed or ran).
    TargetAlreadyExited(AgentUuid),
    /// This harness dispatched effects toward its direct child (the
    /// protocol was acknowledged, or the owned-handle fallback signalled
    /// it) but did not observe the end within the bound. The stopping
    /// claim is kept so the eventual exit is compensated as this kill.
    TerminationFailed { child: AgentUuid, detail: String },
    /// The direct child the route went through answered with a refusal of
    /// its own, relayed distinctly.
    Downstream {
        via: AgentUuid,
        rejection: super::ports::DownstreamRejection,
    },
}

impl fmt::Display for TerminateDelegatedAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Rejected(error) => write!(f, "termination rejected: {error}"),
            Self::ChildUnreachable { child, detail } => {
                write!(f, "direct child {child} unreachable: {detail}")
            }
            Self::NotAccepting => f.write_str("harness is not accepting control commands"),
            Self::TargetAlreadyExited(uuid) => write!(f, "target {uuid} already exited"),
            Self::TerminationFailed { child, detail } => {
                write!(f, "termination of {child} failed: {detail}")
            }
            Self::Downstream { via, rejection } => write!(f, "via {via}: {rejection}"),
        }
    }
}

impl std::error::Error for TerminateDelegatedAgentError {}

// ─── Operator-selected termination (#1936, #1882) ────────────────────────────

/// An operator asks this harness to terminate one delegated agent, named
/// by uuid or live display label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillDelegatedAgentRequest {
    pub reference: String,
}

pub use super::ports::TerminationResult;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KillDelegatedAgentOutcome {
    pub target: DelegatedAgentIdentity,
    pub result: TerminationResult,
    /// The target and every descendant removed with it, target first.
    pub removed: Vec<AgentUuid>,
}

/// Every refusal leaves the registry as it was: it happens before any
/// effect, or lifts the stopping claim when nothing reached the child. Only
/// a failure after effects were dispatched keeps the row claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KillDelegatedAgentError {
    /// The reference names no live delegated agent (unknown, ambiguous,
    /// exited, or not launched through this harness).
    Unresolved(super::ports::ResolutionError),
    /// Another termination of the same agent is already in flight.
    AlreadyStopping,
    /// Routing policy refused: stale generation, cycle, depth, or the
    /// lineage disappeared between resolution and routing.
    Rejected(TerminationRouteError),
    /// The receiving harness is itself frozen or terminated.
    NotAccepting,
    /// The route toward a nested target could not be delivered: the direct
    /// child it goes through did not accept the command. No fallback exists
    /// for a target this harness does not own.
    RouteUnreachable { via: AgentUuid, detail: String },
    /// A harness on the route refused the command for a reason of its own
    /// (target unknown or stale there, its lifecycle, an edge it could not
    /// reach); nothing was dispatched to the target.
    DownstreamRejected { via: AgentUuid, detail: String },
    /// The termination did not observe the agent's end. With
    /// `effects_dispatched` the protocol was acknowledged or the fallback
    /// signalled the child, so the row stays claimed stopping and its
    /// eventual exit is compensated as this kill; without it nothing
    /// reached the child and the claim is lifted.
    Failed {
        detail: String,
        effects_dispatched: bool,
    },
}

impl fmt::Display for KillDelegatedAgentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unresolved(error) => write!(f, "{error}"),
            Self::AlreadyStopping => f.write_str("a termination is already in flight"),
            Self::Rejected(error) => write!(f, "termination rejected: {error}"),
            Self::NotAccepting => f.write_str("harness is not accepting control commands"),
            Self::RouteUnreachable { via, detail } => {
                write!(f, "route via {via} unreachable: {detail}")
            }
            Self::DownstreamRejected { via, detail } => {
                write!(f, "refused via {via}: {detail}")
            }
            Self::Failed { detail, .. } => write!(f, "termination failed: {detail}"),
        }
    }
}

impl std::error::Error for KillDelegatedAgentError {}

// ─── Owned child exit observation ────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObserveOwnedChildExitRequest {
    pub child: DelegatedAgentIdentity,
    pub observation: super::ports::ExitObservation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ObservedExit {
    /// This observation claimed and ran the row's terminal effects.
    Compensated { removed: Vec<AgentUuid> },
    /// Another path had already claimed them; this observation joined it.
    Joined(super::ports::CompensationObservation),
    /// A connection-level observation of a child whose process this harness
    /// still retains: the reaper observes the authoritative exit and runs
    /// the compensation, so nothing was removed while the process lived.
    DeferredToProcessExit,
}

// ─── Failed launch compensation ──────────────────────────────────────────────

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompensateFailedLaunchRequest {
    pub child: DelegatedAgentIdentity,
    /// Whether this launch created the environment it joined, so its
    /// rollback discards the record rather than listing it as stopped.
    pub owns_environment: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedLaunchCompensated {
    pub conclusion: super::ports::TerminationConclusion,
    pub removed: Vec<AgentUuid>,
}

// ─── Fleet teardown (#1938) ──────────────────────────────────────────────────

/// Tear down every direct child of this harness at once, telling each the
/// given reason.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminateAllDelegatedAgentsRequest {
    pub reason: ShutdownReason,
}

/// How one direct child settled under the fleet teardown.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FleetChildResult {
    /// Acknowledged the protocol and its exit was observed; no signal.
    Graceful,
    /// The owned handle's fallback produced the exit.
    Fallback,
    /// Already exited when the teardown reached it.
    AlreadyExited,
    /// Compensated without an observed exit: this harness held no process
    /// for the child and no exit was observed within the bound. Its
    /// environment finalization ran as part of the compensation.
    Unobserved,
    /// Another termination of the same child was already in flight and
    /// completed; this teardown joined it.
    Joined,
}

impl FleetChildResult {
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

impl fmt::Display for FleetChildResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettledChild {
    pub child: DelegatedAgentIdentity,
    pub result: FleetChildResult,
}

/// Result of one fleet teardown run. Every direct child is either settled
/// (its row's terminal effects ran, or were joined) or reported unsettled
/// with its claim lifted, so no child is ever silently dropped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetTeardownOutcome {
    pub reason: ShutdownReason,
    /// `true` when this caller joined a run another trigger had started.
    pub joined: bool,
    /// Settled children, in identity order.
    pub settled: Vec<SettledChild>,
    /// Children whose end could not be settled within the bound. Their
    /// rows stay live with the stopping claim lifted for a later trigger.
    pub unsettled: Vec<(AgentUuid, String)>,
    /// Terminal rows discarded from the roster once the fleet settled.
    pub pruned: Vec<AgentUuid>,
}

impl FleetTeardownOutcome {
    /// Every direct child settled: the roster holds no live delegated agent.
    pub fn is_settled(&self) -> bool {
        self.unsettled.is_empty()
    }

    /// Distinct rows this run moved out of the roster: every settled child
    /// (whose tombstone is then pruned) plus every other pruned terminal
    /// row.
    pub fn removed_count(&self) -> usize {
        self.settled.len()
            + self
                .pruned
                .iter()
                .filter(|uuid| {
                    !self
                        .settled
                        .iter()
                        .any(|settled| &settled.child.uuid == *uuid)
                })
                .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetTeardownError {
    /// The detached run was dropped by its runtime before it completed;
    /// the claims it still held were lifted, so a later trigger may retry.
    Interrupted,
}

impl fmt::Display for FleetTeardownError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Interrupted => f.write_str("fleet teardown was interrupted"),
        }
    }
}

impl std::error::Error for FleetTeardownError {}

// ─── Container config selection at launch (#2024 S4a) ────────────────────────

/// Where the container configs a launch selects from are read: the
/// launching agent's own effective configuration (its base file with its
/// checkout's trusted overlay merged in) or one explicit file the spawn
/// call named, which replaces the layers as `--config` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerConfigSource {
    LaunchingAgent,
    Explicit(std::path::PathBuf),
}

/// One named container config as launch policy sees it: the argv sets a
/// script-managed runtime runs, whether `container: true` selects it, and
/// what an agent choosing between entries needs to know (#2024 S4c):
/// whether the checkout's applied overlay declared it, and the repository
/// its create argv bakes in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerLaunchConfig {
    pub name: String,
    pub default: bool,
    pub create: Vec<String>,
    pub cleanup: Vec<String>,
    pub exec: Vec<String>,
    pub kill: Vec<String>,
    pub inspect: Vec<String>,
    /// Declared by the launching agent's checkout through its applied
    /// `.quecto/config.json` overlay (repo-bound), not by the global file.
    pub repo_bound: bool,
    /// The repository the create argv bakes in (`--repo <url>` for the
    /// shipped scripts); `None` for a sandbox config or an adapter whose
    /// argv names none in that form.
    pub repository: Option<String>,
}

impl ContainerLaunchConfig {
    /// Why launch policy would refuse this entry's argv, if it would:
    /// `create` and `cleanup` are required, and no argument of any set may
    /// be empty or carry a NUL. One rule for the selection and the roster.
    pub fn argv_problem(&self) -> Option<&'static str> {
        if self.create.is_empty() {
            return Some("missing create argv");
        }
        if self.cleanup.is_empty() {
            return Some("missing cleanup argv");
        }
        let unsafe_arg = |arg: &String| arg.is_empty() || arg.contains('\0');
        self.create
            .iter()
            .chain(&self.cleanup)
            .chain(&self.exec)
            .chain(&self.kill)
            .chain(&self.inspect)
            .any(unsafe_arg)
            .then_some("unsafe argv")
    }
}

/// The container configs in effect for a source, sorted by name, and the
/// layer diagnostics the configuration capability reported while
/// resolving them (an untrusted or refused overlay that was not applied,
/// a retired local file that is no longer loaded).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EffectiveContainerConfigSet {
    pub configs: Vec<ContainerLaunchConfig>,
    pub diagnostics: Vec<String>,
    /// The checkout has an overlay that was NOT applied and could have
    /// changed the default (refused, unparseable, or declaring
    /// `container_configs`): `configs` is the global set alone, and
    /// whatever default the overlay labels is unknown to launch policy.
    /// An unapplied overlay that cannot have touched the container set
    /// leaves this false and travels in `diagnostics` alone.
    pub overlay_withheld: bool,
}

impl EffectiveContainerConfigSet {
    /// The configured names, sorted.
    pub fn names(&self) -> Vec<String> {
        let mut names: Vec<String> = self.configs.iter().map(|c| c.name.clone()).collect();
        names.sort_unstable();
        names
    }
}

/// Why the effective container configs of a source could not be read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContainerConfigsError {
    /// No launching-agent configuration was composed for this launcher and
    /// the spawn call named no file.
    NoSource,
    /// The configuration could not be loaded or is invalid; the reason is
    /// the configuration capability's own diagnostic, naming the file(s).
    Invalid(String),
}

impl fmt::Display for ContainerConfigsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSource => {
                f.write_str("container spawn requires --config so container_configs can be loaded")
            }
            Self::Invalid(reason) => f.write_str(reason),
        }
    }
}

impl std::error::Error for ContainerConfigsError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectContainerConfigRequest {
    pub source: ContainerConfigSource,
    /// `container_config: "<name>"`; `None` selects the labelled default.
    pub name: Option<String>,
}

/// The config a new container launches with, and what the launcher should
/// tell the operator about the layers it was read from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectedContainerConfig {
    pub config: ContainerLaunchConfig,
    pub diagnostics: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SelectContainerConfigError {
    /// The spawn call named a relative file: only an absolute, trusted
    /// path may supply argv that this process will execute.
    RelativeConfigPath(std::path::PathBuf),
    Unavailable(ContainerConfigsError),
    /// `container: true` from a checkout whose overlay was not applied and
    /// could have changed the default (#2024 S4a: refused, unparseable,
    /// or declaring `container_configs`): the default it labels is
    /// unknown, so an implicit selection must not quietly land in the
    /// global one. The layer diagnostics say why the overlay was withheld.
    OverlayWithheld {
        diagnostics: Vec<String>,
    },
    /// `container: true` with no entry labelled `"default": true`. The
    /// layer diagnostics ride along: a withheld overlay is the likely
    /// reason the expected entry is missing.
    NoDefault {
        available: Vec<String>,
        diagnostics: Vec<String>,
    },
    Unknown {
        name: String,
        available: Vec<String>,
        diagnostics: Vec<String>,
    },
    /// The selected entry cannot be run: `what` names the argv fault.
    InvalidArgv {
        name: String,
        what: &'static str,
    },
}

fn available(names: &[String]) -> String {
    if names.is_empty() {
        "none configured".to_string()
    } else {
        names.join(", ")
    }
}

/// The layer diagnostics appended to an error line, one
/// `; Configuration diagnostics: <line>` each, so the reason an entry is
/// missing (a withheld overlay) is never dropped from the text.
fn appended(diagnostics: &[String]) -> String {
    diagnostics
        .iter()
        .map(|line| format!("; Configuration diagnostics: {line}"))
        .collect()
}

impl SelectContainerConfigError {
    /// The configuration-layer diagnostics the failed selection carried,
    /// for a caller that reports them on its own channel too.
    pub fn diagnostics(&self) -> &[String] {
        match self {
            Self::OverlayWithheld { diagnostics }
            | Self::NoDefault { diagnostics, .. }
            | Self::Unknown { diagnostics, .. } => diagnostics,
            Self::RelativeConfigPath(_) | Self::Unavailable(_) | Self::InvalidArgv { .. } => &[],
        }
    }
}

impl fmt::Display for SelectContainerConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RelativeConfigPath(_) => {
                f.write_str("container spawn requires an absolute trusted config path")
            }
            Self::Unavailable(error) => write!(f, "{error}"),
            Self::OverlayWithheld { diagnostics } => write!(
                f,
                "container: true refused: the checkout's repo-local config overlay was not applied, so the container config it labels default is unknown ({}); trust it, or name a container_config explicitly to launch from the global configuration",
                diagnostics.join("; ")
            ),
            Self::NoDefault {
                available: names,
                diagnostics,
            } => write!(
                f,
                "no container config is labeled \"default\": true (available container configs: {}){}",
                available(names),
                appended(diagnostics)
            ),
            Self::Unknown {
                name,
                available: names,
                diagnostics,
            } => write!(
                f,
                "unknown container config '{name}' (available container configs: {}){}",
                available(names),
                appended(diagnostics)
            ),
            Self::InvalidArgv { what, .. } => {
                write!(f, "invalid container_configs configuration: {what}")
            }
        }
    }
}

impl std::error::Error for SelectContainerConfigError {}
