//! Why the board's rules refused a move, renewal, takeover or completion.
use super::schema_error::SchemaError;
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
    /// A takeover is only of a claimed or in-progress task.
    NotTakeable(TaskStatus),
    /// The holder renews; a takeover is someone else's.
    AlreadyHolder,
    /// The task links no pull request to complete it.
    NoPrs,
    /// These linked pull requests are not merged yet.
    PrsOpen(Vec<u64>),
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
            Self::NotTakeable(status) => {
                write!(f, "a {} task cannot be taken over", status.as_str())
            }
            Self::AlreadyHolder => f.write_str("the holder renews rather than takes over"),
            Self::NoPrs => f.write_str("the task links no pull request"),
            Self::PrsOpen(open) => write!(f, "pull requests {open:?} are not merged"),
            Self::Invalid(error) => write!(f, "invalid task: {error}"),
        }
    }
}

impl std::error::Error for BoardRuleError {}
