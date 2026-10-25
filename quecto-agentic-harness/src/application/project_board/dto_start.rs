//! What a start takes (S4c): the task and the accepted plan version its
//! caller confirmed; and what ends a run by hand, for a start whose swarm
//! will never fold it back.
use crate::domain::project_board::value_objects::slug::Slug;
use crate::domain::project_board::value_objects::timestamp::Timestamp;
use crate::domain::project_board::value_objects::vocabulary::RunOutcome;

/// Task `task` started on its accepted plan, which the caller confirmed as
/// version `plan_version`: a task that names another now is refused, so
/// no plan the caller did not confirm is ever launched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskStart {
    pub task: Slug,
    pub plan_version: u32,
}

/// Run `run` of task `task` ended by hand as `outcome`, pinned to the task
/// as the caller read it when `expected_updated` is given. `stop` says how
/// a `stopped` run stopped when no board of its swarm was kept to say it
/// (#2705 T5 review round 1); `None` is a cancel. A kept board's own end
/// wins, and any other outcome ignores it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunEnd {
    pub task: Slug,
    pub run: u32,
    pub outcome: RunEndOutcome,
    pub expected_updated: Option<Timestamp>,
    pub stop: Option<crate::domain::project_board::services::run_outcomes::StopKind>,
}

/// How a run is ended by hand: an allowlist. The claim's holder ends it
/// as succeeded, stopped or failed, as a fold-back of a swarm that ended
/// so would; anyone who may end it as lost (the holder, or anyone when
/// nobody claims the task) does so when nobody can account for it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunEndOutcome {
    Succeeded,
    Stopped,
    Failed,
    Lost,
}

impl RunEndOutcome {
    pub const ALL: [Self; 4] = [Self::Succeeded, Self::Stopped, Self::Failed, Self::Lost];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Stopped => "stopped",
            Self::Failed => "failed",
            Self::Lost => "lost",
        }
    }

    /// The board's outcome of a run ended so.
    pub fn outcome(self) -> RunOutcome {
        match self {
            Self::Succeeded => RunOutcome::Succeeded,
            Self::Stopped => RunOutcome::Stopped,
            Self::Failed => RunOutcome::Failed,
            Self::Lost => RunOutcome::Lost,
        }
    }

    /// The outcome `text` names, when it is one of [`Self::ALL`].
    pub fn parse(text: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|outcome| outcome.as_str() == text)
    }
}

/// A run ended by hand, as written, the status its end left its task in
/// (#2705 T5), whether its front end could see that its swarm had
/// stopped, and where its task items came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndedRun {
    pub run: crate::domain::project_board::entities::run::Run,
    pub status: crate::domain::project_board::value_objects::vocabulary::TaskStatus,
    pub liveness: LivenessCheck,
    pub report: RunReport,
    /// For a success recorded from its swarm's kept board, how many of its
    /// recorded items' evidence entries name no commit the push rule
    /// checked: none at all, or one shorter than 12 hex digits or longer
    /// than a full id; each entry counted, repeats included (#2666). `None`
    /// for any other end.
    pub unchecked: Option<u64>,
}

/// Where an end_run took the run's task items from (#2481 S4c follow-up):
/// the board its swarm left, as this front end kept it; or no report at
/// all, every item as todo (a run ended as lost records none either).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RunReport {
    Kept,
    Unreported,
}

/// Whether an end_run's front end looked for the run's swarm still running
/// (S4c): one that can see none (the CLI) did not, and its caller makes
/// sure the coordinator has stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LivenessCheck {
    Checked,
    NotChecked,
}

impl LivenessCheck {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Checked => "checked",
            Self::NotChecked => "not_checked",
        }
    }
}
