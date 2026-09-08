//! Swarm lifecycle vocabulary and effect ports, independent of adapter protocols.
use super::error::DomainError;
use super::subagent_launch::LaunchFuture;

/// A swarm owns its coordination lifecycle; workflow engines cannot run alongside it.
pub fn validate_workflow(swarm_agent: bool, requested: bool) -> Result<(), DomainError> {
    if swarm_agent && requested {
        return Err(DomainError::Tool("workflow is unavailable for swarm agents; omit workflow, workflow_guards and workflow_spec".into()));
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunStatus {
    Setup,
    Running,
    Succeeded,
    Blocked,
    Failed,
    Cancelled,
    BudgetExhausted,
}

impl RunStatus {
    pub fn terminal(self) -> bool {
        !matches!(self, Self::Setup | Self::Running)
    }

    pub fn abort_coordinator(self) -> bool {
        matches!(self, Self::Failed | Self::BudgetExhausted)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberStatus {
    Live,
    Reserved,
    Dead,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcessIdentity {
    pub pid: u32,
    pub started: String,
}

#[derive(Clone, Debug)]
pub struct Member {
    pub id: String,
    pub status: MemberStatus,
    pub process: Option<ProcessIdentity>,
    pub endpoint: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub status: RunStatus,
    pub coordinator: String,
    pub deadline: f64,
    pub members: Vec<Member>,
}

/// Implementations atomically enforce membership policy before reserving slots.
/// Wire names, positional arguments and persistence schemas are private details.
pub trait CoordinationPort {
    fn snapshot(&self) -> Result<Snapshot, DomainError>;
    fn register_endpoint(&self, endpoint: &str) -> Result<(), DomainError>;
    fn reserve_member(&self, member: &str, token: &str) -> Result<(), DomainError>;
    fn record_launch(
        &self,
        member: &str,
        token: &str,
        process: &ProcessIdentity,
    ) -> Result<(), DomainError>;
    fn confirm_unlaunched(&self, member: &str) -> Result<(), DomainError>;
    fn quarantine(&self, member: &str) -> Result<(), DomainError>;
}

pub trait ProcessObservation {
    fn harness_dead(&self, process: &ProcessIdentity) -> bool;
}

pub trait ProcessControl: Sync {
    /// Cancel this member's detached execution registry independently of turn abort.
    fn cancel_local_executions(&self);
    fn abort<'a>(&'a self, member: &'a Member) -> LaunchFuture<'a, bool>;
    /// Adapters must validate the process identity before signalling it.
    fn terminate<'a>(
        &'a self,
        process: &'a ProcessIdentity,
    ) -> LaunchFuture<'a, Result<(), DomainError>>;
}

pub trait Clock {
    fn now_seconds(&self) -> f64;
}

/// Application lifecycle entrypoint, injected by the composition root.
pub trait SwarmLifecycle: std::fmt::Debug + Send + Sync {
    fn reconcile(
        &self,
        coordination: &dyn CoordinationPort,
        processes: &dyn ProcessObservation,
    ) -> Result<Snapshot, DomainError>;
    fn settle<'a>(
        &'a self,
        snapshot: &'a Snapshot,
        actor: &'a str,
        processes: &'a dyn ProcessControl,
    ) -> LaunchFuture<'a, Result<(), DomainError>>;
    fn observed_outcome(&self, snapshot: &Snapshot, clock: &dyn Clock) -> RunStatus;
}
