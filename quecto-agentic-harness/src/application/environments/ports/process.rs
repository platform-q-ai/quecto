//! The environment's retained scripts and its members' shutdown: the process
//! commands, the process liveness port and the member-shutdown report.

use super::PortFuture;
use crate::application::environments::dto::EnvironmentLiveness;
use crate::application::environments::dto::StateOnDisk;
use crate::domain::environments::entities::environment_registry::EnvironmentRecord;
use std::path::Path;

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

    /// Run the retained `cleanup` once (best effort by contract). `Ok`
    /// means the script reported success; `Err` carries its account (or
    /// why it could not be run), so a caller that forgets the environment
    /// afterwards (#2206) does so only once it is really gone.
    fn run_retained_cleanup<'a>(
        &'a self,
        environment_id: &'a str,
        argv: &'a [String],
    ) -> PortFuture<'a, Result<(), String>>;

    /// Whether `record`'s container still runs, through its retained
    /// `inspect` (#2206): the check before a `stopped` record's leftovers
    /// are removed. `Unknown` when the script set has no inspect or the
    /// runtime could not be asked.
    fn observe_liveness<'a>(
        &'a self,
        record: &'a EnvironmentRecord,
    ) -> PortFuture<'a, EnvironmentLiveness>;
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

/// The runtime reality behind a record: its container's liveness through
/// the retained `inspect` argv, its retained `cleanup`, and whether its
/// state directory is still on disk (the host filesystem, #2134). Synchronous
/// (bounded scripts) so a startup restore and the CLI collector can ask
/// without a runtime.
pub trait EnvironmentProcess: Send + Sync {
    fn observe(&self, record: &EnvironmentRecord) -> EnvironmentLiveness;
    /// Run the retained `cleanup` once; `Err` carries the script's account.
    fn cleanup(&self, record: &EnvironmentRecord) -> Result<(), String>;
    /// Whether an environment's state directory is still on disk (#2134).
    fn state_on_disk(&self, environment_dir: &Path) -> StateOnDisk;
    /// Monotonic milliseconds, so a caller can bound how long it spends
    /// inspecting (#2134); only differences are meaningful.
    fn inspect_clock_millis(&self) -> u64;
}
