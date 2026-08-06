//! Explicit lifecycle state machine for spawned sub-agents.
//!
//! The registry's [`SubagentStatus`](super::subagent_registry::SubagentStatus)
//! remains the parent-facing wire/status projection. This module owns the
//! lifecycle vocabulary used by monitor, registry, await, passive-note, and kill
//! code so race-prone transitions are described in one place instead of as
//! scattered status assignments.

use super::subagent_registry::SubagentStatus;

/// Internal lifecycle state of a spawned sub-agent.
///
/// `Ready` distinguishes the child socket being connectable from the child turn
/// being idle/running. Parent-facing status stays intentionally smaller for UDS
/// compatibility and is derived through [`status_projection`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SubagentLifecycleState {
    /// Registry entry exists and the process has been spawned, but the monitor
    /// has not connected to the child socket yet.
    #[default]
    Launched,
    /// The monitor connected to the socket; no child turn has been observed yet.
    SocketReady,
    /// The child is actively processing a prompt or tool call.
    Busy,
    /// The child is between turns and can accept more work.
    Idle,
    /// A run-level error has been observed.
    Failed,
    /// The child process or monitor connection exited.
    Exited,
    /// The parent requested termination and removed the agent from the registry.
    Killed,
}

/// Lifecycle events emitted by the process monitor, await handling, passive note
/// path, and kill path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubagentLifecycleEvent {
    /// Monitor connected to the child socket.
    SocketConnected,
    /// Monitor failed to connect before the child exited or the retry budget was
    /// exhausted.
    SocketConnectFailed,
    /// A new agent turn started or was accepted by the child queue.
    RunStarted,
    /// A tool started inside the current turn.
    ToolStarted,
    /// A turn ended successfully or with only recoverable tool errors.
    RunEnded,
    /// A run-level failure was observed.
    RunFailed,
    /// A manual await reached a terminal/idle result and consumed the duplicate
    /// passive note for this run.
    AwaitConsumedCompletion,
    /// Passive notification for a completion was emitted.
    PassiveNoteEmitted,
    /// Await hit its wall-clock timeout; the child remains in its current state.
    AwaitTimedOut,
    /// Process exited or the monitor stream closed.
    ProcessExited,
    /// Parent requested kill.
    KillRequested,
}

impl SubagentLifecycleState {
    /// Apply a lifecycle event and return the next state.
    ///
    /// Benign duplicate/late observations are idempotent: terminal states remain
    /// terminal, duplicate `SocketConnected` preserves readiness, and await/note
    /// bookkeeping events do not change child process state.
    #[must_use]
    pub fn transition(self, event: SubagentLifecycleEvent) -> Self {
        use SubagentLifecycleEvent as Event;
        use SubagentLifecycleState as State;

        match (self, event) {
            (State::Killed, _) => State::Killed,
            (State::Exited, Event::KillRequested) => State::Killed,
            (State::Exited, _) => State::Exited,

            (_, Event::KillRequested) => State::Killed,
            (_, Event::ProcessExited | Event::SocketConnectFailed) => State::Exited,
            (_, Event::RunFailed) => State::Failed,
            (State::Failed, Event::ToolStarted) => State::Failed,
            (_, Event::RunStarted | Event::ToolStarted) => State::Busy,
            (_, Event::RunEnded) => State::Idle,

            (State::Launched, Event::SocketConnected) => State::SocketReady,
            (state, Event::SocketConnected) => state,
            (
                state,
                Event::AwaitConsumedCompletion | Event::PassiveNoteEmitted | Event::AwaitTimedOut,
            ) => state,
        }
    }

    /// Project the richer internal lifecycle into the existing UDS/registry
    /// status vocabulary.
    #[must_use]
    pub fn status_projection(self) -> SubagentStatus {
        match self {
            Self::Launched | Self::SocketReady => SubagentStatus::Starting,
            Self::Busy => SubagentStatus::Running,
            Self::Idle => SubagentStatus::Idle,
            Self::Failed => SubagentStatus::Error,
            Self::Exited | Self::Killed => SubagentStatus::Exited,
        }
    }

    /// Reconstruct a lifecycle state from the existing registry status when the
    /// richer field is unavailable in older in-memory entries/tests.
    #[must_use]
    pub fn from_status(status: &SubagentStatus) -> Self {
        match status {
            SubagentStatus::Starting => Self::Launched,
            SubagentStatus::Idle => Self::Idle,
            SubagentStatus::Running => Self::Busy,
            SubagentStatus::Error => Self::Failed,
            SubagentStatus::Exited => Self::Exited,
        }
    }
}

/// Apply `event` to `state` in place and return the projected parent-facing
/// status. This helper keeps registry mutations concise and makes every caller
/// name the lifecycle event it is applying.
pub fn apply_lifecycle_event(
    state: &mut SubagentLifecycleState,
    event: SubagentLifecycleEvent,
) -> SubagentStatus {
    *state = state.transition(event);
    state.status_projection()
}

#[cfg(test)]
#[path = "subagent_lifecycle_tests.rs"]
mod tests;
