//! What a running external agent process answers (#2286): how it ended,
//! why it could not start, why a turn could not be written.

/// The most of a process's stderr kept for diagnostics, in bytes.
pub const EXTERNAL_AGENT_STDERR_TAIL_BYTES: usize = 64 * 1024;

/// How an external agent process ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalAgentExit {
    /// It exited with this status.
    Code(i32),
    /// A signal ended it.
    Signal(i32),
    /// Its end could not be observed.
    Unobservable(String),
}

impl ExternalAgentExit {
    /// Whether it exited by itself with status 0.
    pub fn is_clean(&self) -> bool {
        matches!(self, Self::Code(0))
    }
}

/// Why an external agent could not be started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalAgentLaunchError {
    /// The agent's program is not installed where it is looked for.
    NotFound {
        program: String,
        required_for: String,
    },
    /// The launch spec names something the agent cannot be given.
    InvalidSpec(String),
    /// The member's private directories could not be made.
    MemberDirectory(String),
    /// The process could not be spawned.
    Spawn(String),
}

impl ExternalAgentLaunchError {
    /// The error's kind, for telemetry: no detail.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::NotFound { .. } => "not_found",
            Self::InvalidSpec(_) => "invalid_spec",
            Self::MemberDirectory(_) => "member_directory",
            Self::Spawn(_) => "spawn",
        }
    }
}

impl std::fmt::Display for ExternalAgentLaunchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotFound {
                program,
                required_for,
            } => write!(
                f,
                "{program} not found on PATH (required for {required_for})"
            ),
            Self::InvalidSpec(detail) => write!(f, "invalid external agent launch: {detail}"),
            Self::MemberDirectory(detail) => {
                write!(f, "external agent member directory: {detail}")
            }
            Self::Spawn(detail) => write!(f, "external agent spawn failed: {detail}"),
        }
    }
}

impl std::error::Error for ExternalAgentLaunchError {}

/// The id a user turn was written under (#2287). The agent's `result`
/// names the user turns it consumed by these ids, so the session binds
/// each result to the prompts it answers. Opaque: the adapter mints it.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct UserTurnId(pub String);

/// Milliseconds on the monotonic scale an [`crate::application::external_agent::ports::ExternalAgentClock`]
/// chooses (#2287).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct AgentClockInstant(pub u64);

/// Why a user turn could not be written to the agent.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalAgentInputError {
    /// The input was closed: no further turn can be written.
    Closed,
    /// Writing failed (the process may have ended).
    Write(String),
}

impl std::fmt::Display for ExternalAgentInputError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Closed => write!(f, "the external agent's input is closed"),
            Self::Write(detail) => write!(f, "writing to the external agent failed: {detail}"),
        }
    }
}

impl std::error::Error for ExternalAgentInputError {}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;
