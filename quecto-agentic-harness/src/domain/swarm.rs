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
    Paused,
    Succeeded,
    Blocked,
    Failed,
    Cancelled,
    BudgetExhausted,
}

impl RunStatus {
    pub fn terminal(self) -> bool {
        matches!(
            self,
            Self::Succeeded
                | Self::Blocked
                | Self::Failed
                | Self::Cancelled
                | Self::BudgetExhausted
        )
    }

    /// Terminal tools remain read-only; the coordinator can still explain the result.
    pub fn admits_inference(self, coordinator: bool) -> bool {
        match self {
            Self::Setup | Self::Running => true,
            Self::Paused => false,
            Self::Succeeded
            | Self::Blocked
            | Self::Failed
            | Self::Cancelled
            | Self::BudgetExhausted => coordinator,
        }
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
    pub control_generation: u64,
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
    /// Cancel current jobs while retaining admission for a later resume.
    fn suspend_local_executions(&self, snapshot: &Snapshot);
    /// Suspend only this process; never signal a future turn or another member.
    fn suspend_local_inference(&self, snapshot: &Snapshot);
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

#[derive(Clone, Debug)]
pub enum RunControlAction {
    Wake {
        generation: u64,
    },
    UsageBudget {
        token_limit: Option<u64>,
        strict_unknown: bool,
    },
    Pause {
        reason: String,
    },
    Resume,
    Status,
}

#[derive(Clone, Debug)]
pub struct RunControlReceipt {
    pub budget: Option<UsageBudgetStatus>,
    pub wake_allowed: bool,
    pub status: RunStatus,
    pub generation: u64,
}

/// Supervisor operations remain available without model execution or turn-queue admission.
pub trait SwarmRunControl: Send + Sync {
    fn apply(
        &self,
        action: RunControlAction,
    ) -> LaunchFuture<'_, Result<RunControlReceipt, DomainError>>;
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct UsageBudgetStatus {
    pub token_limit: Option<u64>,
    pub strict_unknown: bool,
    pub warned: bool,
    pub observed_tokens: u64,
    pub unknown_usage_requests: u64,
}
