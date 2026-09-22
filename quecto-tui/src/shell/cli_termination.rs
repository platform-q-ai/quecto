//! The termination-signal side of the CLI (#2053): why a spawn stopped, what
//! an exit on a signal did, and the words for both.

/// Why a spawn yielded no usable agent (#2053): it failed, or a termination
/// signal arrived while the TUI was still waiting for its announcement.
#[derive(Debug)]
pub(crate) enum SpawnAbort {
    Failed(String),
    Interrupted(crate::shell::signals::TerminationSignal),
}

/// A termination signal that arrived while the TUI was still starting
/// (#2053): spawning or connecting, nothing to persist yet.
pub(super) fn interrupted_during_startup(
    rx: &mut tokio::sync::mpsc::Receiver<crate::shell::signals::TerminationSignal>,
) -> Option<crate::shell::signals::TerminationSignal> {
    rx.try_recv().ok()
}

/// What became of the owned agent at an exit on a termination signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum OwnedAgentAtExit {
    AskedToEnd,
    LeftRunning,
    NoneOwned,
}

/// What the exit on a termination signal did (#2053). The persist was asked
/// for; whether it landed is reported separately by the exit's own errors.
pub(super) fn termination_exit_message(
    signal: crate::shell::signals::TerminationSignal,
    owned: OwnedAgentAtExit,
) -> String {
    let owned = match owned {
        OwnedAgentAtExit::AskedToEnd => "the owned agent was asked to end",
        OwnedAgentAtExit::LeftRunning => "the owned agent was left running (--detach-on-exit)",
        OwnedAgentAtExit::NoneOwned => "no agent is owned (attached)",
    };
    format!(
        "{}: the session was asked to persist, {owned}",
        signal.name()
    )
}

pub(super) fn startup_interrupted_message(
    signal: crate::shell::signals::TerminationSignal,
    kill_owned: bool,
) -> String {
    let agent = if kill_owned {
        "is terminated"
    } else {
        "is left running (--detach-on-exit)"
    };
    format!(
        "{} during startup: the agent being started {agent}, nothing to persist",
        signal.name()
    )
}
