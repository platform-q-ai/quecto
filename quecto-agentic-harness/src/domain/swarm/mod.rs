//! Swarm lifecycle vocabulary, independent of adapter protocols. The effect
//! ports (`CoordinationPort`, `SwarmRunControl`, `ProcessControl`, …) are the
//! application's (`application::swarm::ports`, #1940, #1960).
//!
//! The coordination board's pure decisions (#2265, #2266) live in the
//! submodules: value records, the policy ported from `swarm_policy.py`, and
//! input validation, wake notification, owner liveness and the usage
//! budget. No I/O and no storage format.
use super::error::DomainError;

pub mod dependencies;
pub mod notification;
pub mod owner;
pub mod policy;
pub mod python_value;
pub mod records;
pub mod telemetry;
pub mod usage;
pub mod validation;

pub use dependencies::{DEPENDENCIES_MAX, dependency_list, validate_dependencies};
pub use notification::{
    NotificationEvent, NotificationState, OWNERSHIP_ACTIONS, READY_WORK_ACTIONS, TaskSummary,
    WORK_HOLDING_STATUSES, WORKING_STATUSES, notification_targets, ready_work_takers,
};
pub use owner::{
    ADDRESSABLE_OWNER_STATES, OWNER_IDLE_AFTER, OWNER_STATES, OwnerState, idle_transition,
    owner_recovery, owner_state,
};
pub use policy::{
    Access, PROPOSED_OUTCOMES, STOP_STATUSES, admission, authorize, completion, describe, expired,
    require_budget, require_unsubmitted, resume_blockers, revalidation, status_is_alive,
    validate_extension,
};
pub use python_value::{python_equal, python_truthy};
pub use records::{
    Criterion, CriterionKind, EvidenceRow, MemberRecord, MemberState, RunRecord, RunState,
    TaskRecord, TaskState,
};
pub use telemetry::{BoardOpObservation, BoardOpOutcome, BoardRole, RefusalKind};
pub use usage::{
    UsageBudget, UsageDecision, UsageTotals, request_measurement, usage_budget_decision,
};
pub use validation::{bounded, bounded_text, criteria};

/// A refusal by the board. `Display` is the exact message members see.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BoardError(pub String);

impl BoardError {
    pub fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }

    /// The refusal's kind (#2303). RED STUB.
    pub fn kind(&self) -> RefusalKind {
        RefusalKind::Internal
    }
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl std::error::Error for BoardError {}

/// A swarm owns its coordination lifecycle; workflow engines cannot run alongside it.
pub fn validate_workflow(swarm_agent: bool, requested: bool) -> Result<(), DomainError> {
    if swarm_agent && requested {
        return Err(DomainError::Tool("workflow is unavailable for swarm agents; omit workflow, workflow_guards and workflow_spec".into()));
    }
    Ok(())
}

/// Whether an agent in a container takes part in a swarm (#1715). Every
/// container carries a placeholder run from its bootstrap with deadline 0;
/// that is an ordinary container whatever status it ends in. `create`
/// requires a future deadline, and a created run keeps it through every later
/// status, so a positive deadline means a swarm exists and every member is a
/// swarm agent.
pub fn participates(deadline: f64) -> bool {
    deadline > 0.0
}

/// Creating a swarm turns the creator into its coordinator, which cannot be
/// running a workflow (guards, a bound spec or a selected template). An idle,
/// merely available workflow tool does not block creation; it refuses to act
/// afterwards.
pub fn validate_swarm_creation(workflow_engaged: bool) -> Result<(), DomainError> {
    if workflow_engaged {
        return Err(DomainError::Tool(
            "a workflow-enabled agent cannot create a swarm; finish or relaunch without workflow, workflow_guards and workflow_spec, then create the run".into(),
        ));
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

    /// Outcomes a coordinator may propose when it ends a run (#1729); each
    /// holds the run as a resumable pause until the supervisor outside the
    /// swarm resumes or closes it.
    pub fn proposable(self) -> bool {
        matches!(
            self,
            Self::Succeeded | Self::Blocked | Self::Failed | Self::BudgetExhausted
        )
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
    /// The member whose harness launched this one (#2121); `None` for the
    /// bootstrapped coordinator and legacy stores.
    pub launcher: Option<String>,
}

#[derive(Clone, Debug)]
pub struct Snapshot {
    pub control_generation: u64,
    pub status: RunStatus,
    /// The outcome a paused run holds after the coordinator ended it, a
    /// budget ran out, the deadline passed or its harness was lost (#1729);
    /// a closed run keeps the outcome it reached. `None` for a plain
    /// supervisor pause, a cancelled run and every live status.
    pub outcome: Option<RunStatus>,
    pub coordinator: String,
    pub deadline: f64,
    pub members: Vec<Member>,
}

impl Snapshot {
    /// A run its coordinator ended (or that ran out of budget), waiting for
    /// the supervisor outside the swarm to resume or close it.
    pub fn ended(&self) -> bool {
        self.status == RunStatus::Paused && self.outcome.is_some_and(RunStatus::proposable)
    }

    /// Whether `actor` may run model inference: an ended run keeps its
    /// coordinator available for reporting, exactly like a terminal one.
    pub fn admits_inference(&self, actor: &str) -> bool {
        let coordinator = actor == self.coordinator;
        if self.ended() {
            coordinator
        } else {
            self.status.admits_inference(coordinator)
        }
    }
}

/// How a launched member's process ended, as its launcher observed it
/// (#1961). Bash tool children run in their own process groups and can
/// outlive an abruptly ended harness, so only an orderly end (the member's
/// own teardown ran: a delegated kill, a protocol shutdown, an exit it
/// chose, or a fallback signal this harness sent to its whole group) frees
/// its file reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MemberExit {
    Orderly,
    /// Killed by a signal this harness did not send, or an unobservable end.
    Abrupt,
}

impl MemberExit {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Orderly => "orderly",
            Self::Abrupt => "abrupt",
        }
    }
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
    /// Supervisor-only: lift a pause, including one holding an outcome.
    Resume,
    /// Supervisor-only: make the outcome a paused run holds terminal (#1729).
    Close,
    /// Supervisor-only: grant wall-clock budget before resuming (#1729).
    ExtendDeadline {
        seconds: u64,
    },
    Status,
}

#[derive(Clone, Debug)]
pub struct RunControlReceipt {
    pub budget: Option<UsageBudgetStatus>,
    pub wake_allowed: bool,
    pub status: RunStatus,
    /// Outcome held by a paused run and the reason it was proposed (#1729).
    pub outcome: Option<RunStatus>,
    pub reason: Option<String>,
    pub generation: u64,
    /// Members a resume could not wake (#1721); empty for other actions.
    pub wake_warnings: Vec<String>,
    /// Why a resume would refuse right now (#1924): a passed deadline, an
    /// exhausted budget or a lost coordinator. Empty for a live run.
    pub resume_blockers: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct UsageBudgetStatus {
    pub token_limit: Option<u64>,
    pub strict_unknown: bool,
    pub warned: bool,
    pub observed_tokens: u64,
    pub unknown_usage_requests: u64,
}

#[cfg(test)]
#[path = "swarm_tests.rs"]
mod tests;
