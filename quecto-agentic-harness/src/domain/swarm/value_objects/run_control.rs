//! Run-control actions and the receipt a control call answers with.

use crate::domain::swarm::value_objects::lifecycle::RunStatus;

#[derive(Clone, Debug)]
pub enum RunControlAction {
    Wake {
        generation: u64,
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
    /// Why a resume would refuse right now (#1924): a passed deadline or a
    /// lost coordinator. Empty for a live run.
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
