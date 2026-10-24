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
