//! Run and member lifecycle values: statuses, a member's process identity
//! and how its process ended.

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

/// How a launched member's process ended, as its launcher observed it
/// (#1961). Bash tool children run in their own process groups and can
/// outlive an abruptly ended harness, so only an orderly end (the member's
/// own teardown ran: a delegated kill, a protocol shutdown, an exit it
/// chose, or a fallback signal this harness sent to its whole group) frees
/// its file reservations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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

#[cfg(test)]
#[path = "lifecycle_tests.rs"]
mod tests;
