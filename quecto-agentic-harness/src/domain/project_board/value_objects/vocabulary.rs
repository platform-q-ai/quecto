//! The closed vocabularies of a task file; any other spelling is refused.
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskKind {
    Epic,
    Task,
    Chore,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Draft,
    Ready,
    Claimed,
    InProgress,
    Review,
    Blocked,
    Done,
    Archived,
}

impl TaskStatus {
    /// Claimed and in-progress tasks always have a holder.
    pub fn requires_claim(self) -> bool {
        matches!(self, Self::Claimed | Self::InProgress)
    }

    /// A task under review or blocked may keep its holder's claim.
    pub fn may_hold_claim(self) -> bool {
        self.requires_claim() || matches!(self, Self::Review | Self::Blocked)
    }
}
