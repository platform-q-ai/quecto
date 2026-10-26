//! How a caller pins a change to what it read (#2705 B9), beside
//! `dto.rs`.
use crate::application::project_board::ports::BoardHead;
use crate::domain::project_board::value_objects::timestamp::Timestamp;

/// How a caller pins a change to what it read (#2705 B9): the board's
/// revision it read at, which `show` answers as `rev`, or, deprecated and
/// taken for one more release, the artifact's `updated`. A rev pin holds
/// while the artifact is as the board held it at that revision, whatever
/// else the board took since.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadPin {
    Rev(BoardHead),
    /// Deprecated: a stamp is for display, and two changes in one second
    /// share one.
    Updated(Timestamp),
}

impl ReadPin {
    /// The pin as the caller gave it.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Rev(rev) => &rev.0,
            Self::Updated(updated) => updated.as_str(),
        }
    }

    /// The deprecated `updated` pin's stamp, when it is one.
    pub fn deprecated_updated(&self) -> Option<&Timestamp> {
        match self {
            Self::Updated(updated) => Some(updated),
            Self::Rev(_) => None,
        }
    }
}
