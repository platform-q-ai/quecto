//! Transport-neutral turn declaration vocabulary. Inactive until runtime composition exists.

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnIdentity {
    pub session_key: String,
    pub outcome_epoch: String,
    pub turn_id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LaunchAuthority {
    TopLevelParent,
    Child,
    ChildCoordinator,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnIntent {
    Waiting,
    Blocked,
    Asking,
    Done,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TurnDeclaration {
    pub identity: TurnIdentity,
    pub intent: TurnIntent,
    pub reason: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TurnBoundary {
    OuterRun,
    ProviderRetry,
    ToolIteration,
}

use crate::domain::shared::error::DomainError;

pub const VALIDATION: &str = "Choose waiting, blocked, asking, or done and provide a nonempty one-line reason of at most 512 characters.";
pub const AUTHORITY: &str =
    "Turn outcomes may be declared only by the active top-level parent turn.";
pub const STALE: &str = "This declaration does not belong to the active turn.";

impl LaunchAuthority {
    pub fn admit(self) -> Result<(), DomainError> {
        match self {
            Self::TopLevelParent => Ok(()),
            Self::Child | Self::ChildCoordinator => Err(DomainError::Tool(AUTHORITY.into())),
        }
    }
}

impl TurnDeclaration {
    pub fn validated(
        identity: TurnIdentity,
        intent: TurnIntent,
        reason: &str,
    ) -> Result<Self, DomainError> {
        let trimmed = reason.trim();
        if (1..=512).contains(&trimmed.chars().count()) && reason.split(['\r', '\n']).count() == 1 {
            Ok(Self {
                identity,
                intent,
                reason: trimmed.into(),
            })
        } else {
            Err(DomainError::Tool(VALIDATION.into()))
        }
    }
}
