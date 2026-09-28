//! Value records the board policy reads (#2266). Statuses stay raw strings:
//! the board stores and compares text, `describe` interpolates it, and a
//! value found in an existing file must survive unchanged. Policy decides on
//! them with affirmative `match`es over the known values.
use std::borrow::Cow;

macro_rules! status_text {
    ($(#[$meta:meta])* $name:ident { $($constant:ident = $text:literal),+ $(,)? }) => {
        $(#[$meta])*
        #[derive(Clone, Debug, PartialEq, Eq)]
        pub struct $name(pub Cow<'static, str>);

        impl $name {
            $(pub const $constant: Self = Self(Cow::Borrowed($text));)+

            /// A status as read from the store, known or not.
            pub fn new(text: impl Into<String>) -> Self {
                Self(Cow::Owned(text.into()))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

status_text! {
    /// `run.status`.
    RunState {
        SETUP = "setup",
        RUNNING = "running",
        PAUSED = "paused",
        SUCCEEDED = "succeeded",
        BLOCKED = "blocked",
        FAILED = "failed",
        CANCELLED = "cancelled",
        BUDGET_EXHAUSTED = "budget-exhausted",
    }
}

status_text! {
    /// `tasks.status`.
    TaskState {
        READY = "ready",
        CLAIMED = "claimed",
        BLOCKED = "blocked",
        SUBMITTED = "submitted",
        COMPLETED = "completed",
    }
}

status_text! {
    /// `members.status`.
    MemberState {
        LIVE = "live",
        RESERVED = "reserved",
        DEAD = "dead",
    }
}

/// The `run` row, as far as policy reads it.
#[derive(Clone, Debug, PartialEq)]
pub struct RunRecord {
    pub status: RunState,
    pub coordinator: String,
    /// Unix seconds (`run.deadline REAL`).
    pub deadline: f64,
    /// 1 through 25, including the coordinator.
    pub member_limit: i64,
    /// The outcome a paused run holds (#1729).
    pub outcome: Option<String>,
    pub outcome_reason: Option<String>,
}

/// A `members` row.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MemberRecord {
    pub id: String,
    pub status: MemberState,
    pub reservation: Option<String>,
}

/// One `{artifact, revision}` entry of a task's evidence.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRef {
    pub artifact: String,
    pub revision: String,
}

/// A `tasks` row, as far as policy reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TaskRecord {
    pub id: i64,
    pub status: TaskState,
    pub owner: Option<String>,
    pub evidence: Vec<EvidenceRef>,
}

/// How a criterion is satisfied: a command check, or a parent-reviewed requirement.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CriterionKind {
    Command,
    Review,
}

impl CriterionKind {
    /// The stored text, or `None` for any other value.
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "command" => Some(Self::Command),
            "review" => Some(Self::Review),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Command => "command",
            Self::Review => "review",
        }
    }
}

/// One entry of `run.criteria`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Criterion {
    pub id: String,
    pub kind: CriterionKind,
    pub description: String,
}

/// An `evidence` row recorded against a criterion.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EvidenceRow {
    pub criterion: String,
    pub revision: String,
    pub kind: CriterionKind,
    pub accepted: bool,
}
