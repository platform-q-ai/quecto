//! Why the board's rules refused a move, renewal, takeover or completion.
use super::schema_error::SchemaError;
use super::slug::Slug;
use super::timestamp::Timestamp;
use super::vocabulary::{PlanState, TaskStatus};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BoardRuleError {
    /// No row of the transition table moves a task between these statuses.
    NotAllowed { from: TaskStatus, to: TaskStatus },
    /// Only the claim's holder may do this.
    NotHolder,
    /// The task has no claim to act on.
    NoClaim,
    /// The claim has not expired yet.
    ClaimLive,
    /// The holder's claim has expired: renew it first.
    ClaimExpired,
    /// This run is still going: end it before the task leaves its holder,
    /// or before another plan is accepted.
    RunOpen(u32),
    /// The run has already ended: a run folds back once.
    RunEnded(u32),
    /// Only a draft plan is edited, accepted or discarded.
    PlanNotDraft(PlanState),
    /// The plan or run belongs to another task, or the run to another
    /// plan version.
    NotThisTasks,
    /// A done or archived task's plans are settled: none is proposed,
    /// edited, accepted or discarded.
    TaskFinished(TaskStatus),
    /// The task has no plan of that version.
    UnknownPlanVersion(u32),
    /// The plan changed since the caller read it (`updated` now): read it
    /// again and decide again.
    PlanChanged { version: u32, updated: Timestamp },
    /// The plan was stamped by a clock running ahead of the outside time:
    /// building on it would drag the task ahead too.
    PlanAhead { version: u32, updated: Timestamp },
    /// The task's plans contradict each other (a hand edit, a hostile
    /// push): mend them before deciding on one.
    PlansInconsistent(String),
    /// The task's runs contradict each other (more than one going): mend
    /// them before acting on the task.
    RunsInconsistent(String),
    /// A run runs the task's accepted plan; this version is not it.
    PlanNotAccepted(u32),
    /// `task` already links the branch or pull request: by its change
    /// `change`, or the old way when `None`. A pull request is linked to
    /// one task, for good, and a branch to one change until it merges.
    ChangeAlreadyLinked { task: Slug, change: Option<u32> },
    /// The task has no change of that number.
    NoSuchChange(u32),
    /// The change was stamped by a clock running ahead of the outside
    /// time: building on it would drag the board ahead too.
    ChangeAhead { change: u32, updated: Timestamp },
    /// The task links changes: it is done when they merge, through the
    /// completion anyone may run, not by its holder's say-so. The holder
    /// may unlink one deliberately first.
    ChangesLinked(Vec<u32>),
    /// The host's reading of this pull request was not given: the rule
    /// does not guess its state.
    PrUnread(u64),
    /// The host reads this pull request as closed or merged: only an open
    /// one is linked.
    PrNotOpen(u64),
    /// The host reads this pull request's head as another repository or
    /// branch than the change names.
    PrHeadMismatch(u64),
    /// This change was unlinked: it delivers nothing now.
    ChangeUnlinked(u32),
    /// This change merged: it stays linked.
    ChangeMerged(u32),
    /// The task links no change to complete it.
    NoChanges,
    /// These linked changes have not merged.
    ChangesUnmerged(Vec<u32>),
    /// The task's changes contradict each other (a number twice, a
    /// branch or pull request linked twice): mend them before acting.
    ChangesInconsistent(String),
    /// A takeover is only of a task that may hold a claim (claimed, in
    /// progress, in review or blocked).
    NotTakeable(TaskStatus),
    /// The holder renews; a takeover is someone else's.
    AlreadyHolder,
    /// The task links no pull request to complete it.
    NoPrs,
    /// These linked pull requests are not merged: open, or closed without
    /// merging.
    PrsUnmerged(Vec<u64>),
    /// The task links these pull requests: it is done when they merge,
    /// through the completion anyone may run (`CompleteMergedTask`, which
    /// reads their states from the host), not by its holder's say-so. The
    /// holder may unlink one deliberately (a wrong or abandoned pull
    /// request) first.
    PrsLinked(Vec<u64>),
    /// The task does not link this pull request.
    PrNotLinked(u64),
    /// The task already links this pull request.
    PrAlreadyLinked(u64),
    /// Only a draft is edited in place; a task past its draft changes
    /// through its moves.
    NotDraft(TaskStatus),
    /// The edit changes nothing.
    NoEdit,
    /// The result would break the task schema.
    Invalid(SchemaError),
}

impl fmt::Display for BoardRuleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAllowed { from, to } => {
                write!(
                    f,
                    "a task does not move from {} to {}",
                    from.as_str(),
                    to.as_str()
                )
            }
            Self::NotHolder => f.write_str("only the claim's holder may do this"),
            Self::NoClaim => f.write_str("the task has no claim"),
            Self::ClaimLive => f.write_str("the claim has not expired"),
            Self::ClaimExpired => f.write_str("the claim has expired; renew it first"),
            Self::RunOpen(run) => write!(f, "run {run} is still open"),
            Self::RunEnded(run) => write!(f, "run {run} has already ended"),
            Self::PlanNotDraft(state) => write!(
                f,
                "a {} plan is not a draft: only drafts are edited, accepted or discarded",
                state.as_str()
            ),
            Self::NotThisTasks => f.write_str("it belongs to another task or plan version"),
            Self::TaskFinished(status) => {
                write!(f, "a {} task's plans are settled", status.as_str())
            }
            Self::UnknownPlanVersion(version) => {
                write!(f, "the task has no plan version {version}")
            }
            Self::PlanChanged { version, updated } => write!(
                f,
                "plan version {version} changed since it was read (updated {}); read it again",
                updated.as_str()
            ),
            Self::PlansInconsistent(why) => write!(f, "the task's plans contradict: {why}"),
            Self::PlanAhead { version, updated } => write!(
                f,
                "plan version {version} was stamped {} by a clock that ran ahead",
                updated.as_str()
            ),
            Self::RunsInconsistent(why) => write!(f, "the task's runs contradict: {why}"),
            Self::PlanNotAccepted(version) => {
                write!(f, "plan version {version} is not the task's accepted plan")
            }
            Self::ChangeAlreadyLinked {
                task,
                change: Some(change),
            } => write!(
                f,
                "change {change} of task {} already links that branch or pull request",
                task.as_str()
            ),
            Self::ChangeAlreadyLinked { task, change: None } => {
                write!(f, "task {} already links that pull request", task.as_str())
            }
            Self::NoSuchChange(number) => write!(f, "the task has no change {number}"),
            Self::ChangeAhead { change, updated } => write!(
                f,
                "change {change} was stamped {} by a clock that ran ahead",
                updated.as_str()
            ),
            Self::ChangesLinked(numbers) => write!(
                f,
                "the task links changes {numbers:?}, so it is done when they merge: \
                 complete it once they have (anyone may), or unlink one first"
            ),
            Self::PrUnread(pr) => write!(f, "the host's reading of pull request {pr} is missing"),
            Self::PrNotOpen(pr) => write!(f, "pull request {pr} is not open"),
            Self::PrHeadMismatch(pr) => write!(
                f,
                "pull request {pr}'s head is another repository or branch than the change names"
            ),
            Self::ChangeUnlinked(number) => write!(f, "change {number} was unlinked"),
            Self::ChangeMerged(number) => write!(f, "change {number} merged: it stays linked"),
            Self::NoChanges => f.write_str("the task links no change"),
            Self::ChangesUnmerged(numbers) => write!(f, "changes {numbers:?} have not merged"),
            Self::ChangesInconsistent(why) => write!(f, "the task's changes contradict: {why}"),
            Self::NotTakeable(status) => {
                write!(f, "a {} task cannot be taken over", status.as_str())
            }
            Self::AlreadyHolder => f.write_str("the holder renews rather than takes over"),
            Self::NoPrs => f.write_str("the task links no pull request"),
            Self::PrsUnmerged(open) => write!(f, "pull requests {open:?} are not merged"),
            Self::PrsLinked(prs) => write!(
                f,
                "the task links pull requests {prs:?}, so it is done when they merge: \
                 complete it once they have (anyone may), or unlink one first"
            ),
            Self::PrNotLinked(pr) => write!(f, "the task does not link pull request {pr}"),
            Self::PrAlreadyLinked(pr) => write!(f, "the task already links pull request {pr}"),
            Self::NotDraft(status) => write!(
                f,
                "a {} task is not a draft: only drafts are edited",
                status.as_str()
            ),
            Self::NoEdit => f.write_str("the edit changes nothing"),
            Self::Invalid(error) => write!(f, "invalid task: {error}"),
        }
    }
}

impl std::error::Error for BoardRuleError {}
