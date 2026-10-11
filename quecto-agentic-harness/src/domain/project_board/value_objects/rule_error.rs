//! Why the board's rules refused a move, renewal, takeover or completion.
use super::schema_error::SchemaError;
use super::slug::Slug;
use super::vocabulary::TaskStatus;
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
    /// This run is still going: end it before the task leaves its holder.
    RunOpen(Slug),
    /// The task has no open run to end.
    NoOpenRun,
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
    /// not by its holder's say-so. The holder may unlink one deliberately
    /// (a wrong or abandoned pull request) first.
    PrsLinked(Vec<u64>),
    /// The task does not link this pull request.
    PrNotLinked(u64),
    /// The task already links this pull request.
    PrAlreadyLinked(u64),
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
            Self::RunOpen(run) => write!(f, "run {} is still open", run.as_str()),
            Self::NoOpenRun => f.write_str("the task has no open run"),
            Self::NotTakeable(status) => {
                write!(f, "a {} task cannot be taken over", status.as_str())
            }
            Self::AlreadyHolder => f.write_str("the holder renews rather than takes over"),
            Self::NoPrs => f.write_str("the task links no pull request"),
            Self::PrsUnmerged(open) => write!(f, "pull requests {open:?} are not merged"),
            Self::PrsLinked(prs) => write!(
                f,
                "the task links pull requests {prs:?}: it is done when they merge"
            ),
            Self::PrNotLinked(pr) => write!(f, "the task does not link pull request {pr}"),
            Self::PrAlreadyLinked(pr) => write!(f, "the task already links pull request {pr}"),
            Self::Invalid(error) => write!(f, "invalid task: {error}"),
        }
    }
}

impl std::error::Error for BoardRuleError {}
