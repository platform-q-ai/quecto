//! Transport-neutral turn declaration vocabulary. Inactive until runtime composition exists.

/// The default is the identity no admitted turn has (#2708).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
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
